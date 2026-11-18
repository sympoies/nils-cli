//! Per-session `TMPDIR` (agent write-location contract, class `tmp`).
//!
//! Each managed session gets `${AGENT_HOME}/tmp/<session-id>/` (mode 0700),
//! exposed through a short symlink as `TMPDIR` at launch. Once the session stops, the directory is
//! reclaimed after a grace period; a sweep also removes directories whose
//! session record is gone. A directory is only ever touched when it carries this
//! module's owner file naming the same session and state directory, so other
//! tools' entries under `${AGENT_HOME}/tmp` and other agent-session state
//! directories are never swept.
use super::*;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, symlink};

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

// The token is published inside the private backing directory before its
// short directory is created, so a later sweep can reclaim an interrupted launch.
const SHORT_TOKEN_FILE: &str = ".agent-session-short";

fn short_root(dir: &Path) -> io::Result<Option<PathBuf>> {
    let marker = dir.join(SHORT_TOKEN_FILE);
    let metadata = match fs::symlink_metadata(&marker) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err),
    };
    if !metadata.is_file()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o777 != 0o600
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "unsafe short path marker",
        ));
    }
    let token = fs::read_to_string(marker)?;
    if token.len() != 32
        || !token
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid short path marker",
        ));
    }
    Ok(Some(Path::new("/tmp").join(format!("ark-{token}"))))
}

fn create_short_marker(dir: &Path) -> io::Result<()> {
    let token = uuid::Uuid::new_v4().simple().to_string();
    let temporary = dir.join(format!(".agent-session-short-{token}"));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temporary)?;
    let result = (|| {
        std::io::Write::write_all(&mut file, token.as_bytes())?;
        file.sync_all()?;
        match fs::hard_link(&temporary, dir.join(SHORT_TOKEN_FILE)) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => Ok(()),
            Err(err) => Err(err),
        }
    })();
    let cleanup = fs::remove_file(temporary);
    result.and(cleanup)
}

fn ensure_short_path(dir: &Path) -> Result<PathBuf, CliError> {
    if short_root(dir)
        .map_err(|err| io_error("session-tmpdir-unsafe", dir, err))?
        .is_none()
    {
        create_short_marker(dir)
            .map_err(|err| io_error("session-tmpdir-create-failed", dir, err))?;
    }
    let root = short_root(dir)
        .map_err(|err| io_error("session-tmpdir-unsafe", dir, err))?
        .ok_or_else(|| {
            io_error(
                "session-tmpdir-unsafe",
                dir,
                io::Error::other("short marker missing"),
            )
        })?;
    match fs::DirBuilder::new().mode(0o700).create(&root) {
        Ok(()) => {}
        Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {}
        Err(err) => return Err(io_error("session-tmpdir-create-failed", dir, err)),
    }
    let metadata =
        fs::symlink_metadata(&root).map_err(|err| io_error("session-tmpdir-unsafe", dir, err))?;
    if !metadata.is_dir()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o777 != 0o700
    {
        return Err(io_error(
            "session-tmpdir-unsafe",
            dir,
            io::Error::new(io::ErrorKind::PermissionDenied, "unsafe short root"),
        ));
    }
    let link = root.join("t");
    match symlink(dir, &link) {
        Ok(()) => Ok(link),
        Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {
            let valid = fs::symlink_metadata(&link).is_ok_and(|meta| {
                meta.file_type().is_symlink() && meta.uid() == unsafe { libc::geteuid() }
            }) && fs::read_link(&link).is_ok_and(|target| target == dir);
            if valid {
                Ok(link)
            } else {
                Err(io_error("session-tmpdir-unsafe", dir, err))
            }
        }
        Err(err) => Err(io_error("session-tmpdir-create-failed", dir, err)),
    }
}

fn remove_short_path(dir: &Path) -> io::Result<()> {
    let Some(root) = short_root(dir)? else {
        return Ok(());
    };
    let metadata = match fs::symlink_metadata(&root) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err),
    };
    if !metadata.is_dir()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o777 != 0o700
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "unsafe short root",
        ));
    }
    let link = root.join("t");
    match fs::symlink_metadata(&link) {
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(err) => return Err(err),
        Ok(meta)
            if meta.file_type().is_symlink()
                && meta.uid() == unsafe { libc::geteuid() }
                && fs::read_link(&link).is_ok_and(|target| target == dir) =>
        {
            fs::remove_file(link)?
        }
        Ok(_) => {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "short path is occupied",
            ));
        }
    }
    fs::remove_dir(root)
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
                sweep::reclaim_for_launch(&root, &record.id)
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
    Ok(Some(ensure_short_path(&dir)?))
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

    impl Drop for Fixture {
        fn drop(&mut self) {
            let root = self.dir.path().join("home/tmp");
            if let Ok(entries) = fs::read_dir(root) {
                for entry in entries.flatten() {
                    let _ = remove_short_path(&entry.path());
                }
            }
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
    fn managed_tmpdir_fits_chrome_socket_limits() {
        let fixture = Fixture::new();
        let long_home = fixture.dir.path().join("a".repeat(100));
        let _home = EnvGuard::set(&fixture._lock, "AGENT_HOME", long_home.to_str().unwrap());
        let record = fixture.record("01234567-89ab-cdef-0123-456789abcdef", json!({}));
        let tmp = prepare_launch(&fixture.state_dir(), &record)
            .unwrap()
            .unwrap();
        let socket = tmp.join("com.google.Chrome.XXXXXX/SingletonSocket");
        let bytes = socket.as_os_str().as_encoded_bytes().len();
        assert!(bytes < 104);
        assert!(bytes < 108);
        remove_short_path(&long_home.join("tmp").join(&record.id)).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn foreign_precreated_predictable_root_does_not_block_launch() {
        const INNER: &str = "NILS_TEST_FOREIGN_SHORT_ROOT_INNER";
        if std::env::var_os(INNER).is_some() {
            let fixture = Fixture::new();
            let record = fixture.record("foreign-preclaim", json!({}));
            let old_name = Path::new("/tmp").join(format!("ark-{}", unsafe { libc::geteuid() }));
            assert!(fs::symlink_metadata(&old_name).unwrap().uid() != unsafe { libc::geteuid() });
            let link = prepare_launch(&fixture.state_dir(), &record)
                .unwrap()
                .unwrap();
            assert_eq!(
                fs::read_link(&link).unwrap(),
                fixture.tmp_dir("foreign-preclaim")
            );
            assert!(old_name.is_dir());
            return;
        }
        let uid = unsafe { libc::geteuid() };
        let gid = unsafe { libc::getegid() };
        if uid == 0 || uid == 65534 {
            eprintln!("skipped: fixture requires an ordinary user");
            return;
        }
        let available = ProcessCommand::new("sudo")
            .args([
                "-n",
                "unshare",
                "--mount",
                "--propagation",
                "private",
                "true",
            ])
            .status();
        if !available.is_ok_and(|status| status.success()) {
            eprintln!("skipped: private privileged mount namespace unavailable");
            return;
        }
        let script = r#"mount -t tmpfs -o mode=1777 tmpfs /tmp
setpriv --reuid=65534 --regid=65534 --clear-groups mkdir "/tmp/ark-$1"
exec setpriv --reuid="$1" --regid="$2" --clear-groups env TMPDIR=/tmp NILS_TEST_FOREIGN_SHORT_ROOT_INNER=1 "$3" runtime_environment::session_tmp::tests::foreign_precreated_predictable_root_does_not_block_launch --exact --nocapture"#;
        let output = ProcessCommand::new("sudo")
            .args([
                "-n",
                "unshare",
                "--mount",
                "--propagation",
                "private",
                "sh",
                "-eu",
                "-c",
                script,
                "fixture",
            ])
            .arg(uid.to_string())
            .arg(gid.to_string())
            .arg(std::env::current_exe().unwrap())
            .output()
            .unwrap();
        assert!(output.status.success(), "isolated cross-UID test failed");
    }

    #[test]
    fn prepare_launch_creates_a_private_dir_under_agent_home() {
        let fixture = Fixture::new();
        let record = fixture.record("sess-create", json!({}));

        let dir = prepare_launch(&fixture.state_dir(), &record).unwrap();

        assert_eq!(
            fs::read_link(dir.as_ref().unwrap()).unwrap(),
            fixture.tmp_dir("sess-create")
        );
        let mode = fs::metadata(fixture.tmp_dir("sess-create"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o700);
        assert_eq!(
            fs::metadata(dir.as_ref().unwrap().parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert!(fixture.tmp_dir("sess-create").join(OWNER_FILE).is_file());
    }

    #[test]
    fn short_paths_isolate_sessions_and_refuse_occupied_paths() {
        let fixture = Fixture::new();
        let first = fixture.record("sess-first", json!({}));
        let second = fixture.record("sess-second", json!({}));
        let first_dir = prepare_launch(&fixture.state_dir(), &first)
            .unwrap()
            .unwrap();
        let second_dir = prepare_launch(&fixture.state_dir(), &second)
            .unwrap()
            .unwrap();
        assert_ne!(first_dir, second_dir);
        fs::write(first_dir.join("private"), "first").unwrap();
        assert!(!second_dir.join("private").exists());

        struct RestoreLink(PathBuf, PathBuf);
        impl Drop for RestoreLink {
            fn drop(&mut self) {
                let _ = fs::remove_file(&self.0);
                let _ = symlink(&self.1, &self.0);
            }
        }
        struct RestoreMode(PathBuf);
        impl Drop for RestoreMode {
            fn drop(&mut self) {
                let _ = fs::set_permissions(&self.0, fs::Permissions::from_mode(0o700));
            }
        }
        let restore_link = RestoreLink(second_dir.clone(), fixture.tmp_dir("sess-second"));
        fs::remove_file(&second_dir).unwrap();
        fs::write(&second_dir, "foreign").unwrap();
        assert!(ensure_short_path(&fixture.tmp_dir("sess-second")).is_err());
        assert_eq!(fs::read_to_string(&second_dir).unwrap(), "foreign");
        fs::remove_file(&second_dir).unwrap();
        let wrong_target = fixture.dir.path().join("wrong-target");
        symlink(&wrong_target, &second_dir).unwrap();
        assert!(ensure_short_path(&fixture.tmp_dir("sess-second")).is_err());
        assert_eq!(fs::read_link(&second_dir).unwrap(), wrong_target);
        drop(restore_link);

        let root = first_dir.parent().unwrap();
        let restore_mode = RestoreMode(root.to_path_buf());
        fs::set_permissions(root, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(ensure_short_path(&fixture.tmp_dir("sess-first")).is_err());
        assert_eq!(
            fs::metadata(root).unwrap().permissions().mode() & 0o777,
            0o755
        );
        drop(restore_mode);
        assert!(first_dir.join("private").is_file());
    }

    #[test]
    fn concurrent_launches_share_one_private_short_path() {
        let fixture = Fixture::new();
        let record = fixture.record("concurrent", json!({}));
        let state = fixture.state_dir();
        let barrier = std::sync::Barrier::new(16);
        let links = std::thread::scope(|scope| {
            let mut handles = Vec::new();
            for _ in 0..16 {
                let barrier = &barrier;
                let state = &state;
                let record = &record;
                handles.push(scope.spawn(move || {
                    barrier.wait();
                    prepare_launch(state, record).unwrap().unwrap()
                }));
            }
            handles
                .into_iter()
                .map(|handle| handle.join().unwrap())
                .collect::<Vec<_>>()
        });
        assert!(links.iter().all(|link| link == &links[0]));
        assert_eq!(
            fs::read_link(&links[0]).unwrap(),
            fixture.tmp_dir("concurrent")
        );
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
        let expected = format!(
            "TMPDIR={}",
            short_root(&fixture.tmp_dir("sess-inject"))
                .unwrap()
                .unwrap()
                .join("t")
                .display()
        );
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
        let replacement = prepare_launch(&fixture.state_dir(), &record)
            .unwrap()
            .unwrap();
        assert!(!dir.exists());
        assert!(!replacement.join("scratch.txt").exists());
        assert!(replacement.join(OWNER_FILE).is_file());
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
        assert!(fixture.tmp_dir("sess-live").is_dir());
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
        assert!(fixture.tmp_dir("sess-stopped").is_dir());
        assert_eq!(report.retained.get("grace"), Some(&1));

        let report = sweep(&fixture.context(), &tmux, Duration::ZERO, None).unwrap();
        assert!(fs::symlink_metadata(&dir).is_err());
        assert!(!fixture.tmp_dir("sess-stopped").exists());
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

        assert!(fs::symlink_metadata(&orphan_dir).is_err());
        assert!(!fixture.tmp_dir("sess-orphan").exists());
        assert!(vanished_dir.is_dir());
        assert!(fixture.tmp_dir("sess-vanished-state").is_dir());
        assert!(foreign_dir.is_dir());
        assert!(fixture.tmp_dir("sess-other-state").is_dir());
        assert!(unowned.is_dir());
        assert_eq!(report.removed, 1);
        assert_eq!(report.retained.get("foreign"), Some(&2));
    }
}
