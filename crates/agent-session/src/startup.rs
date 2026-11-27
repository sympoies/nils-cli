//! Startup observation and bounded failure diagnostics shared by CLI and HTTP create.
pub(crate) mod resume_metrics;

use super::*;
use std::borrow::Cow;
use std::ffi::OsStr;
use std::io::{Read, Seek, SeekFrom};
use std::os::unix::ffi::OsStrExt;
use std::process::Output;

pub(super) const STDERR_FILE: &str = ".startup-stderr.log";

// tmux treats hash runs followed by `[` as literal style syntax in these
// options, while other hashes introduce formats and need escaping.
pub(super) fn literal_tmux_option(value: &str) -> String {
    let mut chars = value.chars().peekable();
    let mut encoded = String::with_capacity(value.len());
    while let Some(ch) = chars.next() {
        if ch != '#' {
            encoded.push(ch);
            continue;
        }
        let mut count = 1;
        while chars.peek() == Some(&'#') {
            chars.next();
            count += 1;
        }
        if chars.peek() != Some(&'[') {
            count *= 2;
        }
        encoded.extend(std::iter::repeat_n('#', count));
    }
    encoded
}

pub(super) fn start_interactive_tmux(
    tmux_bin: &Path,
    agent_bin: &Path,
    agent: AgentKind,
    state_dir: &Path,
    record: &SessionRecord,
    provider_launch_args: &[String],
    agent_args: &[String],
) -> Result<TmuxRuntimeIdentity, CliError> {
    let mut command = ProcessCommand::new(tmux_bin);
    command
        .arg("new-session")
        .arg("-d")
        .arg("-P")
        .arg("-F")
        .arg(TMUX_RUNTIME_IDENTITY_FORMAT)
        .arg("-s")
        .arg(literal_tmux_option(&record.tmux_session))
        .arg("-c")
        .arg(literal_tmux_option(&record.cwd));
    add_runtime_tmux_environment(&mut command, state_dir, record)?;
    begin_held_runtime(&mut command, state_dir, record)?;

    add_interactive_agent_environment(&mut command);

    if agent == AgentKind::Codex && codex_app_server::runtime_is_supported(record) {
        let socket = codex_app_server::socket_path(record).ok_or_else(|| {
            CliError::data(
                "codex-app-server-socket-missing",
                "Codex app-server runtime is missing its private socket",
                Some(json!({ "id": record.id })),
            )
        })?;
        let proxy = codex_app_server::proxy_path(record).ok_or_else(|| {
            CliError::data(
                "codex-app-server-proxy-missing",
                "Codex app-server runtime is missing its private TUI proxy",
                Some(json!({ "id": record.id })),
            )
        })?;
        let handoff = codex_app_server::thread_handoff_path(record).ok_or_else(|| {
            CliError::data(
                "codex-app-server-handoff-missing",
                "Codex app-server runtime is missing its thread handoff path",
                Some(json!({ "id": record.id })),
            )
        })?;
        let attached = codex_app_server::thread_attached_path(record).ok_or_else(|| {
            CliError::data(
                "codex-app-server-attached-marker-missing",
                "Codex app-server runtime is missing its attached marker path",
                Some(json!({ "id": record.id })),
            )
        })?;
        let proxy_bin = current_runtime_helper()?;
        command
            .arg("sh")
            .arg("-c")
            .arg(codex_app_server::launch_script(agent_args))
            .arg("agent-session-codex-app-server")
            .arg(socket)
            .arg(proxy)
            .arg(handoff)
            .arg(attached)
            .arg(proxy_bin)
            .arg(state_dir)
            .arg(&record.id)
            .arg(agent_bin)
            .arg(&record.cwd);
        if provider_launch_args.is_empty() {
            command.arg("--cd").arg(&record.cwd).arg("--no-alt-screen");
        } else {
            command.args(provider_launch_args);
        }
        return run_tmux_new_session(command, agent_args, record);
    }

    command.arg(agent_bin);

    match agent {
        AgentKind::Codex => {
            command.arg("--cd").arg(&record.cwd).arg("--no-alt-screen");
        }
        AgentKind::Claude => {
            command.args(provider_launch_args);
            if let Some(module) =
                crate::send::claude_notice::runtime::prepare_launch(state_dir, record, agent_bin)?
            {
                command.arg("--plugin-dir").arg(module);
            }
            if let Some(title) = record
                .title
                .as_deref()
                .filter(|value| !value.trim().is_empty())
            {
                command.arg("--name").arg(title);
            }
        }
        AgentKind::Dsh => {
            // The launch profile's agent_bin owns the whole command line.
            command.args(provider_launch_args);
        }
    }
    run_tmux_new_session(command, agent_args, record)
}

pub(super) fn start_resume_tmux(
    tmux_bin: &Path,
    agent_bin: &Path,
    state_dir: &Path,
    record: &SessionRecord,
    resume_args: &[String],
) -> Result<TmuxRuntimeIdentity, CliError> {
    let mut command = ProcessCommand::new(tmux_bin);
    command
        .arg("new-session")
        .arg("-d")
        .arg("-P")
        .arg("-F")
        .arg(TMUX_RUNTIME_IDENTITY_FORMAT)
        .arg("-s")
        .arg(literal_tmux_option(&record.tmux_session))
        .arg("-c")
        .arg(literal_tmux_option(&record.cwd));
    add_runtime_tmux_environment(&mut command, state_dir, record)?;
    begin_held_runtime(&mut command, state_dir, record)?;
    add_interactive_agent_environment(&mut command);
    command.arg(agent_bin).args(resume_args);
    if let Some(module) =
        crate::send::claude_notice::runtime::prepare_launch(state_dir, record, agent_bin)?
    {
        command.arg("--plugin-dir").arg(module);
    }
    run_tmux_new_session(command, &record.agent_args, record)
}

/// Apply tmux argv encoding to the full launch before adding the outer scope
/// wrapper's arguments. Launch builders set only arguments and environment.
pub(super) fn output_new_session(
    launch: &ProcessCommand,
    agent_args: &[String],
) -> io::Result<Output> {
    let tmux_bin = Path::new(launch.get_program());
    let mut command = new_session_command(tmux_bin, tmux_scope_runner().as_ref());
    command.args(
        launch
            .get_args()
            .chain(agent_args.iter().map(OsStr::new))
            .map(literal_tmux_argument),
    );
    for (key, value) in launch.get_envs() {
        if let Some(value) = value {
            command.env(key, value);
        } else {
            command.env_remove(key);
        }
    }
    runtime_environment::output(command, PANE_INPUT_COMMAND_TIMEOUT)
}

fn literal_tmux_argument(arg: &OsStr) -> Cow<'_, OsStr> {
    // Both tmux 3.4 and 3.7b interpret a final semicolon before command
    // dispatch and remove one preceding backslash from an escaped argument.
    let Some(prefix) = arg.as_bytes().strip_suffix(b";") else {
        return Cow::Borrowed(arg);
    };
    let mut escaped = OsStr::from_bytes(prefix).to_os_string();
    escaped.push(r"\;");
    Cow::Owned(escaped)
}

/// Observe only the fresh-launch interval. A timeout without a terminal fact
/// keeps the existing uncertain/starting result; elapsed time never proves exit.
pub(super) fn observe(
    context: &CliContext,
    record: &mut SessionRecord,
    tmux: &Path,
    identity: &TmuxRuntimeIdentity,
) -> Result<(), CliError> {
    let timeout = Duration::from_millis(env_u64("AGENT_SESSION_CODEX_CAPTURE_TIMEOUT_MS", 5_000));
    let deadline = Instant::now() + timeout;
    loop {
        let status = session_status(context, tmux, record);
        check_stopped(context, record, &status, tmux)?;
        let phase = activity::activity_status_for_record(context, record)
            .ok()
            .map(|view| view.turn_state.phase);
        // A provider event is stronger than terminal text and prevents a later
        // ordinary exit from being mistaken for an unsuccessful bootstrap.
        if matches!(
            phase,
            Some(activity::TurnPhase::Working | activity::TurnPhase::Waiting)
        ) {
            mark_ready_or_cleanup(context, record, tmux, identity)?;
            return Ok(());
        }
        if status == "running" {
            let target = managed_tmux_pane_target(&record.tmux_session);
            if let Some(pane) = capture_visible_pane(tmux, &target) {
                if let Some(reason) = prompt_reason(&pane) {
                    let cleanup = recover_failed_tmux_launch_bounded(
                        context,
                        record,
                        tmux,
                        Some(identity),
                        SessionTerminationOperation::FailedLaunch,
                    );
                    fail_projection(context, record, reason, cleanup)?;
                    return Err(failure(context, record, reason, cleanup));
                }
                if send_submit::exact_composer_input(&record.agent, &pane).is_some() {
                    mark_ready_or_cleanup(context, record, tmux, identity)?;
                    return Ok(());
                }
            }
            if codex_app_server::runtime_is_supported(record)
                && codex_app_server::thread_attached_path(record).is_some_and(|path| path.is_file())
            {
                mark_ready_or_cleanup(context, record, tmux, identity)?;
                return Ok(());
            }
        }
        if status != "running" || Instant::now() >= deadline {
            return Ok(());
        }
        thread::sleep(
            Duration::from_millis(100).min(deadline.saturating_duration_since(Instant::now())),
        );
    }
}

/// Preserve a known startup exit before the generic prompt cleanup can remove
/// its record or replace its more specific failure projection.
pub(super) fn recover_prompt_failure(
    context: &CliContext,
    record: &mut SessionRecord,
    tmux: &Path,
    identity: Option<&TmuxRuntimeIdentity>,
    operation: SessionTerminationOperation,
) -> Result<FailedLaunchCleanup, CliError> {
    let status = session_status(context, tmux, record);
    check_stopped(context, record, &status, tmux)?;
    Ok(recover_failed_tmux_launch_bounded(
        context, record, tmux, identity, operation,
    ))
}

fn prompt_reason(pane: &str) -> Option<&'static str> {
    let text = pane.to_ascii_lowercase();
    if text.contains("trust this folder")
        || text.contains("do you trust the contents of this directory")
        || text.contains("do you trust the files in this folder")
    {
        Some("startup-trust-required")
    } else if text.contains("select a login method")
        || text.contains("choose how to sign in")
        || text.contains("choose the text style")
        || text.contains("choose the theme")
        || (text.contains("sign in with chatgpt") && text.contains("api key"))
    {
        Some("startup-prompt-required")
    } else {
        None
    }
}

pub(super) fn check_stopped(
    context: &CliContext,
    record: &mut SessionRecord,
    status: &str,
    tmux: &Path,
) -> Result<(), CliError> {
    if status != "stopped" && provider_exit_code(context, record).is_none() {
        return Ok(());
    }
    if startup_projection(record).is_some_and(|startup| startup.state == "ready")
        && startup_failure_marker(context, record).is_none()
    {
        return Ok(());
    }
    if activity::activity_status_for_record(context, record).is_ok_and(|view| {
        matches!(
            view.turn_state.phase,
            activity::TurnPhase::Working | activity::TurnPhase::Waiting
        ) || view.turn_state.last_turn.is_some()
    }) {
        return Ok(());
    }
    // Let the wrapper drain its bounded stderr collector and stop its broker
    // before attempting termination. Otherwise a natural exit can race the
    // exact-pane kill and turn a verified provider failure into pending cleanup.
    if status != "stopped" {
        let deadline = Instant::now() + Duration::from_secs(2);
        while session_status(context, tmux, record) == "running" && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(50));
        }
    }
    // Re-probe the owned boundary: the provider can exit before its wrapper
    // finishes draining stderr. Verify a stopped runtime or terminate the
    // remaining live wrapper and descendants through the same cleanup owner.
    let cleanup = match terminate_tmux_session_with_timeouts(
        context,
        record,
        tmux,
        None,
        PANE_INPUT_COMMAND_TIMEOUT,
        DELETE_TERMINATION_VERIFY_TIMEOUT,
        false,
    ) {
        Ok(()) => FailedLaunchCleanup::Completed,
        Err(reason) => failed_launch_cleanup_from_error(&session_termination_error(
            record,
            reason,
            SessionTerminationOperation::FailedLaunch,
        )),
    };
    fail_projection(context, record, "startup-exited", cleanup)?;
    Err(failure(
        context,
        record,
        "runtime-exited-during-startup",
        cleanup,
    ))
}

fn fail_projection(
    context: &CliContext,
    record: &mut SessionRecord,
    reason: &str,
    cleanup: FailedLaunchCleanup,
) -> Result<(), CliError> {
    let existing = startup_projection(record);
    let started_at = existing
        .as_ref()
        .map(|startup| startup.started_at.as_str())
        .unwrap_or(&record.created_at);
    let mut failed = if reason == "startup-exited" {
        // Preserve the more specific app-server/proxy/client failure marker.
        let stage = startup_stage_marker(context, record).unwrap_or_else(|| "runtime".into());
        let code = startup_failure_marker(context, record)
            .map(|(code, _)| code)
            .unwrap_or_else(|| stopped_startup_failure_code(&stage).into());
        failed_projection(started_at, &code, &stage, None)
    } else {
        failed_projection(started_at, reason, "provider_client", None)
    };
    store_startup_cleanup(&mut failed, cleanup);
    store_startup_projection(record, &failed);
    record.updated_at = Zoned::now().timestamp().to_string();
    write_session_record(context, record)
}

fn provider_exit_code(context: &CliContext, record: &SessionRecord) -> Option<u8> {
    read_bounded_startup_marker(&session_dir(context, &record.id).join(RUNTIME_EXIT_STATUS_FILE))
        .and_then(|value| value.parse::<u8>().ok())
}

fn failure(
    context: &CliContext,
    record: &SessionRecord,
    reason: &str,
    cleanup: FailedLaunchCleanup,
) -> CliError {
    let provider_exit_code = provider_exit_code(context, record);
    let retryable =
        reason == "runtime-exited-during-startup" && cleanup == FailedLaunchCleanup::Completed;
    let (message, next_action) = match reason {
        "startup-trust-required" => (
            "The provider requires folder trust. Review the working directory and configure its trust_level in the provider configuration before retrying.",
            "configure_provider_trust",
        ),
        "startup-prompt-required" => (
            "The provider requires startup configuration or sign-in before retrying.",
            "configure_provider",
        ),
        _ => (
            "The provider exited during session startup.",
            "retry_create",
        ),
    };
    let mut details = json!({
        "reason": reason,
        "provider_exit_code": provider_exit_code,
        "retryable": retryable,
        "safe_to_retry": retryable,
        "next_action": if cleanup == FailedLaunchCleanup::Completed { next_action } else { "inspect_session" },
        "recovery": { "safe_to_retry": retryable, "action": if cleanup == FailedLaunchCleanup::Completed { next_action } else { "inspect_session" } },
        "cleanup": if cleanup == FailedLaunchCleanup::Completed { json!({"state":"completed"}) } else { cleanup.projection() },
    });
    if let Some(tail) = diagnostic_tail(context, record) {
        details["stderr_tail"] = json!(tail);
    }
    with_retained_startup(
        CliError::runtime("start-failed", message, Some(details)),
        record,
    )
}

fn diagnostic_tail(context: &CliContext, record: &SessionRecord) -> Option<String> {
    // Both stderr collectors retain at most 16 KiB. Preserve that context for
    // redaction; the response's smaller suffix limit is applied afterward.
    const CAPTURE_LIMIT: u64 = 16 * 1024;
    let directory = session_dir(context, &record.id);
    let mut text = String::new();
    for name in [STARTUP_DIAGNOSTIC_FILE, STDERR_FILE] {
        let mut file = fs::File::open(directory.join(name)).ok();
        if let Some(file) = file.as_mut() {
            let len = file.metadata().ok()?.len();
            file.seek(SeekFrom::Start(len.saturating_sub(CAPTURE_LIMIT)))
                .ok()?;
            let mut bytes = Vec::new();
            file.take(CAPTURE_LIMIT).read_to_end(&mut bytes).ok()?;
            // At the collector bound, the first line may lack the credential
            // key or other context removed by its raw-byte suffix clipping.
            let start = if len >= CAPTURE_LIMIT {
                bytes
                    .iter()
                    .position(|byte| *byte == b'\n')
                    .map_or(bytes.len(), |index| index + 1)
            } else {
                0
            };
            if start < bytes.len() {
                text.push_str(&String::from_utf8_lossy(&bytes[start..]));
                text.push('\n');
            }
        }
    }
    if text.is_empty() {
        return None;
    }
    for value in [&record.cwd, &display_path(&context.state_dir), &record.id] {
        if !value.is_empty() {
            text = text.replace(value, "[REDACTED]");
        }
    }
    if let Some(prompt_file) = record.prompt_file.as_deref()
        && let Ok(prompt) = fs::read_to_string(prompt_file)
        && !prompt.is_empty()
    {
        text = text.replace(&prompt, "[REDACTED]");
    }
    Some(redact_diagnostic(&text))
}

/// Shared with the Console adapter: never trust an upstream diagnostic to be
/// pre-redacted, and bound UTF-8 bytes rather than Unicode scalar count.
pub(crate) fn redact_diagnostic(text: &str) -> String {
    let plain =
        nils_common::shell::strip_ansi(text, nils_common::shell::AnsiStripMode::CsiAnyTerminator);
    let redacted = nils_common::redact::redact_text(&plain).value;
    let text = redacted
        .split_whitespace()
        .map(|word| {
            if word
                .split(['=', ':', '\'', '"', '('])
                .any(|part| part.starts_with('/') || part.starts_with('\\'))
            {
                "[REDACTED]"
            } else {
                word
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    let text: String = text.chars().filter(|ch| !ch.is_control()).collect();
    let mut start = text.len().saturating_sub(4096);
    while !text.is_char_boundary(start) {
        start += 1;
    }
    text[start..].to_string()
}

fn mark_ready_or_cleanup(
    context: &CliContext,
    record: &mut SessionRecord,
    tmux: &Path,
    identity: &TmuxRuntimeIdentity,
) -> Result<(), CliError> {
    if let Err(error) = mark_ready(context, record) {
        let cleanup = recover_failed_tmux_launch_bounded(
            context,
            record,
            tmux,
            Some(identity),
            SessionTerminationOperation::FailedLaunch,
        );
        return Err(with_retained_startup(
            with_failed_launch_cleanup(error, cleanup),
            record,
        ));
    }
    Ok(())
}

fn mark_ready(context: &CliContext, record: &mut SessionRecord) -> Result<(), CliError> {
    if let Some(startup) = startup_projection(record) {
        let marker = session_dir(context, &record.id).join(STARTUP_STAGE_FILE);
        write_private_file(&marker, b"initial_connection\n")?;
        store_startup_projection(record, &ready_projection(&startup.started_at));
        write_session_record(context, record)?;
    }
    Ok(())
}

pub(super) fn release_held_runtime(
    context: &CliContext,
    record: &SessionRecord,
    kind: resume_metrics::LaunchKind,
) -> Result<(), CliError> {
    let incarnation = coordination::incarnation(record)?;
    let capability = coordination::capability_path(context, &record.id, &incarnation);
    let metadata = fs::metadata(&capability).map_err(|_| {
        CliError::runtime(
            "coordination-broker-start-timeout",
            "coordination capability was not ready before agent launch",
            None,
        )
    })?;
    if !metadata.is_file() || metadata.permissions().mode() & 0o077 != 0 {
        return Err(CliError::runtime(
            "coordination-broker-start-timeout",
            "coordination capability was not private before agent launch",
            None,
        ));
    }
    coordination::ensure_ready(context, record)?;
    resume_metrics::release_gate(
        context,
        record,
        kind,
        &launch_gate_path(&context.state_dir, record),
    )
    .map_err(|_| {
        CliError::runtime(
            "coordination-broker-start-timeout",
            "held agent runtime could not be released",
            None,
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use nils_test_support::{EnvGuard, GlobalStateLock};
    use pretty_assertions::assert_eq;

    #[test]
    fn tmux_new_session_transports_literal_arguments() {
        let lock = GlobalStateLock::new();
        let _no_scope = EnvGuard::remove(&lock, "AGENT_SESSION_TMUX_SCOPE");
        let _discard = EnvGuard::set(&lock, "NILS_TEST_TMUX_DISCARD", "present in parent");
        let temp = tempfile::tempdir().unwrap();
        let tmux = temp.path().join("tmux");
        let argv = temp.path().join("argv");
        fs::write(
            &tmux,
            "#!/bin/sh\nprintf '%s\\n' \"${NILS_TEST_TMUX_DISCARD-unset}\" \"$@\" > \"$NILS_TEST_TMUX_ARGV\"\nexit 1\n",
        )
        .unwrap();
        fs::set_permissions(&tmux, fs::Permissions::from_mode(0o700)).unwrap();
        let record = serde_json::from_value(json!({
            "schema_version": SESSION_DOCUMENT_VERSION,
            "id": "argv-fixture",
            "agent": "codex",
            "mode": "interactive",
            "cwd": "/workspace/project",
            "tmux_session": "argv-fixture",
            "created_at": "2030-01-01T00:00:00Z",
            "updated_at": "2030-01-01T00:00:00Z"
        }))
        .unwrap();
        let mut command = ProcessCommand::new(&tmux);
        command
            .env("NILS_TEST_TMUX_ARGV", &argv)
            .env_remove("NILS_TEST_TMUX_DISCARD")
            .args(["new-session", "-e"])
            .arg(OsStr::from_bytes(b"DEMO=label\xff;"))
            .args(["--", "example"]);
        let agent_args = ["label; with space", "label;", r"label\;", ""].map(str::to_owned);

        let error = run_tmux_new_session(command, &agent_args, &record).unwrap_err();
        assert_eq!(error.code(), "command-failed");
        assert_eq!(
            fs::read(argv).unwrap(),
            b"unset\nnew-session\n-e\nDEMO=label\xff\\;\n--\nexample\nlabel; with space\nlabel\\;\nlabel\\\\;\n\n"
        );
    }
}

#[cfg(test)]
mod literal_option_tests {
    use super::*;
    use nils_test_support::{EnvGuard, GlobalStateLock};
    use pretty_assertions::assert_eq;

    #[test]
    fn tmux_new_session_options_are_literal() {
        let lock = GlobalStateLock::new();
        let _scope = EnvGuard::remove(&lock, "AGENT_SESSION_TMUX_SCOPE");
        let temp = tempfile::tempdir().unwrap();
        let tmux = temp.path().join("tmux");
        let output = temp.path().join("argv");
        fs::write(
            &tmux,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$@\" > {}\nexit 1\n",
                shell_words::quote(&output.to_string_lossy())
            ),
        )
        .unwrap();
        fs::set_permissions(&tmux, fs::Permissions::from_mode(0o700)).unwrap();
        let record: SessionRecord = serde_json::from_value(json!({
            "schema_version": SESSION_DOCUMENT_VERSION, "id":"fixture", "agent":"codex",
            "mode":"interactive", "cwd":"/fixture/#[style]/###[style]/#(command)/#{host}",
            "tmux_session":"fixture-#[style]-###[style]-#(command)-#{host}",
            "prompt_file":"prompt", "log_file":"log",
            "created_at":"2030-01-01T00:00:00Z", "updated_at":"2030-01-01T00:00:00Z",
            "runtime":{"kind":"tmux", "tmux_session":"fixture", "generation":1,
                "started_at":"2030-01-01T00:00:00Z", "launch_id":"fixture-launch"}
        }))
        .unwrap();
        for launch in 0..3 {
            let result = match launch {
                0 => start_interactive_tmux(
                    &tmux,
                    Path::new("agent"),
                    AgentKind::Codex,
                    temp.path(),
                    &record,
                    &[],
                    &[],
                ),
                1 => start_resume_tmux(&tmux, Path::new("agent"), temp.path(), &record, &[]),
                _ => start_run_tmux(
                    &tmux,
                    Path::new("agent"),
                    AgentKind::Codex,
                    temp.path(),
                    &record,
                    &[],
                ),
            };
            assert_eq!(result.unwrap_err().code(), "command-failed");
            let args = fs::read_to_string(&output).unwrap();
            let args: Vec<_> = args.lines().collect();
            let name = args.iter().position(|arg| *arg == "-s").unwrap();
            let cwd = args.iter().position(|arg| *arg == "-c").unwrap();
            assert_eq!(
                (args[name + 1], args[cwd + 1]),
                (
                    "fixture-#[style]-###[style]-##(command)-##{host}",
                    "/fixture/#[style]/###[style]/##(command)/##{host}"
                )
            );
        }
    }
}
