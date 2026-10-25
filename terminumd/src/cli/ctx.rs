// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! What one run of the `terminum` command works with: the environment it was given, the
//! folders derived from it (terminumd reads the same: TM_HOME, default ~/.terminum; TM_APP_DIR,
//! default <home>/state; for tests TM_LAUNCH_AGENTS, default ~/Library/LaunchAgents), the
//! launchd agent's label, and where its output goes. Tests build one directly, so nothing
//! here reads the process's environment after `from_env`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Child;
use std::time::Duration;

use parking_lot::Mutex;

/// The live agent's label: one per user, so only the real install may load it.
pub const LABEL: &str = "io.terminum.terminumd";
/// The only other labels accepted (TM_LAUNCHD_LABEL): the opt-in launchd test's own agent.
pub const TEST_LABEL_PREFIX: &str = "io.terminum.test-";
pub const SYSTEM_LAUNCHCTL: &str = "/bin/launchctl";
/// Seconds a new build has to report itself (AC-16).
pub const WAIT: Duration = Duration::from_secs(10);

pub struct Ctx {
    pub env: HashMap<String, String>,
    /// HOME as given: messages show paths under it as ~
    pub home: PathBuf,
    /// the account's home folder (getpwuid), which launchd's live agent belongs to
    pub real_home: PathBuf,
    pub root: PathBuf,
    pub app_dir: PathBuf,
    /// the LaunchAgents folder the plist goes to
    pub agents: PathBuf,
    pub label: String,
    /// TM_NO_LAUNCHD=1: terminumd as a plain process with a pid file
    pub direct: bool,
    pub launchctl: PathBuf,
    /// what opens a sign-in link in the browser
    pub opener: PathBuf,
    pub wait: Duration,
    /// the package or build folder this command came in (BUILD, release-signers, agents/)
    pub pkg: Option<PathBuf>,
    /// stdout is a terminal
    pub tty: bool,
    /// lines said, kept instead of printed (tests)
    pub captured: Option<Mutex<Vec<String>>>,
    /// terminumd processes started in direct mode by this process, reaped when stopped
    pub children: Mutex<HashMap<u32, Child>>,
}

/// The account's home folder from the password database, not from $HOME.
pub fn account_home() -> PathBuf {
    // SAFETY: getpwuid returns a pointer into static storage or null; read at once
    unsafe {
        let pw = libc::getpwuid(libc::getuid());
        if pw.is_null() || (*pw).pw_dir.is_null() {
            return PathBuf::from("/");
        }
        let dir = std::ffi::CStr::from_ptr((*pw).pw_dir);
        PathBuf::from(<std::ffi::OsStr as std::os::unix::ffi::OsStrExt>::from_bytes(dir.to_bytes()))
    }
}

impl Ctx {
    /// A context for `env` (HOME and the TM_ variables); Err for a label that is not allowed.
    pub fn new(env: HashMap<String, String>, real_home: PathBuf) -> Result<Ctx, String> {
        let get = |k: &str| env.get(k).filter(|v| !v.is_empty()).map(PathBuf::from);
        let home = get("HOME").unwrap_or_else(|| real_home.clone());
        let root = get("TM_HOME").unwrap_or_else(|| home.join(".terminum"));
        let app_dir = get("TM_APP_DIR").unwrap_or_else(|| root.join("state"));
        let agents = get("TM_LAUNCH_AGENTS").unwrap_or_else(|| home.join("Library/LaunchAgents"));
        let label = match env.get("TM_LAUNCHD_LABEL").filter(|v| !v.is_empty()) {
            None => LABEL.to_string(),
            Some(l) if test_label(l) => l.clone(),
            Some(l) => return Err(format!("TM_LAUNCHD_LABEL {l}: only {TEST_LABEL_PREFIX}<letters, digits, -> is accepted")),
        };
        Ok(Ctx {
            direct: env.get("TM_NO_LAUNCHD").is_some_and(|v| v == "1"),
            env,
            home,
            real_home,
            root,
            app_dir,
            agents,
            label,
            launchctl: PathBuf::from(SYSTEM_LAUNCHCTL),
            opener: PathBuf::from("/usr/bin/open"),
            wait: WAIT,
            pkg: None,
            tty: false,
            captured: None,
            children: Mutex::new(HashMap::new()),
        })
    }

    /// The context of this process: its environment, its terminal, its package.
    pub fn from_env(argv0: &str) -> Result<Ctx, String> {
        let mut ctx = Ctx::new(std::env::vars().collect(), account_home())?;
        // SAFETY: isatty has no preconditions
        ctx.tty = unsafe { libc::isatty(1) } == 1;
        ctx.pkg = package_of(argv0);
        Ok(ctx)
    }

    pub fn var(&self, key: &str) -> Option<&str> {
        self.env.get(key).map(String::as_str)
    }

    pub fn log_dir(&self) -> PathBuf {
        self.root.join("logs")
    }
    pub fn log(&self) -> PathBuf {
        self.log_dir().join("terminumd.log")
    }
    pub fn sock(&self) -> PathBuf {
        self.app_dir.join("terminumd.sock")
    }
    pub fn pidfile(&self) -> PathBuf {
        self.app_dir.join("terminumd.pid")
    }
    pub fn plist(&self) -> PathBuf {
        self.agents.join(format!("{}.plist", self.label))
    }
    pub fn lib(&self) -> PathBuf {
        self.root.join("builds")
    }
    pub fn bin(&self) -> PathBuf {
        self.root.join("bin")
    }
    /// What launchd runs: a copy of the live build's terminumd at a path that never changes
    /// (install.rs place_daemon), so macOS keeps its folder grants across upgrades (AC-08).
    pub fn program(&self) -> PathBuf {
        self.root.join("daemon/terminumd")
    }

    /// `path` with the home folder as ~, as messages name it.
    pub fn shown(&self, path: &Path) -> String {
        match path.strip_prefix(&self.home) {
            Ok(rel) => format!("~/{}", rel.display()),
            Err(_) => path.display().to_string(),
        }
    }

    /// One line of output (kept by a test's context).
    pub fn say(&self, line: impl Into<String>) {
        let line = line.into();
        match &self.captured {
            Some(lines) => lines.lock().push(line),
            None => {
                use std::io::Write;
                let mut out = std::io::stdout().lock();
                let _ = writeln!(out, "{line}");
                let _ = out.flush();
            }
        }
    }

    /// Colors only for a person at a terminal that takes them (NO_COLOR: no-color.org).
    pub fn color(&self) -> bool {
        self.tty && !self.env.contains_key("NO_COLOR") && self.var("TERM").unwrap_or("dumb") != "dumb"
    }

    /// A success line: a green check where colors work, the bare text elsewhere.
    pub fn done(&self, msg: &str) {
        self.say(if self.color() { format!("\x1b[32m✓\x1b[0m {msg}") } else { msg.to_string() });
    }
}

pub fn test_label(l: &str) -> bool {
    l.strip_prefix(TEST_LABEL_PREFIX).is_some_and(|rest| !rest.is_empty() && rest.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
}

/// The package or build folder of the binary run as `argv0`: <pkg>/agents/<platform>/terminumd.
/// argv0 when it names a path (a wrapper that execs with `exec -a` keeps it), else the
/// executable itself; links resolved either way (bin/terminum → builds/<b>/terminumd → …).
pub fn package_of(argv0: &str) -> Option<PathBuf> {
    let exe = if argv0.contains('/') { PathBuf::from(argv0) } else { std::env::current_exe().ok()? };
    let real = std::fs::canonicalize(exe).ok()?;
    Some(real.parent()?.parent()?.parent()?.to_path_buf())
}
