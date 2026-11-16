//! Per-session `TMPDIR` (agent write-location contract, class `tmp`).
//!
//! Each managed session gets `${AGENT_HOME}/tmp/<session-id>/` (mode 0700),
//! injected as `TMPDIR` at launch. Once the session stops, the directory is
//! reclaimed after a grace period; a sweep also removes directories whose
//! session record is gone. A directory is only ever touched when it carries this
//! module's owner file naming the same session and state directory, so other
//! tools' entries under `${AGENT_HOME}/tmp` and other agent-session state
//! directories are never swept.
use super::*;

pub(crate) const OPT_OUT_KEY: &str = "AGENT_RUNTIME_SESSION_TMPDIR";
pub(crate) const OPT_OUT_VALUE: &str = "system";
pub(crate) const GRACE_ENV: &str = "AGENT_SESSION_TMP_GRACE_SECS";
const OWNER_SCHEMA: &str = "agent-session.session-tmp.v1";
const OWNER_FILE: &str = ".agent-session-tmp";
const DEFAULT_GRACE: Duration = Duration::from_secs(24 * 60 * 60);
/// A launch only sweeps opportunistically; it never waits on many tmux probes.
const LAUNCH_SWEEP_BUDGET: Duration = Duration::from_secs(2);

#[derive(Debug, Deserialize, Serialize)]
struct Owner {
    schema_version: String,
    session_id: String,
    state_dir: String,
    #[serde(default)]
    stopped_at_epoch: Option<u64>,
}

mod sweep;
pub(crate) use sweep::sweep;

fn agent_home() -> Result<PathBuf, CliError> {
    if let Some(path) = non_empty_env("AGENT_HOME") {
        return absolute_path(Path::new(&path));
    }
    if let Some(path) = non_empty_env("XDG_STATE_HOME") {
        return absolute_path(&PathBuf::from(path).join("agent-runtime-kit"));
    }
    let home = home_dir().ok_or_else(|| {
        CliError::runtime(
            "home-unavailable",
            "HOME is unset; set AGENT_HOME",
            Some(json!({ "env": "AGENT_HOME" })),
        )
    })?;
    Ok(normalize_path(&home.join(".local/state/agent-runtime-kit")))
}

fn tmp_root(state_dir: &Path) -> Result<PathBuf, CliError> {
    // Unit tests launch sessions against temporary state directories; keep their
    // tmp directories out of the developer's real AGENT_HOME, whether it is unset
    // or inherited from the caller. A test that set AGENT_HOME itself keeps it.
    #[cfg(test)]
    if non_empty_env("AGENT_HOME").is_none()
        || nils_test_support::hermetic::is_ambient("AGENT_HOME")
    {
        return Ok(state_dir.join("agent-home/tmp"));
    }
    let _ = state_dir;
    Ok(agent_home()?.join("tmp"))
}

pub(crate) fn grace_from_env() -> Duration {
    non_empty_env(GRACE_ENV)
        .and_then(|value| value.trim().parse::<u64>().ok())
        .map_or(DEFAULT_GRACE, Duration::from_secs)
}

fn now_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

fn io_error(code: &'static str, path: &Path, err: io::Error) -> CliError {
    CliError::runtime(
        code,
        format!("{}: {err}", path.display()),
        Some(json!({ "path": display_path(path) })),
    )
}

/// The owner file of `dir` when it names `id`.
fn read_owner(dir: &Path, id: &str) -> Option<Owner> {
    sweep::read_owned_owner(dir, id)
}

/// The owner file of `dir` when it names `id` and `state_dir`.
fn owner_of(dir: &Path, id: &str, state_dir: &Path) -> Option<Owner> {
    read_owner(dir, id).filter(|owner| owner.state_dir == display_path(state_dir))
}

fn write_owner(dir: &Path, owner: &Owner) -> Result<(), CliError> {
    let bytes = serde_json::to_vec(owner)
        .map_err(|err| CliError::runtime("session-tmpdir-owner-invalid", err.to_string(), None))?;
    write_atomic(&dir.join(OWNER_FILE), &bytes, SECRET_FILE_MODE).map_err(|err| {
        CliError::runtime(
            "session-tmpdir-owner-write-failed",
            err.to_string(),
            Some(json!({ "path": display_path(dir) })),
        )
    })
}

fn expired(stopped_at: u64, grace: Duration) -> bool {
    now_epoch()
        .checked_sub(stopped_at)
        .is_some_and(|age| age >= grace.as_secs())
}

/// Create the session's `TMPDIR` for a launch and return it, or `None` when the
/// session keeps the system `TMPDIR`. A directory left by a stopped earlier run
/// of the same session is reclaimed first once its grace period has passed.
pub(crate) fn prepare_launch(
    state_dir: &Path,
    record: &SessionRecord,
) -> Result<Option<PathBuf>, CliError> {
    let launch_env = launch_env::from_record(record)?;
    if launch_env.get(OPT_OUT_KEY).map(String::as_str) == Some(OPT_OUT_VALUE) {
        return Ok(None);
    }
    validate_id(&record.id)?;
    let root = tmp_root(state_dir)?;
    let dir = root.join(&record.id);
    let grace = grace_from_env();
    fs::create_dir_all(&root)
        .map_err(|err| io_error("session-tmpdir-create-failed", &root, err))?;
    let context = CliContext {
        state_dir: state_dir.to_path_buf(),
        host: None,
    };
    // Best effort: a failed sweep never blocks the launch it piggybacks on.
    let _ = sweep(
        &context,
        &resolve_tmux_bin(None),
        grace,
        Some(LAUNCH_SWEEP_BUDGET),
    );
    match fs::symlink_metadata(&dir) {
        Ok(meta) if meta.is_dir() => {
            let stopped_at =
                owner_of(&dir, &record.id, state_dir).and_then(|owner| owner.stopped_at_epoch);
            if stopped_at.is_some_and(|at| expired(at, grace)) {
                fs::remove_dir_all(&dir)
                    .map_err(|err| io_error("session-tmpdir-reclaim-failed", &dir, err))?;
            }
        }
        Ok(_) => {
            return Err(CliError::data(
                "session-tmpdir-unsafe",
                "session tmp path exists and is not a directory",
                Some(json!({ "path": display_path(&dir) })),
            ));
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(err) => return Err(io_error("session-tmpdir-create-failed", &dir, err)),
    }
    ensure_private_dir(&dir)?;
    write_owner(
        &dir,
        &Owner {
            schema_version: OWNER_SCHEMA.to_string(),
            session_id: record.id.clone(),
            state_dir: display_path(state_dir),
            stopped_at_epoch: None,
        },
    )?;
    Ok(Some(dir))
}

/// Start the grace period of a session whose runtime just exited. Best effort:
/// a sweep that first observes the stop starts the period instead.
pub(crate) fn mark_stopped(state_dir: &Path, id: &str) {
    let Ok(root) = tmp_root(state_dir) else {
        return;
    };
    if validate_id(id).is_err() {
        return;
    }
    let dir = root.join(id);
    if let Some(mut owner) = owner_of(&dir, id, state_dir) {
        owner.stopped_at_epoch = Some(now_epoch());
        let _ = write_owner(&dir, &owner);
    }
}

const SWEEP_COMMAND: &str = "tmp-sweep";

pub(crate) fn run(context: &CliContext, args: cli::TmpArgs) -> i32 {
    match args.command {
        cli::TmpCommand::Sweep(args) => {
            let grace = args
                .grace_secs
                .map_or_else(grace_from_env, Duration::from_secs);
            let tmux_bin = resolve_tmux_bin(args.tmux_bin.as_deref());
            match sweep(context, &tmux_bin, grace, None) {
                Ok(report) => {
                    render_single_success(SWEEP_COMMAND, args.format, &report, render_sweep_text)
                }
                Err(err) => render_error(SWEEP_COMMAND, args.format, err),
            }
        }
    }
}

fn render_sweep_text(report: &sweep::SweepReport) -> String {
    format!(
        "grace_secs: {}\nremoved: {}\nretained: {}\n",
        report.grace_secs,
        report.removed,
        report.retained.values().sum::<usize>()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use nils_test_support::{EnvGuard, GlobalStateLock};
    use pretty_assertions::assert_eq;
    use std::os::unix::fs::PermissionsExt;

    const OWNER_FILE: &str = ".agent-session-tmp";

    struct Fixture {
        _lock: GlobalStateLock,
        _home: EnvGuard,
        _grace: EnvGuard,
        dir: tempfile::TempDir,
    }

    impl Fixture {
        fn new() -> Self {
            let lock = GlobalStateLock::new();
            let dir = tempfile::TempDir::new().unwrap();
            let home = EnvGuard::set(
                &lock,
                "AGENT_HOME",
                dir.path().join("home").to_str().unwrap(),
            );
            let grace = EnvGuard::remove(&lock, GRACE_ENV);
            Self {
                _lock: lock,
                _home: home,
                _grace: grace,
                dir,
            }
        }

        fn state_dir(&self) -> PathBuf {
            self.dir.path().join("state")
        }

        fn context(&self) -> CliContext {
            CliContext {
                state_dir: self.state_dir(),
                host: None,
            }
        }

        fn tmp_dir(&self, id: &str) -> PathBuf {
            self.dir.path().join("home/tmp").join(id)
        }

        /// A fake tmux whose `has-session` reports the session running or absent.
        fn tmux(&self, running: bool) -> PathBuf {
            let path = self
                .dir
                .path()
                .join(if running { "tmux-up" } else { "tmux-down" });
            let body = if running {
                "#!/bin/sh\nexit 0\n"
            } else {
                "#!/bin/sh\necho \"can't find session: x\" >&2\nexit 1\n"
            };
            fs::write(&path, body).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
            path
        }

        fn record(&self, id: &str, launch_env: Value) -> SessionRecord {
            serde_json::from_value(json!({
                "schema_version": crate::SESSION_DOCUMENT_VERSION, "id": id, "agent": "codex",
                "mode": "interactive", "title": null, "cwd": self.dir.path(), "tmux_session": id,
                "prompt_file": null, "log_file": null,
                "created_at": "2030-01-01T00:00:00Z", "updated_at": "2030-01-01T00:00:00Z",
                "launch_env": launch_env,
                "runtime": {"kind": "tmux", "tmux_session": id, "generation": 1,
                    "started_at": "2030-01-01T00:00:00Z", "launch_id": "fixture-runtime"},
            }))
            .unwrap()
        }

        fn persist(&self, id: &str) -> SessionRecord {
            let record = self.record(id, json!({}));
            crate::write_session_record(&self.context(), &record).unwrap();
            record
        }
    }

    fn args_of(command: &ProcessCommand) -> Vec<String> {
        command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }

    mod safety;

    #[test]
    fn prepare_launch_creates_a_private_dir_under_agent_home() {
        let fixture = Fixture::new();
        let record = fixture.record("sess-create", json!({}));

        let dir = prepare_launch(&fixture.state_dir(), &record).unwrap();

        assert_eq!(dir, Some(fixture.tmp_dir("sess-create")));
        let mode = fs::metadata(fixture.tmp_dir("sess-create"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o700);
        assert!(fixture.tmp_dir("sess-create").join(OWNER_FILE).is_file());
    }

    #[test]
    fn prepare_launch_leaves_the_system_tmpdir_when_the_session_opts_out() {
        let fixture = Fixture::new();
        let record = fixture.record("sess-optout", json!({ OPT_OUT_KEY: OPT_OUT_VALUE }));

        assert_eq!(prepare_launch(&fixture.state_dir(), &record).unwrap(), None);
        assert!(!fixture.tmp_dir("sess-optout").exists());
    }

    #[test]
    fn runtime_environment_injects_tmpdir_and_keeps_tmux_tmpdir_and_process_env() {
        let fixture = Fixture::new();
        let record = fixture.record("sess-inject", json!({}));
        let mut command = ProcessCommand::new("tmux");
        command.arg("new-session");

        add_runtime_tmux_environment(&mut command, &fixture.state_dir(), &record).unwrap();

        let args = args_of(&command);
        let expected = format!("TMPDIR={}", fixture.tmp_dir("sess-inject").display());
        assert!(args.contains(&expected), "{args:?}");
        assert!(args.iter().all(|arg| !arg.starts_with("TMUX_TMPDIR")));
        assert!(
            command
                .get_envs()
                .all(|(key, _)| key != "TMPDIR" && key != "TMUX_TMPDIR")
        );

        let optout = fixture.record("sess-inject-off", json!({ OPT_OUT_KEY: OPT_OUT_VALUE }));
        let mut command = ProcessCommand::new("tmux");
        add_runtime_tmux_environment(&mut command, &fixture.state_dir(), &optout).unwrap();
        assert!(
            args_of(&command)
                .iter()
                .all(|arg| !arg.starts_with("TMPDIR="))
        );
    }

    #[test]
    fn relaunch_reclaims_an_expired_dir_and_keeps_one_inside_the_grace_period() {
        let fixture = Fixture::new();
        let record = fixture.record("sess-relaunch", json!({}));
        let dir = prepare_launch(&fixture.state_dir(), &record)
            .unwrap()
            .unwrap();
        fs::write(dir.join("scratch.txt"), "data").unwrap();

        mark_stopped(&fixture.state_dir(), "sess-relaunch");
        let _grace = EnvGuard::set(&fixture._lock, GRACE_ENV, "3600");
        prepare_launch(&fixture.state_dir(), &record).unwrap();
        assert!(dir.join("scratch.txt").is_file());

        mark_stopped(&fixture.state_dir(), "sess-relaunch");
        let _grace = EnvGuard::set(&fixture._lock, GRACE_ENV, "0");
        prepare_launch(&fixture.state_dir(), &record).unwrap();
        assert!(!dir.join("scratch.txt").exists());
        assert!(dir.join(OWNER_FILE).is_file());
    }

    #[test]
    fn sweep_never_removes_the_dir_of_a_live_session() {
        let fixture = Fixture::new();
        let record = fixture.persist("sess-live");
        let dir = prepare_launch(&fixture.state_dir(), &record)
            .unwrap()
            .unwrap();
        mark_stopped(&fixture.state_dir(), "sess-live");

        let report = sweep(
            &fixture.context(),
            &fixture.tmux(true),
            Duration::ZERO,
            None,
        )
        .unwrap();

        assert!(dir.is_dir());
        assert_eq!(report.removed, 0);
        assert_eq!(report.retained.get("live"), Some(&1));
    }

    #[test]
    fn sweep_reclaims_a_stopped_session_only_after_the_grace_period() {
        let fixture = Fixture::new();
        let record = fixture.persist("sess-stopped");
        let dir = prepare_launch(&fixture.state_dir(), &record)
            .unwrap()
            .unwrap();
        let tmux = fixture.tmux(false);

        let report = sweep(&fixture.context(), &tmux, Duration::from_secs(3600), None).unwrap();
        assert!(dir.is_dir());
        assert_eq!(report.retained.get("grace"), Some(&1));

        let report = sweep(&fixture.context(), &tmux, Duration::ZERO, None).unwrap();
        assert!(!dir.exists());
        assert_eq!(report.removed, 1);
    }

    #[test]
    fn sweep_removes_orphans_and_ignores_foreign_dirs() {
        let fixture = Fixture::new();
        let orphan = fixture.record("sess-orphan", json!({}));
        let orphan_dir = prepare_launch(&fixture.state_dir(), &orphan)
            .unwrap()
            .unwrap();
        let other_state = fixture.dir.path().join("other-state");
        fs::create_dir_all(&other_state).unwrap();
        let foreign = fixture.record("sess-other-state", json!({}));
        let foreign_dir = prepare_launch(&other_state, &foreign).unwrap().unwrap();
        let vanished_state = fixture.dir.path().join("vanished-state");
        let vanished = fixture.record("sess-vanished-state", json!({}));
        let vanished_dir = prepare_launch(&vanished_state, &vanished).unwrap().unwrap();
        let unowned = fixture.tmp_dir("not-ours");
        fs::create_dir_all(&unowned).unwrap();

        let report = sweep(
            &fixture.context(),
            &fixture.tmux(false),
            Duration::ZERO,
            None,
        )
        .unwrap();

        assert!(!orphan_dir.exists());
        assert!(vanished_dir.is_dir());
        assert!(foreign_dir.is_dir());
        assert!(unowned.is_dir());
        assert_eq!(report.removed, 1);
        assert_eq!(report.retained.get("foreign"), Some(&2));
    }
}
