//! Hermetic environments for child processes spawned by tests.
//!
//! A developer machine carries state that a CI runner does not: an agent home,
//! provider config roots, credential files, auth-sync and remote-auth hooks, a
//! global git config, XDG directories. A child that inherits the caller's
//! environment reads and writes all of it, so the suite passes in CI and quietly
//! damages (or leaks) real state locally (sympoies/nils-cli#2755).
//!
//! [`apply`] starts a [`Command`] from a cleared environment plus the explicit
//! [`ENV_ALLOWLIST`], and points every home and config root at a per-spawn temp
//! directory. A test that needs any other variable sets it explicitly with
//! `CmdOptions::with_env`, which is applied after this and therefore wins.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::Path;
use std::process::Command;
use std::sync::OnceLock;

/// Variables copied from the caller into a hermetic child.
///
/// These describe the toolchain and terminal, not user state: `PATH` to resolve
/// tools, locale and terminal settings, the temp root, coverage and backtrace
/// plumbing, and the runtime directory that service-manager probes need.
pub const ENV_ALLOWLIST: [&str; 10] = [
    "PATH",
    "LANG",
    "LANGUAGE",
    "TERM",
    "TZ",
    "TMPDIR",
    "LLVM_PROFILE_FILE",
    "RUST_BACKTRACE",
    "RUST_LIB_BACKTRACE",
    "XDG_RUNTIME_DIR",
];

/// Variables copied when their name starts with one of these prefixes.
pub const ENV_ALLOW_PREFIXES: [&str; 1] = ["LC_"];

/// Windows variables a child needs to resolve and start programs at all.
#[cfg(windows)]
const PLATFORM_ALLOWLIST: [&str; 3] = ["SYSTEMROOT", "PATHEXT", "COMSPEC"];
#[cfg(not(windows))]
const PLATFORM_ALLOWLIST: [&str; 0] = [];

/// Per-spawn home layout, relative to the temp root.
const HOME_DIRS: [(&str, &str); 7] = [
    ("HOME", "home"),
    ("AGENT_HOME", "agent-home"),
    ("CODEX_HOME", "codex-home"),
    ("CLAUDE_CONFIG_DIR", "claude-home"),
    ("XDG_CONFIG_HOME", "xdg-config"),
    ("XDG_STATE_HOME", "xdg-state"),
    ("XDG_DATA_HOME", "xdg-data"),
];

/// A temp root holding one hermetic home layout; removed on drop.
///
/// Keep it alive until the child has exited: the child's `HOME`, `AGENT_HOME`,
/// `CODEX_HOME`, `CLAUDE_CONFIG_DIR`, and `XDG_*` directories live inside it.
pub struct HermeticHome {
    dir: tempfile::TempDir,
}

impl HermeticHome {
    pub fn new() -> Self {
        let dir = tempfile::Builder::new()
            .prefix("nils-hermetic-")
            .tempdir()
            .expect("create hermetic home root");
        for (_, name) in HOME_DIRS {
            std::fs::create_dir_all(dir.path().join(name)).expect("create hermetic home dir");
        }
        std::fs::create_dir_all(dir.path().join("xdg-cache")).expect("create hermetic cache dir");
        Self { dir }
    }

    pub fn path(&self) -> &Path {
        self.dir.path()
    }
}

impl Default for HermeticHome {
    fn default() -> Self {
        Self::new()
    }
}

fn allowed(name: &str, extra: &[String]) -> bool {
    ENV_ALLOWLIST.contains(&name)
        || PLATFORM_ALLOWLIST.contains(&name)
        || ENV_ALLOW_PREFIXES
            .iter()
            .any(|prefix| name.starts_with(prefix))
        || extra.iter().any(|allowed| allowed == name)
}

type Vars = BTreeMap<OsString, OsString>;

static AMBIENT_BASELINE: OnceLock<Vars> = OnceLock::new();

/// The environment the test process started with, captured once.
///
/// `EnvGuard` and `GlobalStateLock` capture it before the first in-process change,
/// so a variable that differs from it later was set (or removed) by the test and
/// is intent, while one that equals it is ambient state inherited from the caller.
pub fn ambient_baseline() -> &'static BTreeMap<OsString, OsString> {
    AMBIENT_BASELINE.get_or_init(|| std::env::vars_os().collect())
}

/// True when `name` still has the value the process inherited, or is still absent
/// as it was at start: the caller's ambient state, not something the test set.
///
/// Product code under `#[cfg(test)]` uses this to ignore an inherited home root
/// (for example `AGENT_HOME`) while still honoring one a test pointed at a fixture.
pub fn is_ambient(name: &str) -> bool {
    ambient_baseline().get(std::ffi::OsStr::new(name)) == std::env::var_os(name).as_ref()
}

/// Replace `command`'s environment with the allowlisted caller variables, `extra`
/// additional pass-through names, the variables the test changed in-process, and
/// the per-spawn `home` layout.
///
/// Call this before applying a test's own variables: those are set afterwards and
/// override anything here.
pub fn apply(command: &mut Command, home: &HermeticHome, extra: &[String]) {
    apply_from(
        command,
        home,
        extra,
        std::env::vars_os().collect(),
        ambient_baseline(),
    );
}

/// [`apply`] over explicit caller and baseline variables instead of the process
/// environment, so the policy can be tested without mutating global state.
pub fn apply_from(
    command: &mut Command,
    home: &HermeticHome,
    extra: &[String],
    caller: Vars,
    baseline: &Vars,
) {
    let changed = |name: &OsString| baseline.get(name) != caller.get(name);
    command.env_clear();
    for (name, value) in &caller {
        let keep = name.to_str().is_some_and(|plain| allowed(plain, extra)) || changed(name);
        if keep {
            command.env(name, value);
        }
    }
    // Home and config roots are substituted unless the test changed them itself
    // (set to a fixture, or removed to exercise a default).
    let substitute = |name: &str| !changed(&OsString::from(name));
    for (name, dir) in HOME_DIRS {
        if substitute(name) {
            command.env(name, home.path().join(dir));
        }
    }
    if substitute("XDG_CACHE_HOME") {
        command.env("XDG_CACHE_HOME", home.path().join("xdg-cache"));
    }
    if substitute("GIT_CONFIG_GLOBAL") {
        command.env("GIT_CONFIG_GLOBAL", home.path().join("gitconfig"));
    }
    if substitute("GIT_CONFIG_NOSYSTEM") {
        command.env("GIT_CONFIG_NOSYSTEM", OsString::from("1"));
    }
}

/// Make `command` hermetic and return the home that must outlive the child.
///
/// For tests that build a `Command` themselves (to spawn without waiting, pipe
/// stdio, or keep the child) instead of going through `CmdOptions`. Call it before
/// setting the command's own variables, and hold the returned home until the
/// child has exited.
#[must_use = "dropping the home removes the child's HOME and XDG directories"]
pub fn isolate(command: &mut Command) -> HermeticHome {
    let home = HermeticHome::new();
    apply(command, &home, &[]);
    home
}

/// Spawn-ready hermetic [`Command`] for `program`, plus the home that must outlive
/// the child. For tests that spawn directly instead of through `CmdOptions`.
pub fn command(program: impl AsRef<std::ffi::OsStr>) -> (Command, HermeticHome) {
    let home = HermeticHome::new();
    let mut command = Command::new(program);
    apply(&mut command, &home, &[]);
    (command, home)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn child_env(command: &mut Command) -> std::collections::BTreeMap<String, String> {
        let output = command.output().expect("run env");
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter_map(|line| line.split_once('='))
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect()
    }

    fn vars(pairs: &[(&str, &str)]) -> Vars {
        pairs
            .iter()
            .map(|(name, value)| (OsString::from(name), OsString::from(value)))
            .collect()
    }

    fn run_env(
        baseline: &[(&str, &str)],
        caller: &[(&str, &str)],
        extra: &[&str],
    ) -> (HermeticHome, BTreeMap<String, String>) {
        let home = HermeticHome::new();
        let mut command = Command::new("env");
        let extra: Vec<String> = extra.iter().map(|name| name.to_string()).collect();
        apply_from(&mut command, &home, &extra, vars(caller), &vars(baseline));
        (home, child_env(&mut command))
    }

    #[test]
    fn ambient_variables_are_cleared_and_homes_point_at_the_temp_root() {
        let ambient = [
            ("CODEX_PROXY_AUTH_SYNC_COMMAND", "poison"),
            ("CODEX_AUTH_REMOTE_SSH", "poison"),
            ("GIT_CONFIG_COUNT", "1"),
            ("AGENT_SESSION_ID", "poison"),
            ("HOME", "/real/home"),
            ("LC_ALL", "C"),
            ("TERM", "dumb"),
        ];
        let (home, env) = run_env(&ambient, &ambient, &[]);

        for name in [
            "CODEX_PROXY_AUTH_SYNC_COMMAND",
            "CODEX_AUTH_REMOTE_SSH",
            "GIT_CONFIG_COUNT",
            "AGENT_SESSION_ID",
        ] {
            assert_eq!(env.get(name), None, "{name} must not reach the child");
        }
        for (name, dir) in HOME_DIRS {
            assert_eq!(
                env.get(name).map(String::as_str),
                Some(home.path().join(dir).to_string_lossy().as_ref()),
                "{name}"
            );
        }
        assert_eq!(
            env.get("GIT_CONFIG_GLOBAL").map(String::as_str),
            Some(home.path().join("gitconfig").to_string_lossy().as_ref())
        );
        assert_eq!(
            env.get("GIT_CONFIG_NOSYSTEM").map(String::as_str),
            Some("1")
        );
        assert_eq!(env.get("LC_ALL").map(String::as_str), Some("C"));
        assert_eq!(env.get("TERM").map(String::as_str), Some("dumb"));
    }

    #[test]
    fn variables_the_test_changed_in_process_reach_the_child() {
        let baseline = [
            ("HOME", "/real/home"),
            ("CODEX_HOME", "/real/codex"),
            ("AGENT_HOOK_BIN", "/real/hook"),
        ];
        // The test set HOME and a new variable, changed AGENT_HOOK_BIN, and removed CODEX_HOME.
        let caller = [
            ("HOME", "/fixture/home"),
            ("AGENT_HOOK_BIN", "/fixture/hook"),
            ("FIXTURE_ONLY", "set"),
        ];
        let (home, env) = run_env(&baseline, &caller, &[]);
        assert_eq!(env.get("HOME").map(String::as_str), Some("/fixture/home"));
        assert_eq!(
            env.get("AGENT_HOOK_BIN").map(String::as_str),
            Some("/fixture/hook")
        );
        assert_eq!(env.get("FIXTURE_ONLY").map(String::as_str), Some("set"));
        assert_eq!(env.get("CODEX_HOME"), None, "a removal is intent too");
        assert_eq!(
            env.get("AGENT_HOME").map(String::as_str),
            Some(home.path().join("agent-home").to_string_lossy().as_ref()),
            "unchanged homes are still substituted"
        );
    }

    #[test]
    fn explicit_pass_through_and_later_overrides_win() {
        let ambient = [
            ("NILS_HERMETIC_PASS", "kept"),
            ("NILS_HERMETIC_DROP", "gone"),
        ];
        let home = HermeticHome::new();
        let mut command = Command::new("env");
        apply_from(
            &mut command,
            &home,
            &["NILS_HERMETIC_PASS".to_string()],
            vars(&ambient),
            &vars(&ambient),
        );
        command.env("HOME", "/explicit/home");
        let env = child_env(&mut command);
        assert_eq!(
            env.get("NILS_HERMETIC_PASS").map(String::as_str),
            Some("kept")
        );
        assert_eq!(env.get("NILS_HERMETIC_DROP"), None);
        assert_eq!(env.get("HOME").map(String::as_str), Some("/explicit/home"));
    }
}
