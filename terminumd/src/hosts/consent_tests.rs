// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
use super::*;
use serde_json::json;

/// What another page or `terminum hosts` sends while an enable runs, right after which step.
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
    /// the step right after which `meanwhile` happens (in the save: before its change)
    at: &'static str,
    save_fails: bool,
}

fn fake(meanwhile: Meanwhile, at: &'static str) -> Fake {
    let hosts = json!({"devbox": {"ssh": ["alex@devbox.example"]}, "other": {"ssh": ["alex@other.example"], "files": "enabled"}});
    let f = Fake { hosts: Mutex::new(hosts.as_object().unwrap().clone()), decisions: Decisions::default(), steps: Mutex::new(vec![]),
                   recipe: vec!["alex@devbox.example".into()], meanwhile, at, save_fails: false };
    assert!(matches!(f.decisions.enable("devbox"), Start::Run(..)), "as consent::enable does before its steps");
    f
}

impl Fake {
    fn step(&self, name: &'static str) {
        self.steps.lock().push(name);
        if name == self.at {
            let h = &mut self.hosts.lock();
            match self.meanwhile {
                Meanwhile::Nothing => {}
                Meanwhile::Decline => drop(record(&self.decisions, h, "devbox", Some("declined"))),
                Meanwhile::DeclineThenEnable => {
                    record(&self.decisions, h, "devbox", Some("declined"));
                    assert!(matches!(self.decisions.enable("devbox"), Start::Run(..)), "after Not now a click runs");
                }
                Meanwhile::Remove => drop(record(&self.decisions, h, "devbox", None)),
                Meanwhile::Moved => h["devbox"]["ssh"] = json!(["alex@devbox2.example"]),
            }
        }
    }

    fn files(&self, host: &str) -> Value {
        self.hosts.lock()[host]["files"].clone()
    }
}

impl Steps for Fake {
    async fn probe(&self) -> Result<&'static str, String> {
        self.step("probe");
        Ok("linux-x86_64")
    }
    async fn copy(&self, _: &'static str) -> Result<(), String> {
        self.step("copy");
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
    async fn discard(&self) {
        self.step("discard");
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
    assert_eq!(f.hosts.lock()["devbox"]["helper"], crate::agent::AGENT_ID, "in the same write");
    assert_eq!(f.files("other"), "enabled", "another host's record is kept");
    assert_eq!(*f.steps.lock(), ["probe", "copy", "trial", "save"]);
}

#[test]
fn not_now_during_an_enable_stands_and_no_step_follows_it() {
    // AC-83 edge — a later decision while enabling: after each step and just before the save
    let all = ["probe", "copy", "trial", "save"];
    for (at, taken) in [("probe", 1), ("copy", 2), ("trial", 3), ("save", 4)] {
        let f = fake(Meanwhile::Decline, at);
        assert_eq!(run(enable_with(&f, &f.decisions, "devbox")), Err(OVERTAKEN.into()), "after {at}");
        assert_eq!(f.files("devbox"), "declined", "after {at}: the later decision is never written over");
        let steps = f.steps.lock();
        assert_eq!(steps[..taken], all[..taken], "after {at}: not copied after a Not now during the probe either");
        // a helper copied for nothing is not left on the host with Files off
        assert_eq!(steps[taken..], if taken > 1 { vec!["discard"] } else { vec![] }, "after {at}");
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
    assert_eq!(f.steps.lock().last(), Some(&"discard"), "the helper copied for nothing goes");
}

#[test]
fn an_operations_end_takes_only_its_own_decision() {
    let d = Decisions::default();
    let Start::Run(first, _) = d.enable("devbox") else { panic!("none runs") };
    assert!(matches!(d.enable("devbox"), Start::Join(_)), "the setup by itself and a click together: one runs");
    d.withdraw("devbox");
    let Start::Run(again, _) = d.enable("devbox") else { panic!("after Not now a click runs") };
    assert!(!d.done("devbox", first), "the first enable's end keeps the click after it");
    assert!(d.is("devbox", Decision::Enable));
    assert!(d.done("devbox", again));
    let removal = d.remove("devbox");
    assert!(matches!(d.enable("devbox"), Start::Refused), "AC-83: Enable waits for the removal to end");
    assert!(matches!(d.enable("other"), Start::Run(..)), "another host is not held up");
    d.withdraw("devbox");
    assert!(!d.done("devbox", removal), "Not now during the removal stands: files is not cleared");
}

#[test]
fn a_host_is_enabling_exactly_while_its_enable_runs() {
    // AC-83 failure: whichever step an enable fails at, its end leaves "enabling", and Enable
    // tries again instead of answering at once
    let root = run(async {
        let (root, app) = crate::testutil::app("consent-enabling");
        let d = &app.hosts.decisions;
        let Start::Run(ticket, _to) = d.enable("devbox") else { panic!("none runs") };
        assert_eq!(crate::hosts::tunnel::state(&app, "devbox"), "enabling");
        assert_eq!(crate::hosts::offer(&app, "devbox")["state"], "enabling");
        d.done("devbox", ticket);
        assert_eq!(crate::hosts::tunnel::state(&app, "devbox"), "down");
        assert!(matches!(d.enable("devbox"), Start::Run(..)), "Enable can be tried again");
        root
    });
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn an_enable_that_waited_for_its_turn_takes_no_step_after_not_now() {
    // AC-83 edge: Not now came while another operation at the helper ran
    let f = fake(Meanwhile::Nothing, "");
    record(&f.decisions, &mut f.hosts.lock(), "devbox", Some("declined"));
    assert_eq!(run(enable_with(&f, &f.decisions, "devbox")), Err(OVERTAKEN.into()));
    assert!(f.steps.lock().is_empty(), "the host is not reached");
}

#[test]
fn a_later_enable_gets_the_result_of_the_one_running() {
    // AC-83 edge: neither Ok at once nor a second copy
    run(async {
        for result in [Ok(()), Err("Could not enable devbox: ssh: Connection refused".to_string())] {
            let d = Decisions::default();
            let Start::Run(ticket, to) = d.enable("devbox") else { panic!("none runs") };
            let Start::Join(click) = d.enable("devbox") else { panic!("the second joins") };
            let Start::Join(cli) = d.enable("devbox") else { panic!("the third joins") };
            let waiting = tokio::spawn(joined(click));
            tokio::task::yield_now().await;
            assert!(!waiting.is_finished(), "no answer before the enable ends");
            d.done("devbox", ticket);
            to.send(Some(result.clone())).unwrap();
            assert_eq!(waiting.await.unwrap(), result);
            assert_eq!(joined(cli).await, result, "one that asks for it after the end too");
        }
    });
}

#[test]
fn a_later_enable_hears_that_the_one_running_ended_without_a_result() {
    // AC-83 failure: no wait without end
    run(async {
        let d = Decisions::default();
        let Start::Run(_, to) = d.enable("devbox") else { panic!("none runs") };
        let Start::Join(click) = d.enable("devbox") else { panic!("the second joins") };
        drop(to);
        assert_eq!(joined(click).await, Err(STOPPED.into()));
    });
}
