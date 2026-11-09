// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
use super::*;
use serde_json::json;

/// What another page or `terminum hosts` sends while an enable runs, and before which step.
#[derive(Clone, Copy, PartialEq)]
enum Meanwhile {
    Nothing,
    /// Not now
    Decline,
    /// Not now, then Enable again
    DeclineThenEnable,
    /// Files removed while agents keep the helper
    Remove,
    /// Add host's edit gives the host a new recipe
    Moved,
}

/// The host's answers given, every step recorded; no ssh.
struct Fake {
    hosts: Mutex<Map<String, Value>>,
    decisions: Decisions,
    steps: Mutex<Vec<&'static str>>,
    recipe: Vec<String>,
    meanwhile: Meanwhile,
    /// the step before which `meanwhile` happens
    at: &'static str,
    save_fails: bool,
}

fn fake(meanwhile: Meanwhile, at: &'static str) -> Fake {
    let hosts = json!({"devbox": {"ssh": ["alex@devbox.example"]}, "other": {"ssh": ["alex@other.example"], "files": "enabled"}});
    let f = Fake { hosts: Mutex::new(hosts.as_object().unwrap().clone()), decisions: Decisions::default(), steps: Mutex::new(vec![]),
                   recipe: vec!["alex@devbox.example".into()], meanwhile, at, save_fails: false };
    // as consent::enable does before its steps
    f.decisions.make("devbox", Decision::Enable);
    f
}

impl Fake {
    fn step(&self, name: &'static str) {
        if name == self.at {
            let h = &mut self.hosts.lock();
            match self.meanwhile {
                Meanwhile::Nothing => {}
                Meanwhile::Decline => record(&self.decisions, h, "devbox", Some("declined")),
                Meanwhile::DeclineThenEnable => {
                    record(&self.decisions, h, "devbox", Some("declined"));
                    self.decisions.make("devbox", Decision::Enable);
                }
                Meanwhile::Remove => record(&self.decisions, h, "devbox", None),
                Meanwhile::Moved => h["devbox"]["ssh"] = json!(["alex@devbox2.example"]),
            }
        }
        self.steps.lock().push(name);
    }

    fn files(&self, host: &str) -> Value {
        self.hosts.lock()[host]["files"].clone()
    }
}

impl Steps for Fake {
    async fn install(&self) -> Result<(), String> {
        self.step("install");
        Ok(())
    }
    async fn trial(&self) -> Result<(), String> {
        self.step("trial");
        Ok(())
    }
    fn save(&self, f: &mut dyn FnMut(&mut Map<String, Value>)) -> Result<(), String> {
        self.step("save");
        if self.save_fails {
            return Err("settings.json: cannot be written".into());
        }
        f(&mut self.hosts.lock());
        Ok(())
    }
    fn recipe(&self) -> &[String] {
        &self.recipe
    }
}

fn run<T>(f: impl std::future::Future<Output = T>) -> T {
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(f)
}

#[test]
fn an_enable_records_enabled_and_keeps_every_other_host() {
    // AC-83 happy path
    let f = fake(Meanwhile::Nothing, "");
    assert_eq!(run(enable_with(&f, &f.decisions, "devbox")), Ok(()));
    assert_eq!(f.files("devbox"), "enabled");
    assert_eq!(f.files("other"), "enabled", "another host's record is kept");
    assert_eq!(*f.steps.lock(), ["install", "trial", "save"]);
}

#[test]
fn not_now_during_an_enable_stands_and_stops_it_at_its_next_step() {
    // AC-83 edge — Not now or Remove while enabling: before each step and just before the save
    for (at, steps) in [("trial", &["install", "trial"][..]), ("save", &["install", "trial", "save"][..])] {
        let f = fake(Meanwhile::Decline, at);
        assert_eq!(run(enable_with(&f, &f.decisions, "devbox")), Err(OVERTAKEN.into()), "at {at}");
        assert_eq!(f.files("devbox"), "declined", "at {at}: the later decision is never written over");
        assert_eq!(*f.steps.lock(), steps, "at {at}: no step after the trial's check");
    }
}

#[test]
fn a_removal_during_an_enable_stands_even_when_files_is_back_to_none() {
    // AC-83 edge: "files" was none at the start and is none again; the decision, not the value, counts
    let f = fake(Meanwhile::Remove, "save");
    assert_eq!(run(enable_with(&f, &f.decisions, "devbox")), Err(OVERTAKEN.into()));
    assert!(f.files("devbox").is_null());
}

#[test]
fn enable_again_after_not_now_is_the_last_decision() {
    // AC-83 edge: the user's later Enable keeps the running setup going
    let f = fake(Meanwhile::DeclineThenEnable, "save");
    assert_eq!(run(enable_with(&f, &f.decisions, "devbox")), Ok(()));
    assert_eq!(f.files("devbox"), "enabled");
}

#[test]
fn a_new_recipe_meanwhile_is_not_consented_to() {
    // AC-83 failure: the recipe the user enabled is gone
    let f = fake(Meanwhile::Moved, "save");
    let err = run(enable_with(&f, &f.decisions, "devbox")).unwrap_err();
    assert!(err.contains("changed while it was set up"), "{err}");
    assert!(f.files("devbox").is_null());
}

#[test]
fn a_failed_save_is_the_enables_failure() {
    // AC-83 failure: settings.json not written after a good trial ends the enable with its reason
    let mut f = fake(Meanwhile::Nothing, "");
    f.save_fails = true;
    let err = run(enable_with(&f, &f.decisions, "devbox")).unwrap_err();
    assert!(err.contains("cannot be written"), "{err}");
}

#[test]
fn an_operations_end_keeps_a_later_decision() {
    let d = Decisions::default();
    d.make("devbox", Decision::Enable);
    d.make("devbox", Decision::Remove);
    d.done("devbox", Decision::Enable);
    assert!(d.is("devbox", Decision::Remove), "the removal asked later still runs");
    d.done("devbox", Decision::Remove);
    assert!(!d.is("devbox", Decision::Remove) && !d.is("devbox", Decision::Enable));
}

#[test]
fn every_failed_enable_leaves_enabling() {
    // AC-83 failure: Enable tries again; it never answers at once without doing anything
    let root = run(async {
        let (root, app) = crate::testutil::app("consent-finish");
        let before = begin(&app, "devbox").expect("none runs");
        assert_eq!(crate::hosts::tunnel::state(&app, "devbox"), "enabling");
        assert!(begin(&app, "devbox").is_none(), "the tick and a click together: one runs");
        let err = finish(&app, "devbox", before, Err("settings.json: cannot be written".into())).unwrap_err();
        assert_eq!(err, "Could not enable devbox: settings.json: cannot be written");
        assert_eq!(crate::hosts::tunnel::state(&app, "devbox"), "down");
        // an agents-only connection runs: it keeps its own state
        let task = tokio::spawn(std::future::pending::<()>());
        {
            let mut g = app.tunnels.0.lock();
            let a = g.entry("devbox".into()).or_default();
            (a.state, a.task) = ("up".into(), Some(task));
        }
        let before = begin(&app, "devbox").expect("an agents-only connection is not an enable");
        let _ = finish(&app, "devbox", before, Err("no helper for FreeBSD amd64".into()));
        assert_eq!(crate::hosts::tunnel::state(&app, "devbox"), "up");
        assert!(begin(&app, "devbox").is_some(), "Enable can be tried again");
        crate::hosts::tunnel::stop(&app, "devbox");
        root
    });
    let _ = std::fs::remove_dir_all(&root);
}
