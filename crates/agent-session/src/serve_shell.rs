//! Emergency shells are separate from provider sessions and share only the
//! terminal wire protocol. The edge is the principal authority; every route
//! here requires the machine credential, including reads.
use super::*;
use axum::extract::Query;
use std::os::fd::AsRawFd;

const TIMEOUT: Duration = Duration::from_secs(2);
const INCARNATION_ENV: &str = "AC_SHELL_INCARNATION";

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(super) struct ShellRecord {
    owner: String,
    incarnation: String,
    tmux_name: String,
    session_id: String,
    pane_id: String,
}

#[derive(Deserialize)]
pub(super) struct Fence {
    incarnation: String,
}

fn error(code: &str, message: &str) -> CliError {
    CliError::runtime(code, message, None)
}

fn owner_name(owner: &str) -> Result<String, CliError> {
    if owner.is_empty()
        || owner.len() > 64
        || !owner.bytes().enumerate().all(|(i, b)| {
            b.is_ascii_lowercase()
                || b.is_ascii_digit()
                || (i > 0 && matches!(b, b'.' | b'_' | b'-'))
        })
    {
        return Err(CliError::data(
            "shell-owner-invalid",
            "invalid shell owner",
            None,
        ));
    }
    let mut name = String::from("ac-shell-");
    for b in owner.bytes() {
        if matches!(b, b'.' | b'_') {
            name.push_str(&format!("_{b:02x}"));
        } else {
            name.push(char::from(b));
        }
    }
    Ok(name)
}

pub(super) fn context(state: &ServeState) -> CliContext {
    CliContext {
        state_dir: state.context.state_dir.join("emergency-shell"),
        host: state.context.host.clone(),
    }
}

// Held through creation, close, input and resize, including across processes.
fn lock(context: &CliContext, owner: &str) -> Result<std::fs::File, CliError> {
    owner_name(owner)?;
    fs::create_dir_all(&context.state_dir)
        .map_err(|_| error("shell-state-unavailable", "shell state is unavailable"))?;
    fs::set_permissions(&context.state_dir, fs::Permissions::from_mode(0o700))
        .map_err(|_| error("shell-state-unavailable", "shell state is unavailable"))?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(context.state_dir.join(format!("{owner}.lock")))
        .map_err(|_| error("shell-state-unavailable", "shell lock is unavailable"))?;
    // SAFETY: file owns a valid descriptor and flock does not access pointers.
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } != 0 {
        return Err(error("shell-lock-unavailable", "shell lock is unavailable"));
    }
    Ok(file)
}

fn load(context: &CliContext, owner: &str) -> Result<Option<ShellRecord>, CliError> {
    match fs::read(context.state_dir.join(format!("{owner}.json"))) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|_| error("shell-state-invalid", "shell state is invalid")),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(error(
            "shell-state-unavailable",
            "shell state is unavailable",
        )),
    }
}

fn save(context: &CliContext, record: &ShellRecord) -> Result<(), CliError> {
    crate::write_private_file(
        &context.state_dir.join(format!("{}.json", record.owner)),
        &serde_json::to_vec(record)
            .map_err(|_| error("shell-state-invalid", "shell state is invalid"))?,
    )
}

fn command(tmux: &Path, args: &[&str]) -> Result<std::process::Output, CliError> {
    let mut command = ProcessCommand::new(tmux);
    command.args(args);
    crate::run_output_with_timeout_and_cap(command, TIMEOUT, 16 * 1024).map_err(|_| {
        error(
            "shell-tmux-unavailable",
            "shell tmux command is unavailable",
        )
    })
}

fn checked(tmux: &Path, args: &[&str]) -> Result<(), CliError> {
    if command(tmux, args)?.status.success() {
        Ok(())
    } else {
        Err(error("shell-tmux-failed", "shell tmux command failed"))
    }
}

fn probe(tmux: &Path, name: &str) -> Result<Option<(String, String, String)>, CliError> {
    let target = format!("={name}:");
    let output = command(
        tmux,
        &[
            "display-message",
            "-p",
            "-t",
            &target,
            "#{session_id}\t#{pane_id}\t#{AC_SHELL_INCARNATION}\t#{pane_dead}",
        ],
    )?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("can't find")
            || stderr.contains("no server running")
            || stderr.contains("No such file or directory")
        {
            return Ok(None);
        }
        return Err(error(
            "shell-tmux-unavailable",
            "shell tmux state could not be checked",
        ));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let parts: Vec<_> = text.trim_end_matches('\n').split('\t').collect();
    if parts.iter().all(|part| part.is_empty()) {
        return Ok(None);
    }
    if parts.len() != 4 || !parts[0].starts_with('$') || !parts[1].starts_with('%') {
        return Err(error(
            "shell-tmux-invalid",
            "shell tmux identity is invalid",
        ));
    }
    if parts[3] == "1" {
        return Ok(None);
    }
    Ok(Some((parts[0].into(), parts[1].into(), parts[2].into())))
}

fn current(
    context: &CliContext,
    tmux: &Path,
    owner: &str,
) -> Result<Option<ShellRecord>, CliError> {
    let name = owner_name(owner)?;
    let record = load(context, owner)?;
    let Some((session_id, pane_id, incarnation)) = probe(tmux, &name)? else {
        return Ok(None);
    };
    let Some(mut record) = record else {
        return Err(error(
            "shell-name-conflict",
            "fixed shell name is already in use",
        ));
    };
    if record.owner != owner || record.tmux_name != name || record.incarnation != incarnation {
        return Err(error(
            "shell-name-conflict",
            "fixed shell identity does not match its owner",
        ));
    }
    // Recover an interrupted create only when the tmux marker proves identity.
    if record.session_id.is_empty() {
        record.session_id = session_id;
        record.pane_id = pane_id;
        save(context, &record)?;
    } else if record.session_id != session_id || record.pane_id != pane_id {
        return Err(error(
            "shell-runtime-changed",
            "shell runtime identity changed",
        ));
    }
    Ok(Some(record))
}

fn projection(owner: &str, record: Option<&ShellRecord>) -> Value {
    json!({"schema_version":"agent-session.shell.v1", "owner":owner,
        "status":if record.is_some() { "running" } else { "stopped" },
        "incarnation":record.map(|r| &r.incarnation), "tmux_name":owner_name(owner).ok()})
}

fn ensure(
    context: &CliContext,
    tmux: &Path,
    owner: &str,
    home: &Path,
) -> Result<ShellRecord, CliError> {
    let _lock = lock(context, owner)?;
    if let Some(record) = current(context, tmux, owner)? {
        return Ok(record);
    }
    if let Some(old) = load(context, owner)? {
        let _ = fs::remove_dir_all(session_dir(context, &old.incarnation));
    }
    let zsh = ProcessCommand::new("zsh")
        .args(["-f", "-c", "exit 0"])
        .status()
        .map_err(|_| {
            error(
                "shell-zsh-unavailable",
                "zsh is unavailable on this machine",
            )
        })?;
    if !zsh.success() {
        return Err(error(
            "shell-zsh-unavailable",
            "zsh is unavailable on this machine",
        ));
    }
    let mut record = ShellRecord {
        owner: owner.into(),
        incarnation: uuid::Uuid::new_v4().to_string(),
        tmux_name: owner_name(owner)?,
        session_id: String::new(),
        pane_id: String::new(),
    };
    save(context, &record)?;
    let target = format!("={}:", record.tmux_name);
    let cwd = home
        .to_str()
        .ok_or_else(|| error("shell-home-unavailable", "host home is unavailable"))?;
    checked(
        tmux,
        &[
            "new-session",
            "-d",
            "-s",
            &record.tmux_name,
            "-c",
            cwd,
            "-e",
            &format!("HOME={cwd}"),
            "-e",
            &format!("{INCARNATION_ENV}={}", record.incarnation),
            "zsh",
            "-il",
            ";",
            "set-option",
            "-t",
            &target,
            "remain-on-exit",
            "off",
        ],
    )?;
    let Some((session, pane, incarnation)) = probe(tmux, &record.tmux_name)? else {
        return Err(error("shell-start-failed", "zsh exited during startup"));
    };
    if incarnation != record.incarnation {
        return Err(error("shell-name-conflict", "fixed shell identity changed"));
    }
    record.session_id = session;
    record.pane_id = pane;
    save(context, &record)?;
    Ok(record)
}

fn fenced(
    context: &CliContext,
    tmux: &Path,
    expected: &ShellRecord,
) -> Result<ShellRecord, CliError> {
    let record = current(context, tmux, &expected.owner)?
        .ok_or_else(|| error("shell-not-running", "shell is stopped"))?;
    if record.incarnation != expected.incarnation || record.pane_id != expected.pane_id {
        return Err(CliError::data(
            "shell-incarnation-conflict",
            "shell runtime has changed",
            None,
        ));
    }
    Ok(record)
}

pub(super) async fn status(
    State(state): State<Arc<ServeState>>,
    headers: HeaderMap,
    AxPath(owner): AxPath<String>,
) -> Response {
    if let Some(response) = deny_unauthorized(&state, &headers) {
        return response;
    }
    let context = context(&state);
    let tmux = state.tmux_bin.clone();
    match tokio::task::spawn_blocking(move || {
        let _lock = lock(&context, &owner)?;
        Ok::<_, CliError>(projection(
            &owner,
            current(&context, &tmux, &owner)?.as_ref(),
        ))
    })
    .await
    {
        Ok(Ok(shell)) => envelope_ok(json!({"shell":shell})),
        Ok(Err(e)) => envelope_err(e),
        Err(_) => join_err(),
    }
}

pub(super) async fn open(
    State(state): State<Arc<ServeState>>,
    headers: HeaderMap,
    AxPath(owner): AxPath<String>,
) -> Response {
    if let Some(response) = deny_unauthorized(&state, &headers) {
        return response;
    }
    let context = context(&state);
    let tmux = state.tmux_bin.clone();
    match tokio::task::spawn_blocking(move || {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or_else(|| error("shell-home-unavailable", "host home is unavailable"))?;
        let record = ensure(&context, &tmux, &owner, &home)?;
        Ok::<_, CliError>(projection(&owner, Some(&record)))
    })
    .await
    {
        Ok(Ok(shell)) => envelope_ok(json!({"shell":shell})),
        Ok(Err(e)) => envelope_err(e),
        Err(_) => join_err(),
    }
}

pub(super) async fn close(
    State(state): State<Arc<ServeState>>,
    headers: HeaderMap,
    AxPath(owner): AxPath<String>,
    body: Result<Json<Fence>, JsonRejection>,
) -> Response {
    if let Some(response) = deny_unauthorized(&state, &headers) {
        return response;
    }
    let Ok(Json(fence)) = body else {
        return envelope_err(CliError::data(
            "shell-fence-required",
            "shell incarnation is required",
            None,
        ));
    };
    let context = context(&state);
    let tmux = state.tmux_bin.clone();
    match tokio::task::spawn_blocking(move || {
        let _lock = lock(&context, &owner)?;
        if let Some(record) = current(&context, &tmux, &owner)? {
            if record.incarnation != fence.incarnation {
                return Err(CliError::data(
                    "shell-incarnation-conflict",
                    "shell runtime has changed",
                    None,
                ));
            }
            guarded(&tmux, &record, &["kill-session", "-t", &record.session_id])?;
            let _ = fs::remove_dir_all(session_dir(&context, &record.incarnation));
        }
        Ok::<_, CliError>(projection(&owner, None))
    })
    .await
    {
        Ok(Ok(shell)) => envelope_ok(json!({"shell":shell})),
        Ok(Err(e)) => envelope_err(e),
        Err(_) => join_err(),
    }
}

pub(super) async fn attach(
    State(state): State<Arc<ServeState>>,
    headers: HeaderMap,
    AxPath(owner): AxPath<String>,
    query: Result<Query<Fence>, QueryRejection>,
    ws: Result<WebSocketUpgrade, axum::extract::ws::rejection::WebSocketUpgradeRejection>,
) -> Response {
    if let Some(response) = deny_unauthorized(&state, &headers) {
        return response;
    }
    let Ok(Query(fence)) = query else {
        return envelope_err(CliError::data(
            "shell-fence-required",
            "shell incarnation is required",
            None,
        ));
    };
    let ws = match ws {
        Ok(ws) => ws,
        Err(rejection) => return rejection.into_response(),
    };
    let ctx = context(&state);
    let tmux = state.tmux_bin.clone();
    let lookup = tokio::task::spawn_blocking(move || {
        let _lock = lock(&ctx, &owner)?;
        let record = current(&ctx, &tmux, &owner)?
            .ok_or_else(|| error("shell-not-running", "shell is stopped"))?;
        if record.incarnation != fence.incarnation {
            return Err(CliError::data(
                "shell-incarnation-conflict",
                "shell runtime has changed",
                None,
            ));
        }
        let path = session_dir(&ctx, &record.incarnation);
        fs::create_dir_all(&path).map_err(|_| {
            error(
                "shell-state-unavailable",
                "shell terminal state is unavailable",
            )
        })?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).map_err(|_| {
            error(
                "shell-state-unavailable",
                "shell terminal state is unavailable",
            )
        })?;
        Ok::<_, CliError>(record)
    })
    .await;
    match lookup {
        Ok(Ok(shell)) => {
            let mut record = terminal_record(&shell);
            record
                .extra
                .insert("shell_state_dir".into(), json!(context(&state).state_dir));
            ws.on_upgrade(move |socket| attach_socket_inner(socket, state, record, Some(shell)))
        }
        Ok(Err(e)) => envelope_err(e),
        Err(_) => join_err(),
    }
}

fn terminal_record(shell: &ShellRecord) -> crate::SessionRecord {
    crate::SessionRecord {
        schema_version: crate::SESSION_DOCUMENT_VERSION.into(),
        id: shell.incarnation.clone(),
        agent: "shell".into(),
        mode: "interactive".into(),
        coordination_mode: cli::CoordinationMode::Advisory,
        title: None,
        title_state: None,
        title_revision: 0,
        cwd: String::new(),
        tmux_session: shell.pane_id.clone(),
        prompt_file: None,
        log_file: None,
        created_at: String::new(),
        updated_at: String::new(),
        provider_resume: None,
        runtime: None,
        public_metadata: None,
        agent_args: Vec::new(),
        agent_bin: None,
        extra: BTreeMap::from([(
            "emergency_shell".into(),
            serde_json::to_value(shell).expect("serializable shell identity"),
        )]),
        resume_sidecar_extra: BTreeMap::new(),
    }
}

pub(super) fn terminal_context(record: &crate::SessionRecord) -> Result<CliContext, CliError> {
    Ok(CliContext {
        state_dir: record
            .extra
            .get("shell_state_dir")
            .and_then(Value::as_str)
            .map(PathBuf::from)
            .ok_or_else(|| error("shell-state-invalid", "shell terminal state is missing"))?,
        host: None,
    })
}

// The owner lock serializes Console lifecycle commands. A tmux format guard
// also protects against server restarts that reuse numeric pane identifiers.
pub(super) fn terminal_command(
    context: &CliContext,
    tmux: &Path,
    record: &crate::SessionRecord,
    args: &[&str],
) -> Result<std::process::Output, CliError> {
    let expected: ShellRecord = serde_json::from_value(
        record
            .extra
            .get("emergency_shell")
            .cloned()
            .ok_or_else(|| error("shell-state-invalid", "shell identity is missing"))?,
    )
    .map_err(|_| error("shell-state-invalid", "shell identity is invalid"))?;
    let _lock = lock(context, &expected.owner)?;
    fenced(context, tmux, &expected)?;
    guarded(tmux, &expected, args)
}

fn guarded(
    tmux: &Path,
    expected: &ShellRecord,
    args: &[&str],
) -> Result<std::process::Output, CliError> {
    let target = format!("={}:", expected.tmux_name);
    let condition = format!("#{{==:#{{AC_SHELL_INCARNATION}},{}}}", expected.incarnation);
    let action = args
        .iter()
        .map(|arg| shell_words::quote(arg))
        .collect::<Vec<_>>()
        .join(" ");
    let output = command(
        tmux,
        &[
            "if-shell",
            "-F",
            "-t",
            &target,
            &condition,
            &action,
            "display-message -p shell-runtime-changed",
        ],
    )?;
    if !output.status.success() || output.stdout == b"shell-runtime-changed\n" {
        return Err(error(
            "shell-incarnation-conflict",
            "shell runtime has changed",
        ));
    }
    Ok(output)
}

pub(super) async fn input(
    state: Arc<ServeState>,
    expected: ShellRecord,
    frame: String,
) -> Result<(), CliError> {
    let context = context(&state);
    let tmux = state.tmux_bin.clone();
    tokio::task::spawn_blocking(move || {
        let _lock = lock(&context, &expected.owner)?;
        let record = fenced(&context, &tmux, &expected)?;
        let value: Value = serde_json::from_str(&frame)
            .map_err(|_| CliError::data("shell-input-invalid", "invalid terminal input", None))?;
        if let Some(resize) = value.get("resize") {
            let cols = resize
                .get("cols")
                .and_then(Value::as_u64)
                .filter(|v| (1..=1000).contains(v));
            let rows = resize
                .get("rows")
                .and_then(Value::as_u64)
                .filter(|v| (1..=1000).contains(v));
            if let (Some(cols), Some(rows)) = (cols, rows) {
                guarded(
                    &tmux,
                    &record,
                    &[
                        "resize-window",
                        "-t",
                        &record.pane_id,
                        "-x",
                        &cols.to_string(),
                        "-y",
                        &rows.to_string(),
                    ],
                )?;
            }
            return Ok(());
        }
        if let Some(text) = value.get("text").and_then(Value::as_str) {
            if text.contains('\0') {
                return Err(CliError::data(
                    "shell-input-invalid",
                    "terminal input contains NUL",
                    None,
                ));
            }
            guarded(
                &tmux,
                &record,
                &["send-keys", "-t", &record.pane_id, "-l", "--", text],
            )?;
        }
        let keys = value
            .get("keys")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .chain(value.get("key").and_then(Value::as_str));
        for key in keys.take(MAX_SEND_KEYS) {
            if let Some(key) = SpecialKey::from_name(key) {
                guarded(
                    &tmux,
                    &record,
                    &["send-keys", "-t", &record.pane_id, key.tmux_key()],
                )?;
            }
        }
        Ok(())
    })
    .await
    .map_err(|_| error("shell-input-failed", "shell input worker failed"))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::{assert_eq, assert_ne};

    struct Fixture {
        tmp: tempfile::TempDir,
        tmux: PathBuf,
        context: CliContext,
    }
    impl Fixture {
        fn new() -> Self {
            let tmp = tempfile::Builder::new()
                .prefix("shell-")
                .tempdir_in("/tmp")
                .unwrap();
            let wrapper = tmp.path().join("tmux");
            let socket = tmp.path().join("tmux.sock");
            let zdotdir = tmp.path().join("zsh");
            fs::create_dir(&zdotdir).unwrap();
            // Keep host first-run and completion prompts out of this fixture.
            fs::write(zdotdir.join(".zshenv"), "unsetopt GLOBAL_RCS\n").unwrap();
            fs::write(zdotdir.join(".zshrc"), "").unwrap();
            fs::write(
                &wrapper,
                format!(
                    "#!/bin/sh\nexport ZDOTDIR={}\nexec tmux -S {} -f /dev/null \"$@\"\n",
                    shell_words::quote(zdotdir.to_str().unwrap()),
                    shell_words::quote(socket.to_str().unwrap())
                ),
            )
            .unwrap();
            fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o700)).unwrap();
            let context = CliContext {
                state_dir: tmp.path().join("state"),
                host: None,
            };
            Self {
                tmp,
                tmux: wrapper,
                context,
            }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = command(&self.tmux, &["kill-server"]);
        }
    }

    #[test]
    fn emergency_shell_fixed_names_are_collision_safe() {
        assert_eq!(owner_name("alice").unwrap(), "ac-shell-alice");
        assert_ne!(owner_name("a.b").unwrap(), owner_name("a_2eb").unwrap());
        for owner in ["../bad", "Bad", "a:b", "", "-bad"] {
            assert!(owner_name(owner).is_err());
        }
    }

    #[test]
    fn emergency_shell_reuses_runtime_and_keeps_owners_separate() {
        let f = Fixture::new();
        let a = ensure(&f.context, &f.tmux, "alice", f.tmp.path()).unwrap();
        let again = ensure(&f.context, &f.tmux, "alice", f.tmp.path()).unwrap();
        assert_eq!(a.incarnation, again.incarnation);
        assert_eq!(a.pane_id, again.pane_id);
        let b = ensure(&f.context, &f.tmux, "bob", f.tmp.path()).unwrap();
        assert_ne!(a.pane_id, b.pane_id);
        assert_ne!(a.tmux_name, b.tmux_name);
        assert_eq!(
            String::from_utf8(
                command(&f.tmux, &["list-sessions", "-F", "#{session_name}"])
                    .unwrap()
                    .stdout
            )
            .unwrap()
            .lines()
            .count(),
            2
        );
    }

    #[test]
    fn emergency_shell_concurrent_open_is_singleton() {
        let f = Fixture::new();
        let records = std::thread::scope(|scope| {
            let jobs: Vec<_> = (0..4)
                .map(|_| {
                    scope.spawn(|| ensure(&f.context, &f.tmux, "alice", f.tmp.path()).unwrap())
                })
                .collect();
            jobs.into_iter()
                .map(|j| j.join().unwrap())
                .collect::<Vec<_>>()
        });
        assert!(records.iter().all(|r| r.incarnation == records[0].incarnation && r.pane_id == records[0].pane_id));
    }

    #[test]
    fn emergency_shell_recreate_rejects_stale_runtime() {
        let f = Fixture::new();
        let old = ensure(&f.context, &f.tmux, "alice", f.tmp.path()).unwrap();
        checked(&f.tmux, &["kill-session", "-t", &old.session_id]).unwrap();
        let new = ensure(&f.context, &f.tmux, "alice", f.tmp.path()).unwrap();
        assert_ne!(old.incarnation, new.incarnation);
        assert_eq!(
            fenced(&f.context, &f.tmux, &old).unwrap_err().code(),
            "shell-incarnation-conflict"
        );
        assert_eq!(
            current(&f.context, &f.tmux, "alice")
                .unwrap()
                .unwrap()
                .incarnation,
            new.incarnation
        );
    }

    #[test]
    fn emergency_shell_recovers_interrupted_create_without_an_extra_session() {
        let f = Fixture::new();
        let record = ShellRecord {
            owner: "alice".into(),
            incarnation: uuid::Uuid::new_v4().to_string(),
            tmux_name: owner_name("alice").unwrap(),
            session_id: String::new(),
            pane_id: String::new(),
        };
        let _lock = lock(&f.context, "alice").unwrap();
        save(&f.context, &record).unwrap();
        drop(_lock);
        checked(
            &f.tmux,
            &[
                "new-session",
                "-d",
                "-s",
                &record.tmux_name,
                "-e",
                &format!("AC_SHELL_INCARNATION={}", record.incarnation),
                "zsh",
                "-f",
            ],
        )
        .unwrap();
        let recovered = ensure(&f.context, &f.tmux, "alice", f.tmp.path()).unwrap();
        assert_eq!(recovered.incarnation, record.incarnation);
        assert!(!recovered.pane_id.is_empty());
        assert_eq!(
            command(&f.tmux, &["list-sessions", "-F", "#{session_name}"])
                .unwrap()
                .stdout,
            b"ac-shell-alice\n"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn emergency_shell_public_lifecycle_and_terminal_roundtrip() {
        use tokio_tungstenite::tungstenite::{Message as Frame, client::IntoClientRequest};
        let f = Fixture::new();
        let state = super::super::tests::state(
            &f.context.state_dir,
            Some("shell-test-token"),
            f.tmux.clone(),
        );
        let context = super::context(&state);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app = router(state.clone());
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let client = reqwest::Client::new();
        let url = format!("http://{addr}/shells/alice");
        let stopped: Value = client
            .get(&url)
            .bearer_auth("shell-test-token")
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(stopped["data"]["shell"]["status"], "stopped");
        let created: Value = client
            .post(&url)
            .bearer_auth("shell-test-token")
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let incarnation = created["data"]["shell"]["incarnation"].as_str().unwrap();
        assert!(!state.context.state_dir.join("sessions").exists());
        let ws_url = format!("ws://{addr}/shells/alice/attach?incarnation={incarnation}");
        let connect = || {
            let mut request = ws_url.clone().into_client_request().unwrap();
            request
                .headers_mut()
                .insert("authorization", "Bearer shell-test-token".parse().unwrap());
            request
        };
        let (mut socket, _) = tokio_tungstenite::connect_async(connect()).await.unwrap();
        socket
            .send(Frame::Text(
                json!({"resize":{"cols":90,"rows":25}}).to_string().into(),
            ))
            .await
            .unwrap();
        // Set shell-local state, then require output that cannot come from the
        // echoed command: the token is assembled only when zsh executes it.
        socket
            .send(Frame::Text(
                json!({"text":"export AC_SHELL_TEST=kept; printf 'round%s\n' trip"})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
        socket
            .send(Frame::Text(json!({"key":"enter"}).to_string().into()))
            .await
            .unwrap();
        let mut output = String::new();
        tokio::time::timeout(Duration::from_secs(5), async {
            while !output.contains("roundtrip") {
                if let Some(Ok(Frame::Binary(bytes))) = socket.next().await {
                    output.push_str(&String::from_utf8_lossy(&bytes));
                }
            }
        })
        .await
        .expect("shell must execute input and return output");
        socket.close(None).await.unwrap();
        drop(socket);
        let reused: Value = client
            .post(&url)
            .bearer_auth("shell-test-token")
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(reused["data"]["shell"]["incarnation"], incarnation);
        let (mut socket, _) = tokio_tungstenite::connect_async(connect()).await.unwrap();
        socket
            .send(Frame::Text(
                json!({"text":"printf 'state=%s\n' $AC_SHELL_TEST"})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
        socket
            .send(Frame::Text(json!({"key":"enter"}).to_string().into()))
            .await
            .unwrap();
        let mut output = String::new();
        // An immediate reconnect can include the prior broker's bounded
        // teardown as well as pipe setup and snapshot capture after upgrade.
        tokio::time::timeout(Duration::from_secs(10), async {
            while !output.contains("state=kept") {
                if let Some(Ok(Frame::Binary(bytes))) = socket.next().await {
                    output.push_str(&String::from_utf8_lossy(&bytes));
                }
            }
        })
        .await
        .unwrap_or_else(|_| panic!("detach must preserve shell-local state; output: {output:?}"));
        let old = current(&context, &f.tmux, "alice").unwrap().unwrap();
        let old_record = terminal_record(&old);
        let closed: Value = client
            .delete(&url)
            .bearer_auth("shell-test-token")
            .json(&json!({"incarnation":incarnation}))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(closed["data"]["shell"]["status"], "stopped");
        let next: Value = client
            .post(&url)
            .bearer_auth("shell-test-token")
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_ne!(next["data"]["shell"]["incarnation"], incarnation);
        let stale: Value = client
            .delete(&url)
            .bearer_auth("shell-test-token")
            .json(&json!({"incarnation":incarnation}))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(stale["error"]["code"], "shell-incarnation-conflict");
        let mut request = format!(
            "ws://{addr}/shells/alice/attach?incarnation={}",
            next["data"]["shell"]["incarnation"].as_str().unwrap()
        )
        .into_client_request()
        .unwrap();
        request
            .headers_mut()
            .insert("authorization", "Bearer shell-test-token".parse().unwrap());
        let (mut replacement, _) = tokio_tungstenite::connect_async(request).await.unwrap();
        tokio::time::timeout(Duration::from_secs(3), replacement.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(state.attach_brokers.entries.lock().await.len(), 1);
        drop(replacement);
        assert!(
            terminal_command(
                &context,
                &f.tmux,
                &old_record,
                &["capture-pane", "-p", "-t", &old.pane_id]
            )
            .is_err()
        );
        assert!(
            terminal_command(
                &context,
                &f.tmux,
                &old_record,
                &["pipe-pane", "-t", &old.pane_id]
            )
            .is_err()
        );
        assert!(tokio_tungstenite::connect_async(connect()).await.is_err());
        drop(socket);
        state.attach_brokers.shutdown_all().await;
        server.abort();
    }
}
