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

#[derive(Debug, Serialize)]
pub(crate) struct SweepEntry {
    pub(crate) id: String,
    pub(crate) reason: &'static str,
}

#[derive(Debug, Serialize)]
pub(crate) struct SweepReport {
    pub(crate) root: String,
    pub(crate) grace_secs: u64,
    pub(crate) removed: Vec<SweepEntry>,
    pub(crate) retained: Vec<SweepEntry>,
}

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
    // tmp directories out of the developer's real AGENT_HOME.
    #[cfg(test)]
    if non_empty_env("AGENT_HOME").is_none() {
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
    let path = dir.join(OWNER_FILE);
    if !fs::symlink_metadata(&path).ok()?.is_file() {
        return None;
    }
    let owner: Owner = serde_json::from_slice(&fs::read(&path).ok()?).ok()?;
    (owner.schema_version == OWNER_SCHEMA && owner.session_id == id).then_some(owner)
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
    now_epoch().saturating_sub(stopped_at) >= grace.as_secs()
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

/// Remove stopped-session directories past `grace`, and directories of deleted
/// session records. A session that is not provably stopped, or whose record is
/// locked, keeps its directory. `budget` bounds the walk for callers on a
/// latency-sensitive path.
pub(crate) fn sweep(
    context: &CliContext,
    tmux_bin: &Path,
    grace: Duration,
    budget: Option<Duration>,
) -> Result<SweepReport, CliError> {
    let root = tmp_root(&context.state_dir)?;
    let mut report = SweepReport {
        root: display_path(&root),
        grace_secs: grace.as_secs(),
        removed: Vec::new(),
        retained: Vec::new(),
    };
    let entries = match fs::read_dir(&root) {
        Ok(entries) => entries,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(report),
        Err(err) => return Err(io_error("session-tmpdir-list-failed", &root, err)),
    };
    let mut ids: Vec<String> = entries
        .flatten()
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| validate_id(name).is_ok())
        .collect();
    ids.sort();
    let started = Instant::now();
    for id in ids {
        if budget.is_some_and(|budget| started.elapsed() >= budget) {
            break;
        }
        let dir = root.join(&id);
        let Some(mut owner) = read_owner(&dir, &id) else {
            continue;
        };
        // Another live state directory owns its sessions' dirs. A state directory
        // that no longer exists leaves nothing to reclaim them, so they are orphans.
        let ours = owner.state_dir == display_path(&context.state_dir);
        if !ours && Path::new(&owner.state_dir).is_dir() {
            continue;
        }
        let record_gone = !ours || !session_dir(context, &id).join("session.json").exists();
        let (remove, reason) = if record_gone {
            let age = fs::metadata(&dir)
                .and_then(|meta| meta.modified())
                .ok()
                .and_then(|modified| modified.elapsed().ok())
                .unwrap_or_default();
            (age >= grace, "orphan")
        } else {
            let Ok(Some(_lock)) = try_acquire_session_record_lock(context, &id) else {
                report.retained.push(SweepEntry { id, reason: "busy" });
                continue;
            };
            let Ok(record) = load_session_record(context, &id) else {
                report.retained.push(SweepEntry {
                    id,
                    reason: "unreadable",
                });
                continue;
            };
            match session_status(context, tmux_bin, &record).as_str() {
                "stopped" => {
                    let stopped_at = *owner.stopped_at_epoch.get_or_insert_with(now_epoch);
                    let due = expired(stopped_at, grace);
                    if !due {
                        let _ = write_owner(&dir, &owner);
                    }
                    (due, "expired")
                }
                "running" => {
                    if owner.stopped_at_epoch.take().is_some() {
                        let _ = write_owner(&dir, &owner);
                    }
                    report.retained.push(SweepEntry { id, reason: "live" });
                    continue;
                }
                _ => {
                    report.retained.push(SweepEntry {
                        id,
                        reason: "unknown",
                    });
                    continue;
                }
            }
        };
        if !remove {
            report.retained.push(SweepEntry {
                id,
                reason: "grace",
            });
        } else if fs::remove_dir_all(&dir).is_ok() {
            report.removed.push(SweepEntry { id, reason });
        } else {
            report.retained.push(SweepEntry {
                id,
                reason: "remove-failed",
            });
        }
    }
    Ok(report)
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

fn render_sweep_text(report: &SweepReport) -> String {
    let mut text = format!(
        "root: {}\ngrace_secs: {}\nremoved: {}\n",
        report.root,
        report.grace_secs,
        report.removed.len()
    );
    for entry in &report.removed {
        text.push_str(&format!("  {} ({})\n", entry.id, entry.reason));
    }
    text.push_str(&format!("retained: {}\n", report.retained.len()));
    for entry in &report.retained {
        text.push_str(&format!("  {} ({})\n", entry.id, entry.reason));
    }
    text
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

    fn ids(entries: &[SweepEntry]) -> Vec<(&str, &str)> {
        entries
            .iter()
            .map(|entry| (entry.id.as_str(), entry.reason))
            .collect()
    }

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
        assert!(report.removed.is_empty());
        assert_eq!(ids(&report.retained), vec![("sess-live", "live")]);
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
        assert_eq!(ids(&report.retained), vec![("sess-stopped", "grace")]);

        let report = sweep(&fixture.context(), &tmux, Duration::ZERO, None).unwrap();
        assert!(!dir.exists());
        assert_eq!(ids(&report.removed), vec![("sess-stopped", "expired")]);
        assert_eq!(
            report.root,
            fixture.dir.path().join("home/tmp").display().to_string()
        );
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
        assert!(!vanished_dir.exists());
        assert!(foreign_dir.is_dir());
        assert!(unowned.is_dir());
        assert_eq!(
            ids(&report.removed),
            vec![("sess-orphan", "orphan"), ("sess-vanished-state", "orphan")]
        );
        assert!(report.retained.is_empty());
    }
}
