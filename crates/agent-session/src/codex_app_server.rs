//! Strict metadata-only projection of the Codex app-server v2 protocol.
//!
//! The interactive TUI's human-readable error text is intentionally ignored.
//! Structured failures are admitted only when the live protocol reports an
//! allowlisted `codexErrorInfo` and a matching terminal `failed` completion for
//! the same bound thread and turn. Auto-resume distinguishes
//! `usageLimitExceeded` from bounded `serverOverloaded` capacity recovery.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::env;
use std::fs::{self, OpenOptions};
use std::io::{Read, Seek, SeekFrom};
use std::os::fd::AsRawFd;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::net::UnixDatagram;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use futures_util::{SinkExt, StreamExt};
use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{mpsc, oneshot, watch};
use tokio_tungstenite::tungstenite::{Message, protocol::WebSocketConfig};

use crate::{
    AgentKind, CliContext, CliError, ProviderResume, SessionRecord, auto_resume::UsageSnapshot,
    canonical_provider_resume_args, display_path, mutate_session_record, write_private_file,
    write_session_record,
};

pub const MANAGED_ACCOUNT_HANDOFF_CAPABILITY: &str =
    "agent-session.codex-managed-account-handoff.v1";
const MANAGED_ACCOUNT_HANDOFF_CAPABILITY_KEY: &str = "managed_account_handoff_capability";

pub(crate) const RUNTIME_KIND: &str = "codex_app_server";
pub(crate) const PROTOCOL_KEY: &str = "codex_app_server_protocol";
pub(crate) const PROTOCOL_VERSION: &str = "v2";
pub(crate) const SOCKET_KEY: &str = "codex_app_server_socket";
pub(crate) const PROXY_KEY: &str = "codex_app_server_proxy";
pub(crate) const THREAD_HANDOFF_KEY: &str = "codex_app_server_thread_handoff";
pub(crate) const THREAD_ATTACHED_KEY: &str = "codex_app_server_thread_attached";
pub(crate) const ATTENTION_AUTHORITY_KEY: &str = "codex_attention_authority";
pub(crate) const ATTENTION_AUTHORITY_ENV: &str = "AGENT_SESSION_ATTENTION_AUTHORITY";
const ATTENTION_AUTHORITY_PROTOCOL: &str = "protocol";
const ATTENTION_AUTHORITY_HOOK: &str = "hook";

pub(crate) const UNIX_SOCKET_PATH_BUDGET: usize = 100;
/// Digest bytes in a runtime socket name; rendered as twice as many hex digits.
pub(crate) const RUNTIME_NAMESPACE_BYTES: usize = 8;
const MAX_PROTOCOL_ID_BYTES: usize = 256;
const MAX_REDUCER_PENDING_TURNS: usize = 64;
const MAX_PENDING_ATTENTION_REQUESTS: usize = 64;
const MAX_PROXY_OBSERVATIONS: usize = 16;
const MAX_PROXY_OBSERVATION_BYTES: usize = 64 * 1024;
const MAX_PROXY_MESSAGE_BYTES: usize = 16 * 1024 * 1024;
const MAX_PROXY_FRAME_BYTES: usize = 16 * 1024 * 1024;
const CONTROL_RESPONSE_TIMEOUT: Duration = Duration::from_secs(10);
const CONTROL_SUBMISSION_TIMEOUT: Duration = Duration::from_secs(15);
const CONTROL_SUBMIT_TOTAL_TIMEOUT: Duration = Duration::from_secs(30);
const MINIMUM_APP_SERVER_VERSION: (u64, u64, u64) = (0, 145, 0);
const AUDITED_EXACT_ATTENTION_VERSIONS: &[(u64, u64, u64)] = &[(0, 144, 1), (0, 144, 3)];
const APP_SERVER_CAPABILITY_PROBE_TIMEOUT: Duration = Duration::from_secs(2);
const APP_SERVER_CAPABILITY_PROBE_MAX_OUTPUT_BYTES: u64 = 64 * 1024;
const CODEX_ACCOUNT_READINESS_SCHEMA_VERSION: &str = "agent-session.codex-account-readiness.v1";
const MANUAL_INPUT_SECTION_FILE: &str = ".codex-app-server-manual-input";
const MANUAL_INPUT_GATE_FILE: &str = ".codex-app-server-manual-input-gate";
const MANUAL_INPUT_SECTION_VERSION: &str = "agent-session.codex-manual-input.v1";
const MANUAL_INPUT_SECTION_TTL: Duration = Duration::from_secs(30);
const MANUAL_INPUT_GATE_TIMEOUT: Duration = Duration::from_secs(12);
const MANUAL_INPUT_ACK_TIMEOUT: Duration = Duration::from_millis(250);
const PROXY_CAPABILITY_FILE: &str = ".codex-app-server-proxy-capability";
const PROXY_CAPABILITY_VERSION: &str = "agent-session.codex-manual-input-proxy.v1";
const CONVERSATION_CAPABILITY: &str = "agent-session.codex-conversation-rebind.v1";
const PROXY_CAPABILITY_TTL: Duration = Duration::from_secs(365 * 24 * 60 * 60);
const PROXY_CAPABILITY_READY_TIMEOUT: Duration = Duration::from_secs(2);
const SYSTEM_EPHEMERAL_THREADS_FILE: &str = ".codex-app-server-system-ephemeral-threads.json";
const SYSTEM_EPHEMERAL_THREADS_VERSION: &str = "agent-session.codex-system-ephemeral-threads.v1";
const MAX_SYSTEM_EPHEMERAL_THREADS: usize = 16;
const MAX_SYSTEM_EPHEMERAL_THREADS_BYTES: usize = 4096;
pub(crate) const PROVIDER_RESUME_CAPTURE_METHOD: &str = "codex-app-server-thread-binding";

pub(crate) fn attention_authority(record: &SessionRecord) -> &'static str {
    if record.runtime.as_ref().is_some_and(|runtime| {
        runtime.kind == RUNTIME_KIND
            && runtime
                .extra
                .get(ATTENTION_AUTHORITY_KEY)
                .and_then(Value::as_str)
                == Some(ATTENTION_AUTHORITY_PROTOCOL)
    }) {
        ATTENTION_AUTHORITY_PROTOCOL
    } else {
        ATTENTION_AUTHORITY_HOOK
    }
}

pub(crate) fn exact_attention_version_is_audited(version: (u64, u64, u64)) -> bool {
    AUDITED_EXACT_ATTENTION_VERSIONS.contains(&version)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct AppServerCapabilities {
    transport: bool,
    exact_attention: bool,
    source_guard: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct CodexAccountReadiness {
    schema_version: &'static str,
    supported: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    provider_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason_code: Option<&'static str>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct AppServerProbe {
    capabilities: AppServerCapabilities,
    provider_version: Option<String>,
    reason_code: Option<&'static str>,
}

impl AppServerProbe {
    fn unavailable(reason_code: &'static str, provider_version: Option<String>) -> Self {
        Self {
            capabilities: AppServerCapabilities {
                transport: false,
                exact_attention: false,
                source_guard: false,
            },
            provider_version,
            reason_code: Some(reason_code),
        }
    }
}

pub(crate) fn account_binding_readiness(
    agent_bin: &Path,
    state_dir: &Path,
) -> CodexAccountReadiness {
    let probe = app_server_probe(agent_bin);
    let mut supported = probe.capabilities.transport;
    let mut reason_code = probe.reason_code;
    // A capable CLI still needs a usable private socket directory.
    if supported
        && let Err(err) = private_runtime_dir(state_dir)
            .and_then(|dir| socket_path_in(&dir, &"0".repeat(RUNTIME_NAMESPACE_BYTES * 2)))
    {
        supported = false;
        reason_code = Some(crate::codex_runtime_dir::fallback_reason(err.code()));
    }
    CodexAccountReadiness {
        schema_version: CODEX_ACCOUNT_READINESS_SCHEMA_VERSION,
        supported,
        provider_version: probe.provider_version,
        reason_code,
    }
}

pub(crate) fn configure_runtime(
    context: &CliContext,
    agent_bin: &Path,
    record: &mut SessionRecord,
    managed: bool,
) -> Result<(), CliError> {
    if record.agent != "codex" {
        return Ok(());
    }
    let binding_required = crate::codex_account::binding_is_present(record);
    if !managed && !binding_required {
        return Ok(());
    }
    let preference = env::var("AGENT_SESSION_CODEX_RUNTIME").unwrap_or_else(|_| "auto".into());
    if !matches!(preference.as_str(), "auto" | "app-server") && !binding_required {
        return Ok(());
    }
    let forced = preference == "app-server" || binding_required;
    let probe = app_server_probe(agent_bin);
    let mut capabilities = probe.capabilities;
    capabilities.source_guard = crate::activity::codex_protocol_attention_source_guard_configured();
    if !capabilities.transport {
        if forced {
            return Err(CliError::data(
                "codex-app-server-capability-unavailable",
                "installed Codex does not advertise app-server Unix listen support",
                Some(json!({
                    "provider_version": probe.provider_version,
                    "reason_code": probe.reason_code,
                })),
            ));
        }
        crate::codex_runtime_dir::record_fallback(
            record,
            probe
                .reason_code
                .unwrap_or("codex-app-server-transport-unavailable"),
        );
        return write_session_record(context, record);
    }
    configure_runtime_with_capabilities(context, record, forced, managed, capabilities)
}

fn configure_runtime_with_capabilities(
    context: &CliContext,
    record: &mut SessionRecord,
    forced: bool,
    managed: bool,
    capabilities: AppServerCapabilities,
) -> Result<(), CliError> {
    let socket = match allocate_socket_path(context, record) {
        Ok(socket) => socket,
        Err(err) if !forced => {
            crate::codex_runtime_dir::record_fallback(record, err.code());
            return write_session_record(context, record);
        }
        Err(err) => return Err(err),
    };
    crate::codex_runtime_dir::clear_fallback(record);
    let runtime = record.runtime.as_mut().ok_or_else(|| {
        CliError::data(
            "runtime-id-missing",
            "session runtime is missing its launch metadata",
            Some(json!({ "id": record.id })),
        )
    })?;
    runtime.kind = RUNTIME_KIND.to_string();
    runtime.extra.insert(
        ATTENTION_AUTHORITY_KEY.to_string(),
        json!(
            if capabilities.exact_attention && capabilities.source_guard {
                ATTENTION_AUTHORITY_PROTOCOL
            } else {
                ATTENTION_AUTHORITY_HOOK
            }
        ),
    );
    runtime
        .extra
        .insert(PROTOCOL_KEY.to_string(), json!(PROTOCOL_VERSION));
    runtime
        .extra
        .insert(SOCKET_KEY.to_string(), json!(display_path(&socket)));
    runtime.extra.insert(
        PROXY_KEY.to_string(),
        json!(display_path(&socket.with_extension("proxy"))),
    );
    runtime.extra.insert(
        THREAD_HANDOFF_KEY.to_string(),
        json!(display_path(&socket.with_extension("thread"))),
    );
    runtime.extra.insert(
        THREAD_ATTACHED_KEY.to_string(),
        json!(display_path(&socket.with_extension("attached"))),
    );
    if managed {
        runtime.extra.insert(
            MANAGED_ACCOUNT_HANDOFF_CAPABILITY_KEY.to_string(),
            json!(MANAGED_ACCOUNT_HANDOFF_CAPABILITY),
        );
    } else {
        runtime.extra.remove(MANAGED_ACCOUNT_HANDOFF_CAPABILITY_KEY);
    }
    write_session_record(context, record)
}

pub fn managed_account_handoff_supported(record: &SessionRecord) -> bool {
    record.runtime.as_ref().is_some_and(|runtime| {
        runtime.kind == RUNTIME_KIND
            && runtime
                .extra
                .get(MANAGED_ACCOUNT_HANDOFF_CAPABILITY_KEY)
                .and_then(Value::as_str)
                == Some(MANAGED_ACCOUNT_HANDOFF_CAPABILITY)
    })
}

#[cfg(test)]
fn app_server_capabilities(agent_bin: &Path) -> AppServerCapabilities {
    app_server_probe(agent_bin).capabilities
}

fn app_server_probe(agent_bin: &Path) -> AppServerProbe {
    let Some(version) = bounded_command_output(agent_bin, &["--version"]) else {
        return AppServerProbe::unavailable("codex-unavailable", None);
    };
    let version_text = String::from_utf8_lossy(&version.stdout);
    let Some(version) = parse_version_triplet(&version_text) else {
        return AppServerProbe::unavailable("codex-version-unrecognized", None);
    };
    let provider_version = Some(format!("{}.{}.{}", version.0, version.1, version.2));
    if version < MINIMUM_APP_SERVER_VERSION {
        return AppServerProbe::unavailable("codex-version-too-old", provider_version);
    }
    let Some(output) = bounded_command_output(agent_bin, &["app-server", "--help"]) else {
        return AppServerProbe::unavailable(
            "codex-app-server-transport-unavailable",
            provider_version,
        );
    };
    let advertised_transport = [output.stdout, output.stderr].into_iter().any(|bytes| {
        let text = String::from_utf8_lossy(&bytes);
        text.contains("--listen <URL>") && text.contains("unix://")
    });
    if !advertised_transport {
        return AppServerProbe::unavailable(
            "codex-app-server-transport-unavailable",
            provider_version,
        );
    }
    AppServerProbe {
        capabilities: AppServerCapabilities {
            transport: true,
            exact_attention: exact_attention_version_is_audited(version),
            source_guard: false,
        },
        provider_version,
        reason_code: None,
    }
}

fn bounded_command_output(agent_bin: &Path, args: &[&str]) -> Option<std::process::Output> {
    bounded_command_output_with_timeout(agent_bin, args, APP_SERVER_CAPABILITY_PROBE_TIMEOUT)
}

fn bounded_command_output_with_timeout(
    agent_bin: &Path,
    args: &[&str],
    timeout: Duration,
) -> Option<std::process::Output> {
    let Ok(mut child) = Command::new(agent_bin)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn()
    else {
        return None;
    };
    let mut stdout_pipe = child.stdout.take()?;
    let mut stderr_pipe = child.stderr.take()?;
    let (output_tx, output_rx) = std::sync::mpsc::channel();
    let stdout_tx = output_tx.clone();
    thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stdout_pipe
            .by_ref()
            .take(APP_SERVER_CAPABILITY_PROBE_MAX_OUTPUT_BYTES)
            .read_to_end(&mut bytes);
        let _ = stdout_tx.send((true, bytes));
    });
    thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stderr_pipe
            .by_ref()
            .take(APP_SERVER_CAPABILITY_PROBE_MAX_OUTPUT_BYTES)
            .read_to_end(&mut bytes);
        let _ = output_tx.send((false, bytes));
    });

    let deadline = Instant::now() + timeout;
    let mut status = None;
    let mut stdout = None;
    let mut stderr = None;
    loop {
        if status.is_none() {
            match child.try_wait() {
                Ok(Some(exit)) if exit.success() => status = Some(exit),
                Ok(Some(_)) | Err(_) => {
                    terminate_probe_process_group(&mut child);
                    return None;
                }
                Ok(None) => {}
            }
        }
        while let Ok((is_stdout, bytes)) = output_rx.try_recv() {
            if is_stdout {
                stdout = Some(bytes);
            } else {
                stderr = Some(bytes);
            }
        }
        if status.is_some() && stdout.is_some() && stderr.is_some() {
            return Some(std::process::Output {
                status: status.take().expect("checked probe status"),
                stdout: stdout.take().expect("checked probe stdout"),
                stderr: stderr.take().expect("checked probe stderr"),
            });
        }
        if Instant::now() >= deadline {
            terminate_probe_process_group(&mut child);
            return None;
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn terminate_probe_process_group(child: &mut std::process::Child) {
    let pid = child.id();
    // SAFETY: the probe is launched as the leader of a fresh process group.
    unsafe {
        libc::kill(-(pid as i32), libc::SIGKILL);
    }
    let _ = child.kill();
    let _ = child.wait();
}

fn parse_version_triplet(raw: &str) -> Option<(u64, u64, u64)> {
    let mut fields = raw.split_whitespace();
    if fields.next()? != "codex-cli" {
        return None;
    }
    let token = fields.next()?;
    if fields.next().is_some() {
        return None;
    }
    let mut parts = token.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

fn runtime_namespace(context: &CliContext, record: &SessionRecord) -> Result<String, CliError> {
    let launch_id = record
        .runtime
        .as_ref()
        .map(|runtime| runtime.launch_id.as_str())
        .filter(|launch_id| !launch_id.is_empty())
        .ok_or_else(|| {
            CliError::data(
                "runtime-id-missing",
                "session runtime is missing its launch metadata",
                Some(json!({ "id": record.id })),
            )
        })?;
    let mut digest = Sha256::new();
    digest.update(context.state_dir.as_os_str().as_bytes());
    digest.update([0]);
    digest.update(record.id.as_bytes());
    digest.update([0]);
    digest.update(launch_id.as_bytes());
    Ok(digest
        .finalize()
        .iter()
        .take(RUNTIME_NAMESPACE_BYTES)
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn validate_private_runtime_dir(path: &Path) -> Result<(), CliError> {
    let metadata = fs::symlink_metadata(path).map_err(|err| {
        CliError::runtime(
            "codex-app-server-runtime-dir-unavailable",
            format!("Codex app-server runtime directory is unavailable: {err}"),
            None,
        )
    })?;
    if metadata.file_type().is_symlink()
        || !metadata.is_dir()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o077 != 0
    {
        return Err(CliError::runtime(
            "codex-app-server-runtime-dir-unsafe",
            "Codex app-server requires an owned, non-symlinked 0700 runtime directory",
            None,
        ));
    }
    Ok(())
}

fn private_runtime_dir(state_dir: &Path) -> Result<PathBuf, CliError> {
    let runtime_root = crate::codex_runtime_dir::runtime_root(state_dir)?;
    validate_private_runtime_dir(&runtime_root)?;
    let dir = runtime_root.join("agent-session");
    match fs::create_dir(&dir) {
        Ok(()) => fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).map_err(|err| {
            CliError::runtime(
                "codex-app-server-runtime-dir-unavailable",
                format!("failed to secure the Codex app-server runtime directory: {err}"),
                None,
            )
        })?,
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(err) => {
            return Err(CliError::runtime(
                "codex-app-server-runtime-dir-unavailable",
                format!("failed to create the Codex app-server runtime directory: {err}"),
                None,
            ));
        }
    }
    validate_private_runtime_dir(&dir)?;
    Ok(dir)
}

fn allocate_socket_path(context: &CliContext, record: &SessionRecord) -> Result<PathBuf, CliError> {
    let dir = private_runtime_dir(&context.state_dir)?;
    let suffix = runtime_namespace(context, record)?;
    socket_path_in(&dir, &suffix)
}

fn socket_path_in(dir: &Path, suffix: &str) -> Result<PathBuf, CliError> {
    let path = dir.join(format!("cx-{suffix}.sock"));
    if path.as_os_str().as_encoded_bytes().len() > UNIX_SOCKET_PATH_BUDGET {
        return Err(CliError::runtime(
            "codex-app-server-socket-path-too-long",
            "the Codex runtime directory is too long for a private Unix socket",
            None,
        ));
    }
    Ok(path)
}

fn persisted_runtime_paths(
    context: &CliContext,
    record: &SessionRecord,
) -> Result<[PathBuf; 4], CliError> {
    let socket = socket_path(record).map(PathBuf::from).ok_or_else(|| {
        CliError::data(
            "codex-app-server-runtime-path-invalid",
            "Codex app-server socket metadata is missing",
            None,
        )
    })?;
    let proxy = proxy_path(record).map(PathBuf::from).ok_or_else(|| {
        CliError::data(
            "codex-app-server-runtime-path-invalid",
            "Codex app-server proxy metadata is missing",
            None,
        )
    })?;
    let handoff = thread_handoff_path(record)
        .map(PathBuf::from)
        .ok_or_else(|| {
            CliError::data(
                "codex-app-server-runtime-path-invalid",
                "Codex app-server handoff metadata is missing",
                None,
            )
        })?;
    let attached = thread_attached_path(record)
        .map(PathBuf::from)
        .ok_or_else(|| {
            CliError::data(
                "codex-app-server-runtime-path-invalid",
                "Codex app-server attached metadata is missing",
                None,
            )
        })?;
    let expected_name = format!("cx-{}.sock", runtime_namespace(context, record)?);
    let valid = socket.file_name().and_then(|name| name.to_str()) == Some(expected_name.as_str())
        && socket
            .parent()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            == Some("agent-session")
        && proxy == socket.with_extension("proxy")
        && handoff == socket.with_extension("thread")
        && attached == socket.with_extension("attached");
    if !valid {
        return Err(CliError::data(
            "codex-app-server-runtime-path-invalid",
            "Codex app-server runtime paths do not match the session runtime identity",
            None,
        ));
    }
    Ok([socket, proxy, handoff, attached])
}

const STARTUP_DIAGNOSTIC_COLLECTOR_SCRIPT: &str = r#"collect_startup_diagnostic() {
  if (umask 077; tail -c 16384 > "$startup_diagnostic_buffer"); then
    runtime_exit="$(cat "$runtime_exit_status" 2>/dev/null)"
    if { [ "$(cat "$startup_stage" 2>/dev/null)" != initial_connection ] ||
         { [ -n "$runtime_exit" ] && [ "$runtime_exit" != 0 ]; }; } &&
       [ -s "$startup_diagnostic_buffer" ]; then
      mv -f "$startup_diagnostic_buffer" "$startup_diagnostic" 2>/dev/null ||
        rm -f -- "$startup_diagnostic_buffer"
    else
      rm -f -- "$startup_diagnostic_buffer"
    fi
    if [ "$runtime_exit" = 0 ]; then
      rm -f -- "$runtime_exit_status"
    fi
  else
    rm -f -- "$startup_diagnostic_buffer"
  fi
}"#;

/// The remote TUI forwards only a subset of its configuration to thread/start.
/// Give the already session-owned server the explicit configuration flags too,
/// leaving TOML parsing and override precedence to Codex itself. Stop at `--`
/// so option-shaped prompt text stays literal.
fn app_server_config_args(agent_args: &[String]) -> Vec<&str> {
    let mut config = Vec::new();
    let mut args = agent_args.iter().map(String::as_str);
    while let Some(arg) = args.next() {
        match arg {
            "--" => break,
            "-c" | "--config" | "--enable" | "--disable" => {
                config.push(arg);
                if let Some(value) = args.next() {
                    config.push(value);
                }
            }
            _ if arg.starts_with("--config=")
                || arg.starts_with("--enable=")
                || arg.starts_with("--disable=")
                || arg.starts_with("-c") =>
            {
                config.push(arg)
            }
            _ => {}
        }
    }
    config
}

pub(crate) fn launch_script(agent_args: &[String]) -> String {
    [
        r#"socket=$1
proxy=$2
handoff=$3
attached=$4
proxy_bin=$5
state_dir=$6
session_id=$7
agent=$8
cwd=$9
shift 9
startup_dir="$state_dir/sessions/$session_id"
startup_stage="$startup_dir/.startup-stage"
startup_failure="$startup_dir/.startup-failure"
startup_diagnostic="$startup_dir/.startup-diagnostic.log"
runtime_exit_status="$startup_dir/.runtime-exit-status"
startup_diagnostic_buffer="$startup_dir/.startup-diagnostic.buffer"
startup_diagnostic_pipe="$startup_dir/.startup-diagnostic.pipe"
provider_stderr_pipe="$startup_dir/.provider-stderr.pipe"
"#,
        STARTUP_DIAGNOSTIC_COLLECTOR_SCRIPT,
        r#"
write_startup_marker() {
  (umask 077; printf '%s\n' "$2" > "$1") 2>/dev/null || true
}
record_startup_failure() {
  write_startup_marker "$startup_failure" "$1"
}
rm -f -- "$startup_failure" "$startup_diagnostic" "$runtime_exit_status" "$startup_diagnostic_buffer" "$startup_diagnostic_pipe" "$provider_stderr_pipe"
write_startup_marker "$startup_stage" app_server
if ! (umask 077; mkfifo "$startup_diagnostic_pipe"); then
  record_startup_failure startup-exited
  exit 1
fi
(umask 077; collect_startup_diagnostic < "$startup_diagnostic_pipe") &
diagnostic_pid=$!
rm -f -- "$socket" "$proxy" "$attached"
"$agent" app-server --listen "unix://$socket" __SESSION_CONFIG_ARGS__ </dev/null >/dev/null 2>"$startup_diagnostic_pipe" &
server=$!
proxy_pid=
provider_stderr_pid=
diagnostic_hold_open=
cleanup_started=
cleanup() {
  if [ -n "$cleanup_started" ]; then
    return
  fi
  cleanup_started=1
  trap - EXIT
  trap '' HUP INT TERM
  if [ -n "$diagnostic_hold_open" ]; then
    exec 9>&-
    diagnostic_hold_open=
  fi
  if [ -n "$proxy_pid" ]; then
    owned_pid=$proxy_pid
    proxy_pid=
    kill "$owned_pid" 2>/dev/null || true
    wait "$owned_pid" 2>/dev/null || true
  fi
  if [ -n "$server" ]; then
    owned_pid=$server
    server=
    kill "$owned_pid" 2>/dev/null || true
    wait "$owned_pid" 2>/dev/null || true
  fi
  if [ -n "$provider_stderr_pid" ]; then
    owned_pid=$provider_stderr_pid
    provider_stderr_pid=
    kill "$owned_pid" 2>/dev/null || true
    sleep 0.25
    kill -9 "$owned_pid" 2>/dev/null || true
    wait "$owned_pid" 2>/dev/null || true
  fi
  if [ -n "$diagnostic_pid" ]; then
    owned_pid=$diagnostic_pid
    diagnostic_pid=
    wait "$owned_pid" 2>/dev/null || true
  fi
  rm -f -- "$socket" "$proxy" "$handoff" "$attached" "$startup_diagnostic_buffer" "$startup_diagnostic_pipe" "$provider_stderr_pipe"
}
handle_signal() {
  signal_status=$1
  cleanup
  exit "$signal_status"
}
trap cleanup EXIT
trap 'handle_signal 129' HUP
trap 'handle_signal 130' INT
trap 'handle_signal 143' TERM
i=0
while [ ! -S "$socket" ]; do
  if ! kill -0 "$server" 2>/dev/null; then
    record_startup_failure app-server-start-failed
    exit 1
  fi
  i=$((i + 1))
  if [ "$i" -ge 100 ]; then
    record_startup_failure startup-timeout
    exit 1
  fi
  sleep 0.05
done
write_startup_marker "$startup_stage" proxy
(umask 077; exec "$proxy_bin" --state-dir "$state_dir" codex-app-server-proxy --id "$session_id" --upstream "$socket" --listen "$proxy" </dev/null >/dev/null 2>"$startup_diagnostic_pipe") &
proxy_pid=$!
i=0
while [ ! -S "$proxy" ]; do
  if ! kill -0 "$proxy_pid" 2>/dev/null; then
    if [ ! -x "$proxy_bin" ]; then
      record_startup_failure runtime-helper-unavailable
    else
      record_startup_failure proxy-start-failed
    fi
    exit 1
  fi
  i=$((i + 1))
  if [ "$i" -ge 100 ]; then
    record_startup_failure startup-timeout
    exit 1
  fi
  sleep 0.05
done
write_startup_marker "$startup_stage" provider_client
if ! (umask 077; mkfifo "$provider_stderr_pipe"); then
  record_startup_failure startup-exited
  exit 1
fi
tee "$startup_diagnostic_pipe" < "$provider_stderr_pipe" >&2 &
provider_stderr_pid=$!
if ! exec 9>"$startup_diagnostic_pipe"; then
  record_startup_failure startup-exited
  exit 1
fi
diagnostic_hold_open=1
"$agent" -c check_for_update_on_startup=false --remote "unix://$proxy" "$@" 9>&- 2>"$provider_stderr_pipe"
status=$?
if [ "$(cat "$startup_stage" 2>/dev/null)" != initial_connection ] ||
   { [ "$status" != 0 ] && [ ! -f "$attached" ]; }; then
  record_startup_failure provider-client-exited
fi
write_startup_marker "$runtime_exit_status" "$status"
exec 9>&-
diagnostic_hold_open=
sleep 0.25
kill "$provider_stderr_pid" 2>/dev/null || true
sleep 0.25
kill -9 "$provider_stderr_pid" 2>/dev/null || true
owned_pid=$provider_stderr_pid
provider_stderr_pid=
wait "$owned_pid" 2>/dev/null || true
rm -f -- "$provider_stderr_pipe"
exit "$status"
"#,
    ]
    .concat()
    .replace(
        "__SESSION_CONFIG_ARGS__",
        &shell_words::join(app_server_config_args(agent_args)),
    )
}

pub(crate) fn runtime_is_supported(record: &SessionRecord) -> bool {
    record.agent == "codex"
        && record.runtime.as_ref().is_some_and(|runtime| {
            runtime.kind == RUNTIME_KIND
                && runtime.extra.get(PROTOCOL_KEY).and_then(Value::as_str) == Some(PROTOCOL_VERSION)
                && runtime
                    .extra
                    .get(SOCKET_KEY)
                    .and_then(Value::as_str)
                    .is_some_and(|socket| Path::new(socket).is_absolute())
                && runtime
                    .extra
                    .get(PROXY_KEY)
                    .and_then(Value::as_str)
                    .is_some_and(|proxy| Path::new(proxy).is_absolute())
                && runtime
                    .extra
                    .get(THREAD_HANDOFF_KEY)
                    .and_then(Value::as_str)
                    .is_some_and(|path| Path::new(path).is_absolute())
                && runtime
                    .extra
                    .get(THREAD_ATTACHED_KEY)
                    .and_then(Value::as_str)
                    .is_some_and(|path| Path::new(path).is_absolute())
        })
}

#[derive(Debug, Deserialize, Serialize)]
struct RuntimeProcessMarker {
    schema_version: String,
    launch_id: String,
    token: String,
    owner_pid: u32,
    expires_at_epoch_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    conversation_capability: Option<String>,
}

#[derive(Debug)]
pub(crate) struct ManualInputMarker {
    path: PathBuf,
    token: String,
    _owner_file: fs::File,
    gate_path: Option<PathBuf>,
    ack_path: Option<PathBuf>,
    ack_socket: Option<UnixDatagram>,
    cleanup_on_drop: bool,
}

impl ManualInputMarker {
    pub(crate) fn finish(mut self, release_lifecycle_lock: impl FnOnce()) {
        self.finish_with_timeout(release_lifecycle_lock, MANUAL_INPUT_GATE_TIMEOUT);
    }

    fn finish_with_timeout(&mut self, release_lifecycle_lock: impl FnOnce(), timeout: Duration) {
        if let Some(socket) = self.ack_socket.as_ref() {
            let mut ack = [0_u8; 1];
            let _ = socket.recv(&mut ack);
        }
        let gate = self
            .gate_path
            .as_deref()
            .and_then(open_manual_input_gate_file)
            .filter(|file| lock_file_timed(file, timeout));
        if let Some(gate) = gate {
            self.remove_if_owned();
            self.remove_ack_path();
            release_lifecycle_lock();
            unlock_bootstrap_file(&gate);
        } else {
            // Invalidate before releasing lifecycle state. Even if unlink
            // fails, dropping this marker releases the continuously held owner
            // lease, so stale bytes cannot authorize a future Busy result.
            self.remove_if_owned();
            self.remove_ack_path();
            release_lifecycle_lock();
        }
        self.cleanup_on_drop = false;
    }

    fn remove_ack_path(&mut self) {
        if let Some(path) = self.ack_path.take() {
            let _ = fs::remove_file(path);
        }
    }

    fn remove_if_owned(&mut self) {
        let owned =
            read_runtime_process_marker(&self.path).is_some_and(|owner| owner.token == self.token);
        if owned {
            let _ = fs::remove_file(&self.path);
        }
    }
}

impl Drop for ManualInputMarker {
    fn drop(&mut self) {
        if self.cleanup_on_drop {
            self.remove_if_owned();
        }
        self.remove_ack_path();
    }
}

struct ProxyCapabilityGuard {
    _marker: ManualInputMarker,
}

fn epoch_millis() -> Option<u64> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_millis();
    u64::try_from(millis).ok()
}

fn manual_input_section_path(context: &CliContext, record: &SessionRecord) -> PathBuf {
    crate::session_dir(context, &record.id).join(MANUAL_INPUT_SECTION_FILE)
}

fn manual_input_gate_path(context: &CliContext, record: &SessionRecord) -> PathBuf {
    account_mutation_gate_path(context, &record.id)
}

fn account_mutation_gate_path(context: &CliContext, id: &str) -> PathBuf {
    crate::session_dir(context, id).join(MANUAL_INPUT_GATE_FILE)
}

fn manual_input_ack_path(record: &SessionRecord) -> Option<PathBuf> {
    proxy_path(record).map(|path| path.with_extension("ack"))
}

fn proxy_capability_path(context: &CliContext, record: &SessionRecord) -> PathBuf {
    crate::session_dir(context, &record.id).join(PROXY_CAPABILITY_FILE)
}

fn read_runtime_process_marker(path: &Path) -> Option<RuntimeProcessMarker> {
    let metadata = fs::symlink_metadata(path).ok()?;
    if !metadata.file_type().is_file() || metadata.len() > 1024 {
        return None;
    }
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}

fn read_runtime_process_marker_file(file: &mut fs::File) -> Option<RuntimeProcessMarker> {
    let metadata = file.metadata().ok()?;
    if !metadata.file_type().is_file() || metadata.len() > 1024 {
        return None;
    }
    file.seek(SeekFrom::Start(0)).ok()?;
    let mut bytes = Vec::with_capacity(usize::try_from(metadata.len()).ok()?);
    file.read_to_end(&mut bytes).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn process_is_live(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return false;
    };
    // SAFETY: signal 0 probes process existence without delivering a signal.
    unsafe { libc::kill(pid, 0) == 0 }
}

fn valid_runtime_process_marker(
    marker: &RuntimeProcessMarker,
    record: &SessionRecord,
    schema_version: &str,
    ttl: Duration,
) -> bool {
    let Some(runtime) = record.runtime.as_ref() else {
        return false;
    };
    let Some(now) = epoch_millis() else {
        return false;
    };
    let ttl_ms = u64::try_from(ttl.as_millis()).unwrap_or(u64::MAX);
    marker.schema_version == schema_version
        && marker.launch_id == runtime.launch_id
        && uuid::Uuid::parse_str(&marker.token).is_ok()
        && process_is_live(marker.owner_pid)
        && marker.expires_at_epoch_ms >= now
        && marker.expires_at_epoch_ms <= now.saturating_add(ttl_ms)
}

fn write_runtime_process_marker(
    path: PathBuf,
    record: &SessionRecord,
    schema_version: &str,
    ttl: Duration,
) -> Result<ManualInputMarker, CliError> {
    let launch_id = record
        .runtime
        .as_ref()
        .map(|runtime| runtime.launch_id.clone())
        .ok_or_else(|| {
            CliError::data(
                "runtime-id-missing",
                "session runtime is missing its launch metadata",
                Some(json!({ "id": record.id })),
            )
        })?;
    let now = epoch_millis().ok_or_else(|| {
        CliError::runtime(
            "codex-input-section-time-unavailable",
            "system time is unavailable for Codex manual input",
            Some(json!({ "id": record.id })),
        )
    })?;
    let ttl_ms = u64::try_from(ttl.as_millis()).unwrap_or(u64::MAX);
    let token = uuid::Uuid::new_v4().to_string();
    let marker = RuntimeProcessMarker {
        schema_version: schema_version.to_string(),
        launch_id,
        token: token.clone(),
        owner_pid: std::process::id(),
        expires_at_epoch_ms: now.saturating_add(ttl_ms),
        conversation_capability: (schema_version == PROXY_CAPABILITY_VERSION)
            .then(|| CONVERSATION_CAPABILITY.to_string()),
    };
    let bytes = serde_json::to_vec(&marker).map_err(|err| {
        CliError::runtime(
            "codex-input-section-encode-failed",
            format!("failed to encode Codex manual input state: {err}"),
            Some(json!({ "id": record.id })),
        )
    })?;
    write_private_file(&path, &bytes)?;
    let file = fs::File::open(&path).map_err(|err| {
        CliError::runtime(
            "codex-input-section-open-failed",
            format!("failed to open Codex manual input state: {err}"),
            Some(json!({ "id": record.id })),
        )
    })?;
    // The marker inode is also an owner lease. A proxy authorizes Busy only
    // while this shared lock is continuously held by the serialized sender.
    // SAFETY: `flock` observes the valid descriptor borrowed for this call.
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_SH) } != 0 {
        return Err(CliError::runtime(
            "codex-input-section-lease-failed",
            "failed to lock Codex manual input state",
            Some(json!({ "id": record.id })),
        ));
    }
    Ok(ManualInputMarker {
        path,
        token,
        _owner_file: file,
        gate_path: None,
        ack_path: None,
        ack_socket: None,
        cleanup_on_drop: true,
    })
}

pub(crate) fn input_contains_submission(
    text: Option<&str>,
    keys: &[crate::cli::SpecialKey],
) -> bool {
    text.is_some_and(crate::text_is_bare_newline) || keys.contains(&crate::cli::SpecialKey::Enter)
}

pub(crate) fn ensure_manual_input_capability(
    context: &CliContext,
    record: &SessionRecord,
) -> Result<(), CliError> {
    crate::codex_account::ensure_terminal_input_allowed(record)?;
    if !runtime_is_supported(record) {
        return Ok(());
    }
    let deadline = Instant::now() + PROXY_CAPABILITY_READY_TIMEOUT;
    loop {
        if live_proxy_capability(context, record) {
            return Ok(());
        }
        if Instant::now() >= deadline {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    Err(CliError::runtime(
        "codex-input-section-unavailable",
        "the Codex proxy did not advertise serialized input support; retry, then recreate the session if it persists",
        Some(json!({ "id": record.id, "retryable": true })),
    ))
}

fn live_proxy_marker(context: &CliContext, record: &SessionRecord) -> Option<RuntimeProcessMarker> {
    let path = proxy_capability_path(context, record);
    let Ok(mut file) = fs::File::open(&path) else {
        return None;
    };
    let (Ok(own), Ok(current)) = (file.metadata(), fs::metadata(&path)) else {
        return None;
    };
    if own.dev() != current.dev() || own.ino() != current.ino() {
        return None;
    }
    // A live proxy holds a shared lock for its complete advertised lifetime.
    // Acquiring an exclusive lock therefore identifies an unlocked stale file.
    // SAFETY: `flock` observes the valid descriptor borrowed for this call.
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
        unlock_bootstrap_file(&file);
        return None;
    }
    if std::io::Error::last_os_error().kind() != std::io::ErrorKind::WouldBlock {
        return None;
    }
    read_runtime_process_marker_file(&mut file).filter(|marker| {
        valid_runtime_process_marker(
            marker,
            record,
            PROXY_CAPABILITY_VERSION,
            PROXY_CAPABILITY_TTL,
        )
    })
}

fn live_proxy_capability(context: &CliContext, record: &SessionRecord) -> bool {
    live_proxy_marker(context, record).is_some()
}

pub(crate) fn live_conversation_capability(context: &CliContext, record: &SessionRecord) -> bool {
    live_proxy_marker(context, record).is_some_and(|marker| {
        marker.conversation_capability.as_deref() == Some(CONVERSATION_CAPABILITY)
    })
}

pub(crate) fn begin_manual_input_section(
    context: &CliContext,
    record: &SessionRecord,
) -> Result<Option<ManualInputMarker>, CliError> {
    if !runtime_is_supported(record) {
        return Ok(None);
    }
    ensure_manual_input_capability(context, record)?;
    let mut marker = write_runtime_process_marker(
        manual_input_section_path(context, record),
        record,
        MANUAL_INPUT_SECTION_VERSION,
        MANUAL_INPUT_SECTION_TTL,
    )?;
    let gate_path = manual_input_gate_path(context, record);
    open_manual_input_gate_file(&gate_path).ok_or_else(|| {
        CliError::runtime(
            "codex-input-gate-open-failed",
            "failed to open the Codex manual input gate",
            Some(json!({ "id": record.id })),
        )
    })?;
    marker.gate_path = Some(gate_path);
    let ack_path = manual_input_ack_path(record).ok_or_else(|| {
        CliError::data(
            "codex-input-ack-path-missing",
            "Codex runtime is missing its private input acknowledgement path",
            Some(json!({ "id": record.id })),
        )
    })?;
    let _ = fs::remove_file(&ack_path);
    let ack_socket = UnixDatagram::bind(&ack_path).map_err(|err| {
        CliError::runtime(
            "codex-input-ack-bind-failed",
            format!("failed to bind the Codex manual input acknowledgement: {err}"),
            Some(json!({ "id": record.id })),
        )
    })?;
    ack_socket
        .set_read_timeout(Some(MANUAL_INPUT_ACK_TIMEOUT))
        .map_err(|err| {
            CliError::runtime(
                "codex-input-ack-timeout-failed",
                format!("failed to bound the Codex manual input acknowledgement: {err}"),
                Some(json!({ "id": record.id })),
            )
        })?;
    marker.ack_path = Some(ack_path);
    marker.ack_socket = Some(ack_socket);
    Ok(Some(marker))
}

fn manual_input_request_matches_bound_thread(
    context: &CliContext,
    record: &SessionRecord,
    value: &Value,
) -> bool {
    if value.get("method").and_then(Value::as_str) != Some("turn/start")
        || value.get("id").and_then(json_id_key).is_none()
        || !value.pointer("/params/input").is_some_and(Value::is_array)
    {
        return false;
    }
    let Some(thread_id) = value.pointer("/params/threadId").and_then(Value::as_str) else {
        return false;
    };
    if !protocol_id_is_valid(thread_id) {
        return false;
    }
    let Some(attached) = thread_attached_path(record) else {
        return false;
    };
    if fs::read_to_string(attached).ok().as_deref() != Some(&projected_thread_binding(thread_id)) {
        return false;
    }
    let _ = context;
    true
}

pub(crate) struct ManualInputGate {
    gate_file: fs::File,
    _owner_file: Option<fs::File>,
}

impl Drop for ManualInputGate {
    fn drop(&mut self) {
        unlock_bootstrap_file(&self.gate_file);
    }
}

fn open_manual_input_gate_file(path: &Path) -> Option<fs::File> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(path)
        .ok()
}

fn lock_file_timed(file: &fs::File, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        // SAFETY: `flock` observes the valid descriptor borrowed for this call.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
            return true;
        }
        let error = std::io::Error::last_os_error();
        if error.kind() != std::io::ErrorKind::WouldBlock || Instant::now() >= deadline {
            return false;
        }
        thread::sleep(Duration::from_millis(5));
    }
}

fn marker_has_live_shared_lock(file: &fs::File) -> bool {
    // SAFETY: `flock` observes the valid descriptor borrowed for this call.
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
        unlock_bootstrap_file(file);
        return false;
    }
    std::io::Error::last_os_error().kind() == std::io::ErrorKind::WouldBlock
}

#[cfg(test)]
fn acquire_manual_input_gate(
    context: &CliContext,
    record: &SessionRecord,
    value: &Value,
) -> Option<ManualInputGate> {
    if !manual_input_request_matches_bound_thread(context, record, value) {
        return None;
    }
    let path = manual_input_section_path(context, record);
    let mut owner_file = fs::File::open(&path).ok()?;
    if !marker_has_live_shared_lock(&owner_file) {
        return None;
    }
    let gate_file = open_manual_input_gate_file(&manual_input_gate_path(context, record))?;
    if !lock_file_timed(&gate_file, MANUAL_INPUT_GATE_TIMEOUT) {
        return None;
    }
    let own = owner_file.metadata().ok()?;
    let current = fs::metadata(&path).ok()?;
    if own.dev() != current.dev() || own.ino() != current.ino() {
        return None;
    }
    if !marker_has_live_shared_lock(&owner_file) {
        return None;
    }
    let marker = read_runtime_process_marker_file(&mut owner_file)?;
    if !valid_runtime_process_marker(
        &marker,
        record,
        MANUAL_INPUT_SECTION_VERSION,
        MANUAL_INPUT_SECTION_TTL,
    ) {
        return None;
    }
    UnixDatagram::unbound()
        .ok()?
        .send_to(&[1], manual_input_ack_path(record)?)
        .ok()?;
    Some(ManualInputGate {
        gate_file,
        _owner_file: Some(owner_file),
    })
}

/// Serialize a provider `turn/start` with account mutation. The proxy takes
/// this gate before forwarding and retains it through the matching JSON-RPC
/// response, closing the accepted-but-not-yet-observed turn window. A manual
/// sender marker, when present, is revalidated under the same lock.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TuiMutationRejection {
    TurnAlreadyPending,
    AccountMutationForbidden,
    AccountNotReady,
    TurnRequestInvalid,
    TurnGateOpenFailed,
    TurnGateBusy,
    ManualMarkerThreadMismatch,
    ManualMarkerReplaced,
    ManualMarkerInvalid,
    ManualAckFailed,
    RuntimeIdentityMissing,
    ManualCancellationBusy,
    RuntimeChanged,
    AccountAuthorityUnavailable,
}

impl TuiMutationRejection {
    fn code(self) -> &'static str {
        match self {
            Self::TurnAlreadyPending => "turn_already_pending",
            Self::AccountMutationForbidden => "account_mutation_forbidden",
            Self::AccountNotReady => "account_not_ready",
            Self::TurnRequestInvalid => "turn_request_invalid",
            Self::TurnGateOpenFailed => "turn_gate_open_failed",
            Self::TurnGateBusy => "turn_gate_busy",
            Self::ManualMarkerThreadMismatch => "manual_marker_thread_mismatch",
            Self::ManualMarkerReplaced => "manual_marker_replaced",
            Self::ManualMarkerInvalid => "manual_marker_invalid",
            Self::ManualAckFailed => "manual_ack_failed",
            Self::RuntimeIdentityMissing => "runtime_identity_missing",
            Self::ManualCancellationBusy => "manual_cancellation_busy",
            Self::RuntimeChanged => "runtime_changed",
            Self::AccountAuthorityUnavailable => "account_authority_unavailable",
        }
    }
}

fn acquire_turn_start_gate(
    context: &CliContext,
    record: &SessionRecord,
    value: &Value,
) -> Result<ManualInputGate, TuiMutationRejection> {
    if value.get("method").and_then(Value::as_str) != Some("turn/start")
        || value.get("id").and_then(json_id_key).is_none()
        || !value.pointer("/params/input").is_some_and(Value::is_array)
        || !value
            .pointer("/params/threadId")
            .and_then(Value::as_str)
            .is_some_and(protocol_id_is_valid)
    {
        return Err(TuiMutationRejection::TurnRequestInvalid);
    }
    let gate_file = open_manual_input_gate_file(&manual_input_gate_path(context, record))
        .ok_or(TuiMutationRejection::TurnGateOpenFailed)?;
    // A proxy never waits here: a manual sender may already own the session
    // record lock, and a competing account mutation drops that record lock
    // when its own non-blocking gate attempt loses.
    // SAFETY: `flock` observes the valid descriptor borrowed for this call.
    if unsafe { libc::flock(gate_file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err(TuiMutationRejection::TurnGateBusy);
    }

    let marker_path = manual_input_section_path(context, record);
    let Ok(mut owner_file) = fs::File::open(&marker_path) else {
        return Ok(ManualInputGate {
            gate_file,
            _owner_file: None,
        });
    };
    if !marker_has_live_shared_lock(&owner_file) {
        return Ok(ManualInputGate {
            gate_file,
            _owner_file: None,
        });
    }
    if !manual_input_request_matches_bound_thread(context, record, value) {
        // Keep the rejected TUI's candidate for explicit, provider-verified
        // recovery. This does not change the binding or admit the failed turn.
        if let Some(thread_id) = value.pointer("/params/threadId").and_then(Value::as_str) {
            let _ = crate::conversation::retain_observation(context, record, thread_id);
        }
        return Err(TuiMutationRejection::ManualMarkerThreadMismatch);
    }
    let own = owner_file
        .metadata()
        .map_err(|_| TuiMutationRejection::ManualMarkerReplaced)?;
    let current =
        fs::metadata(&marker_path).map_err(|_| TuiMutationRejection::ManualMarkerReplaced)?;
    if own.dev() != current.dev() || own.ino() != current.ino() {
        return Err(TuiMutationRejection::ManualMarkerReplaced);
    }
    let marker = read_runtime_process_marker_file(&mut owner_file)
        .ok_or(TuiMutationRejection::ManualMarkerInvalid)?;
    if !valid_runtime_process_marker(
        &marker,
        record,
        MANUAL_INPUT_SECTION_VERSION,
        MANUAL_INPUT_SECTION_TTL,
    ) {
        return Err(TuiMutationRejection::ManualMarkerInvalid);
    }
    UnixDatagram::unbound()
        .map_err(|_| TuiMutationRejection::ManualAckFailed)?
        .send_to(
            &[1],
            manual_input_ack_path(record).ok_or(TuiMutationRejection::ManualAckFailed)?,
        )
        .map_err(|_| TuiMutationRejection::ManualAckFailed)?;
    Ok(ManualInputGate {
        gate_file,
        _owner_file: Some(owner_file),
    })
}

/// Hold account mutation behind every provider `turn/start` that has been
/// forwarded but has not received its matching response yet.
pub(crate) fn acquire_account_mutation_gate(
    context: &CliContext,
    id: &str,
) -> Result<ManualInputGate, CliError> {
    crate::validate_id(id)?;
    let path = account_mutation_gate_path(context, id);
    let gate_file = open_manual_input_gate_file(&path).ok_or_else(|| {
        CliError::runtime(
            "codex-account-mutation-gate-unavailable",
            "Codex account mutation gate is unavailable",
            Some(json!({ "id": id })),
        )
    })?;
    // The caller already owns the session-record lock. Never wait here: the
    // proxy acquires this gate before that lock, so a losing account mutation
    // must release the record promptly instead of inverting the order.
    // SAFETY: `flock` observes the valid descriptor borrowed for this call.
    if unsafe { libc::flock(gate_file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err(CliError::runtime(
            "codex-account-session-busy",
            "Codex account mutation is waiting for an in-flight turn submission",
            Some(json!({ "id": id })),
        ));
    }
    let own = gate_file.metadata().map_err(|_| {
        CliError::runtime(
            "codex-account-mutation-gate-unavailable",
            "Codex account mutation gate is unavailable",
            Some(json!({ "id": id })),
        )
    })?;
    let current = fs::metadata(&path).map_err(|_| {
        CliError::runtime(
            "codex-account-mutation-gate-unavailable",
            "Codex account mutation gate is unavailable",
            Some(json!({ "id": id })),
        )
    })?;
    if own.dev() != current.dev() || own.ino() != current.ino() {
        return Err(CliError::runtime(
            "codex-account-mutation-gate-replaced",
            "Codex account mutation gate changed during authorization",
            Some(json!({ "id": id })),
        ));
    }
    Ok(ManualInputGate {
        gate_file,
        _owner_file: None,
    })
}

fn begin_proxy_capability(
    context: &CliContext,
    record: &SessionRecord,
) -> Result<ProxyCapabilityGuard, CliError> {
    let mut marker = write_runtime_process_marker(
        proxy_capability_path(context, record),
        record,
        PROXY_CAPABILITY_VERSION,
        PROXY_CAPABILITY_TTL,
    )?;
    // An unlocked capability file is harmless stale state and may be replaced
    // by the next proxy. Avoid racy pathname cleanup across proxy generations.
    marker.cleanup_on_drop = false;
    Ok(ProxyCapabilityGuard { _marker: marker })
}

fn runtime_path<'a>(record: &'a SessionRecord, key: &str) -> Option<&'a Path> {
    runtime_is_supported(record).then(|| {
        record
            .runtime
            .as_ref()
            .and_then(|runtime| runtime.extra.get(key))
            .and_then(Value::as_str)
            .map(Path::new)
    })?
}

pub(crate) fn thread_handoff_path(record: &SessionRecord) -> Option<&Path> {
    runtime_path(record, THREAD_HANDOFF_KEY)
}

pub(crate) struct CreateBootstrapGuard {
    path: PathBuf,
    file: fs::File,
}

impl Drop for CreateBootstrapGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

impl CreateBootstrapGuard {
    pub(crate) fn finish(self, release_lifecycle_lock: impl FnOnce()) {
        if lock_bootstrap_file(&self.file) {
            release_lifecycle_lock();
            let _ = fs::remove_file(&self.path);
            unlock_bootstrap_file(&self.file);
        } else {
            // Lock failure must sacrifice bootstrap availability, never expose
            // a marker that can authorize arbitrary record-lock contention.
            let _ = fs::remove_file(&self.path);
            release_lifecycle_lock();
        }
    }
}

struct CreateBootstrapGate {
    file: fs::File,
}

impl Drop for CreateBootstrapGate {
    fn drop(&mut self) {
        unlock_bootstrap_file(&self.file);
    }
}

fn lock_bootstrap_file(file: &fs::File) -> bool {
    loop {
        // SAFETY: `flock` observes the valid descriptor borrowed for this call.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } == 0 {
            return true;
        }
        if std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted {
            return false;
        }
    }
}

fn unlock_bootstrap_file(file: &fs::File) {
    // SAFETY: `flock` observes the valid descriptor borrowed for this call.
    unsafe {
        libc::flock(file.as_raw_fd(), libc::LOCK_UN);
    }
}

pub(crate) fn begin_create_bootstrap(
    record: &SessionRecord,
) -> Result<Option<CreateBootstrapGuard>, CliError> {
    if !runtime_is_supported(record) {
        return Ok(None);
    }
    let path = thread_handoff_path(record)
        .map(PathBuf::from)
        .ok_or_else(|| {
            CliError::data(
                "codex-app-server-handoff-missing",
                "Codex app-server runtime is missing its create bootstrap marker",
                Some(json!({ "id": record.id })),
            )
        })?;
    let launch_id = record
        .runtime
        .as_ref()
        .map(|runtime| runtime.launch_id.as_bytes())
        .ok_or_else(|| {
            CliError::data(
                "runtime-id-missing",
                "session runtime is missing its launch metadata",
                Some(json!({ "id": record.id })),
            )
        })?;
    write_private_file(&path, launch_id)?;
    let file = fs::File::open(&path).map_err(|err| {
        CliError::runtime(
            "codex-app-server-handoff-open-failed",
            format!("failed to open the create bootstrap marker: {err}"),
            Some(json!({ "id": record.id })),
        )
    })?;
    Ok(Some(CreateBootstrapGuard { path, file }))
}

fn create_bootstrap_is_live(record: &SessionRecord) -> bool {
    #[cfg(test)]
    BOOTSTRAP_LIVE_CHECKS.with(|checks| checks.set(checks.get() + 1));
    let Some(path) = thread_handoff_path(record) else {
        return false;
    };
    let Some(runtime) = record.runtime.as_ref() else {
        return false;
    };
    fs::read(path).is_ok_and(|bytes| bytes == runtime.launch_id.as_bytes())
}

fn acquire_create_bootstrap_gate(record: &SessionRecord) -> Option<CreateBootstrapGate> {
    let path = thread_handoff_path(record)?;
    let mut file = fs::File::open(path).ok()?;
    #[cfg(test)]
    BOOTSTRAP_GATE_ATTEMPTS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    if !lock_bootstrap_file(&file) {
        return None;
    }
    let own = file.metadata().ok()?;
    let current = fs::metadata(path).ok()?;
    if own.dev() != current.dev() || own.ino() != current.ino() {
        return None;
    }
    let mut token = Vec::new();
    file.read_to_end(&mut token).ok()?;
    let runtime = record.runtime.as_ref()?;
    (token == runtime.launch_id.as_bytes()).then_some(CreateBootstrapGate { file })
}

#[cfg(test)]
std::thread_local! {
    static BOOTSTRAP_LIVE_CHECKS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
fn normal_cancellation_attempts()
-> &'static std::sync::Mutex<std::collections::HashMap<String, usize>> {
    static ATTEMPTS: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<String, usize>>,
    > = std::sync::OnceLock::new();
    ATTEMPTS.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

#[cfg(test)]
static BOOTSTRAP_GATE_ATTEMPTS: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

pub(crate) fn thread_attached_path(record: &SessionRecord) -> Option<&Path> {
    runtime_path(record, THREAD_ATTACHED_KEY)
}

pub(crate) fn socket_path(record: &SessionRecord) -> Option<&str> {
    runtime_is_supported(record).then(|| {
        record
            .runtime
            .as_ref()
            .and_then(|runtime| runtime.extra.get(SOCKET_KEY))
            .and_then(Value::as_str)
    })?
}

pub(crate) fn proxy_path(record: &SessionRecord) -> Option<&Path> {
    runtime_path(record, PROXY_KEY)
}

pub(crate) fn cleanup_runtime_files(
    context: &CliContext,
    record: &SessionRecord,
) -> Result<(), CliError> {
    if !runtime_is_supported(record) {
        return Ok(());
    }
    for path in persisted_runtime_paths(context, record)? {
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => {
                return Err(CliError::runtime(
                    "codex-app-server-cleanup-failed",
                    format!("failed to remove a private Codex runtime file: {err}"),
                    None,
                ));
            }
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StructuredFailureKind {
    Authentication,
    UsageExhausted,
    ProviderCapacity,
}

impl StructuredFailureKind {
    pub(crate) fn activity_reason(self) -> &'static str {
        match self {
            Self::Authentication => "authentication",
            Self::UsageExhausted => "usage_exhausted",
            Self::ProviderCapacity => "provider_capacity",
        }
    }

    fn from_codex_error_info(value: &Value) -> Option<Self> {
        match value.as_str() {
            Some("usageLimitExceeded") => Some(Self::UsageExhausted),
            Some("serverOverloaded") => Some(Self::ProviderCapacity),
            Some("unauthorized") => Some(Self::Authentication),
            _ if [
                "httpConnectionFailed",
                "responseStreamConnectionFailed",
                "responseStreamDisconnected",
                "responseTooManyFailedAttempts",
            ]
            .iter()
            .any(|variant| {
                value
                    .get(variant)
                    .and_then(|info| info.get("httpStatusCode"))
                    .and_then(Value::as_u64)
                    == Some(401)
            }) =>
            {
                Some(Self::Authentication)
            }
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StructuredFailure {
    pub(crate) thread_id: String,
    pub(crate) turn_id: String,
    pub(crate) kind: StructuredFailureKind,
}

#[derive(Debug)]
pub(crate) struct FailureReducer {
    thread_id: String,
    active_turn_id: Option<String>,
    pending_turns: BTreeMap<String, Option<StructuredFailureKind>>,
    pending_order: VecDeque<String>,
    completed_turns: BTreeSet<String>,
    completed_order: VecDeque<String>,
}

impl FailureReducer {
    pub(crate) fn new(thread_id: impl Into<String>) -> Self {
        Self {
            thread_id: thread_id.into(),
            active_turn_id: None,
            pending_turns: BTreeMap::new(),
            pending_order: VecDeque::new(),
            completed_turns: BTreeSet::new(),
            completed_order: VecDeque::new(),
        }
    }

    pub(crate) fn ingest(&mut self, message: &Value) -> Option<StructuredFailure> {
        self.observe_turn_lifecycle(message);
        match message.get("method").and_then(Value::as_str) {
            Some("error") => {
                let params = message.get("params")?;
                if params.get("threadId").and_then(Value::as_str) != Some(self.thread_id.as_str()) {
                    return None;
                }
                let kind = params
                    .pointer("/error/codexErrorInfo")
                    .and_then(StructuredFailureKind::from_codex_error_info)?;
                let turn_id = params
                    .get("turnId")
                    .and_then(Value::as_str)
                    .filter(|turn_id| protocol_id_is_valid(turn_id))?;
                if kind == StructuredFailureKind::Authentication
                    && !self.completed_turns.contains(turn_id)
                {
                    return Some(StructuredFailure {
                        thread_id: self.thread_id.clone(),
                        turn_id: turn_id.into(),
                        kind,
                    });
                }
                if params.get("willRetry").and_then(Value::as_bool) != Some(false) {
                    return None;
                }
                if !self.completed_turns.contains(turn_id) {
                    insert_bounded_failure(
                        &mut self.pending_turns,
                        &mut self.pending_order,
                        turn_id,
                        kind,
                    );
                }
                None
            }
            Some("turn/completed") => {
                let params = message.get("params")?;
                if params.get("threadId").and_then(Value::as_str) != Some(self.thread_id.as_str())
                    || params.pointer("/turn/status").and_then(Value::as_str) != Some("failed")
                {
                    return None;
                }
                let turn_id = params
                    .pointer("/turn/id")
                    .and_then(Value::as_str)
                    .filter(|turn_id| protocol_id_is_valid(turn_id))?;
                let embedded_kind = params
                    .pointer("/turn/error/codexErrorInfo")
                    .and_then(StructuredFailureKind::from_codex_error_info);
                let matched_kind = remove_bounded_failure(
                    &mut self.pending_turns,
                    &mut self.pending_order,
                    turn_id,
                )
                .flatten();
                insert_bounded_id(
                    &mut self.completed_turns,
                    &mut self.completed_order,
                    turn_id,
                );
                let kind = embedded_kind.or(matched_kind);
                kind.map(|kind| StructuredFailure {
                    thread_id: self.thread_id.clone(),
                    turn_id: turn_id.to_string(),
                    kind,
                })
            }
            _ => None,
        }
    }

    fn observe_turn_lifecycle(&mut self, message: &Value) {
        let method = message.get("method").and_then(Value::as_str);
        let thread_id = message.pointer("/params/threadId").and_then(Value::as_str);
        let turn_id = message
            .pointer("/params/turn/id")
            .and_then(Value::as_str)
            .filter(|turn_id| protocol_id_is_valid(turn_id));
        if thread_id != Some(self.thread_id.as_str()) {
            return;
        }
        match (method, turn_id) {
            (Some("turn/started"), Some(turn_id)) => self.note_started(turn_id),
            (Some("turn/completed"), Some(turn_id))
                if self.active_turn_id.as_deref() == Some(turn_id) =>
            {
                self.active_turn_id = None;
            }
            _ => {}
        }
    }

    fn note_started(&mut self, turn_id: &str) {
        if protocol_id_is_valid(turn_id) {
            self.active_turn_id = Some(turn_id.to_string());
        }
    }

    fn raw_turn_for_projection(
        &self,
        runtime_id: &str,
        expected_projected_turn_id: &str,
    ) -> Result<String, String> {
        let raw_turn_id = self
            .active_turn_id
            .as_deref()
            .ok_or_else(|| "Codex active turn identity is unavailable".to_string())?;
        let projected = crate::activity::projected_codex_turn_identifier(runtime_id, raw_turn_id)
            .map_err(|_| "Codex active turn identity is invalid".to_string())?;
        if projected != expected_projected_turn_id {
            return Err("Codex active turn changed before steering".to_string());
        }
        Ok(raw_turn_id.to_string())
    }
}

fn insert_bounded_failure(
    pending: &mut BTreeMap<String, Option<StructuredFailureKind>>,
    order: &mut VecDeque<String>,
    id: &str,
    kind: StructuredFailureKind,
) {
    if let Some(existing) = pending.get_mut(id) {
        if existing.is_some_and(|existing| existing != kind) {
            *existing = None;
        }
        return;
    }
    while pending.len() >= MAX_REDUCER_PENDING_TURNS {
        let Some(oldest) = order.pop_front() else {
            break;
        };
        pending.remove(&oldest);
    }
    pending.insert(id.to_string(), Some(kind));
    order.push_back(id.to_string());
}

fn remove_bounded_failure(
    pending: &mut BTreeMap<String, Option<StructuredFailureKind>>,
    order: &mut VecDeque<String>,
    id: &str,
) -> Option<Option<StructuredFailureKind>> {
    let removed = pending.remove(id);
    if removed.is_some()
        && let Some(index) = order.iter().position(|candidate| candidate == id)
    {
        order.remove(index);
    }
    removed
}

fn insert_bounded_id(set: &mut BTreeSet<String>, order: &mut VecDeque<String>, id: &str) {
    if set.contains(id) {
        return;
    }
    while set.len() >= MAX_REDUCER_PENDING_TURNS {
        let Some(oldest) = order.pop_front() else {
            break;
        };
        set.remove(&oldest);
    }
    let owned = id.to_string();
    set.insert(owned.clone());
    order.push_back(owned);
}

pub(crate) fn initialize_request(id: u64) -> Value {
    json!({
        "id": id,
        "method": "initialize",
        "params": {
            "clientInfo": { "name": "agent-session", "title": "agent-session", "version": env!("CARGO_PKG_VERSION") },
            "capabilities": {
                "experimentalApi": true,
                "requestAttestation": false
            }
        }
    })
}

pub(crate) fn initialized_notification() -> Value {
    json!({ "method": "initialized" })
}

pub(crate) fn loaded_threads_request(id: u64) -> Value {
    json!({ "id": id, "method": "thread/loaded/list", "params": {} })
}

/// Rejoin the bound thread for live control without hydrating its unbounded
/// turn history into a single WebSocket response.
pub(crate) fn resume_thread_request(id: u64, thread_id: &str, cwd: &str) -> Value {
    json!({
        "id": id,
        "method": "thread/resume",
        "params": { "threadId": thread_id, "cwd": cwd, "excludeTurns": true }
    })
}

pub(crate) fn rate_limits_request(id: u64) -> Value {
    json!({ "id": id, "method": "account/rateLimits/read" })
}

pub(crate) fn external_auth_login_request(
    id: u64,
    access_token: &str,
    chatgpt_account_id: &str,
    chatgpt_plan_type: Option<&str>,
) -> Value {
    json!({
        "id": id,
        "method": "account/login/start",
        "params": {
            "type": "chatgptAuthTokens",
            "accessToken": access_token,
            "chatgptAccountId": chatgpt_account_id,
            "chatgptPlanType": chatgpt_plan_type
        }
    })
}

pub(crate) fn external_auth_refresh_response(
    id: Value,
    access_token: &str,
    chatgpt_account_id: &str,
    chatgpt_plan_type: Option<&str>,
) -> Value {
    json!({
        "id": id,
        "result": {
            "accessToken": access_token,
            "chatgptAccountId": chatgpt_account_id,
            "chatgptPlanType": chatgpt_plan_type
        }
    })
}

pub(crate) fn continuation_request(id: u64, thread_id: &str, message: &str) -> Value {
    json!({
        "id": id,
        "method": "turn/start",
        "params": {
            "threadId": thread_id,
            "input": [{ "type": "text", "text": message, "text_elements": [] }]
        }
    })
}

pub(crate) fn steering_request(
    id: u64,
    thread_id: &str,
    expected_turn_id: &str,
    message: &str,
) -> Value {
    json!({
        "id": id,
        "method": "turn/steer",
        "params": {
            "threadId": thread_id,
            "expectedTurnId": expected_turn_id,
            "input": [{ "type": "text", "text": message, "text_elements": [] }]
        }
    })
}

pub(crate) fn latest_turn_request(id: u64, thread_id: &str) -> Value {
    json!({
        "id": id,
        "method": "thread/turns/list",
        "params": {
            "threadId": thread_id,
            "limit": 1,
            "sortDirection": "desc",
            "itemsView": "notLoaded"
        }
    })
}

fn latest_in_progress_turn_id(result: &Value) -> Option<&str> {
    match latest_turn_state(result)? {
        LatestTurnState::InProgress(turn_id) => Some(turn_id),
        LatestTurnState::Idle(_) => None,
    }
}

#[derive(Debug, PartialEq, Eq)]
enum LatestTurnState<'a> {
    Idle(Option<&'a str>),
    InProgress(&'a str),
}

fn latest_turn_state(result: &Value) -> Option<LatestTurnState<'_>> {
    let turns = result.get("data")?.as_array()?;
    let Some(turn) = turns.first() else {
        return Some(LatestTurnState::Idle(None));
    };
    let turn_id = turn
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| protocol_id_is_valid(id))?;
    match turn.get("status").and_then(Value::as_str)? {
        "inProgress" => Some(LatestTurnState::InProgress(turn_id)),
        "completed" | "failed" | "interrupted" => Some(LatestTurnState::Idle(Some(turn_id))),
        _ => None,
    }
}

pub(crate) fn loaded_thread_ids(result: &Value) -> Option<Vec<String>> {
    let data = result.get("data")?.as_array()?;
    data.iter()
        .map(|value| {
            value
                .as_str()
                .filter(|id| protocol_id_is_valid(id))
                .map(str::to_string)
        })
        .collect()
}

fn protocol_id_is_valid(id: &str) -> bool {
    !id.is_empty() && id.len() <= MAX_PROTOCOL_ID_BYTES
}

pub(crate) fn usage_snapshot(result: &Value) -> UsageSnapshot {
    let Some(legacy_snapshot) = result.get("rateLimits").filter(|value| value.is_object()) else {
        return UsageSnapshot {
            authoritative: false,
            has_exhausted_windows: false,
            exhausted_reset_epochs: Vec::new(),
            soonest_reset_epoch: None,
        };
    };
    let mut exhausted_reset_epochs = Vec::new();
    let mut has_exhausted_windows = false;
    let mut observed_window = false;
    let mut snapshots = vec![legacy_snapshot];
    match result.get("rateLimitsByLimitId") {
        None | Some(Value::Null) => {}
        Some(Value::Object(by_limit_id)) => snapshots.extend(by_limit_id.values()),
        Some(_) => {
            return UsageSnapshot {
                authoritative: false,
                has_exhausted_windows: false,
                exhausted_reset_epochs: Vec::new(),
                soonest_reset_epoch: None,
            };
        }
    }
    for snapshot in snapshots {
        if !snapshot.is_object() {
            return UsageSnapshot {
                authoritative: false,
                has_exhausted_windows: false,
                exhausted_reset_epochs: Vec::new(),
                soonest_reset_epoch: None,
            };
        }
        for key in ["primary", "secondary"] {
            let Some(window) = snapshot.get(key).filter(|value| !value.is_null()) else {
                continue;
            };
            let Some(used_percent) = window.get("usedPercent").and_then(Value::as_f64) else {
                return UsageSnapshot {
                    authoritative: false,
                    has_exhausted_windows: false,
                    exhausted_reset_epochs: Vec::new(),
                    soonest_reset_epoch: None,
                };
            };
            observed_window = true;
            if used_percent >= 100.0 {
                has_exhausted_windows = true;
                if let Some(epoch) = window.get("resetsAt").and_then(Value::as_i64) {
                    exhausted_reset_epochs.push(epoch);
                }
            }
        }
    }
    exhausted_reset_epochs.sort_unstable();
    exhausted_reset_epochs.dedup();
    UsageSnapshot {
        authoritative: observed_window,
        has_exhausted_windows,
        exhausted_reset_epochs,
        soonest_reset_epoch: None,
    }
}

#[derive(Clone)]
pub(crate) struct ControlHandle {
    sender: mpsc::Sender<ControlCommand>,
    ready: watch::Receiver<bool>,
}

pub(crate) enum ControlCommand {
    Usage(oneshot::Sender<Result<UsageSnapshot, String>>),
    Prompt {
        message: String,
        response: oneshot::Sender<Result<String, String>>,
    },
    Steer {
        message: String,
        expected_turn_id: String,
        response: oneshot::Sender<Result<String, String>>,
    },
    Continue {
        message: String,
        response: oneshot::Sender<Result<String, String>>,
    },
    BindAccount {
        account: String,
        revision: u64,
        response: oneshot::Sender<Result<crate::codex_account::CodexAccountView, String>>,
    },
    /// Drain and apply a queued next-account intent if the turn is idle. Used by
    /// the periodic idle-boundary drive; a no-op when nothing is drainable.
    ApplyNext {
        response: oneshot::Sender<Result<(), String>>,
    },
}

impl ControlHandle {
    // Registration permits account recovery while startup is still in progress.
    // Prompt callers must wait without holding the record lock needed by thread
    // persistence and the initial usage wakeup.
    pub(crate) async fn wait_ready(&self, timeout: Duration) -> Result<(), String> {
        let mut ready = self.ready.clone();
        tokio::time::timeout(timeout, async {
            loop {
                if *ready.borrow_and_update() {
                    return Ok(());
                }
                ready
                    .changed()
                    .await
                    .map_err(|_| "codex control startup ended".to_string())?;
            }
        })
        .await
        .map_err(|_| "codex control startup timed out".to_string())?
    }

    pub(crate) async fn usage(&self) -> Result<UsageSnapshot, String> {
        let (response, receive) = oneshot::channel();
        tokio::time::timeout(CONTROL_RESPONSE_TIMEOUT, async {
            self.sender
                .send(ControlCommand::Usage(response))
                .await
                .map_err(|_| "codex control connection unavailable".to_string())?;
            receive
                .await
                .map_err(|_| "codex control connection closed".to_string())?
        })
        .await
        .map_err(|_| "codex rate-limit request timed out".to_string())?
    }

    pub(crate) async fn submit(&self, message: &str) -> Result<String, String> {
        let (response, receive) = oneshot::channel();
        tokio::time::timeout(CONTROL_SUBMIT_TOTAL_TIMEOUT, async {
            self.sender
                .send(ControlCommand::Continue {
                    message: message.to_string(),
                    response,
                })
                .await
                .map_err(|_| "codex control connection unavailable".to_string())?;
            receive
                .await
                .map_err(|_| "codex control connection closed".to_string())?
        })
        .await
        .map_err(|_| "codex turn submission timed out".to_string())?
    }

    pub(crate) async fn submit_prompt(&self, message: &str) -> Result<String, String> {
        let (response, receive) = oneshot::channel();
        tokio::time::timeout(CONTROL_SUBMIT_TOTAL_TIMEOUT, async {
            self.sender
                .send(ControlCommand::Prompt {
                    message: message.to_string(),
                    response,
                })
                .await
                .map_err(|_| "codex control connection unavailable".to_string())?;
            receive
                .await
                .map_err(|_| "codex control connection closed".to_string())?
        })
        .await
        .map_err(|_| "codex turn submission timed out".to_string())?
    }

    pub(crate) async fn steer_prompt(
        &self,
        message: &str,
        expected_turn_id: &str,
    ) -> Result<String, String> {
        let (response, receive) = oneshot::channel();
        tokio::time::timeout(
            CONTROL_RESPONSE_TIMEOUT,
            self.sender.send(ControlCommand::Steer {
                message: message.to_string(),
                expected_turn_id: expected_turn_id.to_string(),
                response,
            }),
        )
        .await
        .map_err(|_| "codex turn steering enqueue timed out".to_string())?
        .map_err(|_| "codex control connection unavailable".to_string())?;
        // Once the command is queued, the caller's durable notification fence
        // must remain held until the bounded control handler answers or the
        // connection closes. Timing out here would let a late provider write
        // outlive that fence and overlap a newly admitted atomic operation.
        receive
            .await
            .map_err(|_| "codex control connection closed".to_string())?
    }

    pub(crate) async fn bind_account(
        &self,
        account: &str,
        revision: u64,
    ) -> Result<crate::codex_account::CodexAccountView, String> {
        let (response, receive) = oneshot::channel();
        tokio::time::timeout(CONTROL_SUBMIT_TOTAL_TIMEOUT, async {
            self.sender
                .send(ControlCommand::BindAccount {
                    account: account.to_string(),
                    revision,
                    response,
                })
                .await
                .map_err(|_| "codex control connection unavailable".to_string())?;
            receive
                .await
                .map_err(|_| "codex control connection closed".to_string())?
        })
        .await
        .map_err(|_| "Codex account binding timed out".to_string())?
    }

    pub(crate) async fn apply_next(&self) -> Result<(), String> {
        let (response, receive) = oneshot::channel();
        tokio::time::timeout(CONTROL_SUBMIT_TOTAL_TIMEOUT, async {
            self.sender
                .send(ControlCommand::ApplyNext { response })
                .await
                .map_err(|_| "codex control connection unavailable".to_string())?;
            receive
                .await
                .map_err(|_| "codex control connection closed".to_string())?
        })
        .await
        .map_err(|_| "Codex next-account apply timed out".to_string())?
    }
}

pub(crate) fn starting_control_channel() -> (
    ControlHandle,
    mpsc::Receiver<ControlCommand>,
    watch::Sender<bool>,
) {
    let (sender, receive) = mpsc::channel(4);
    let (ready, readiness) = watch::channel(false);
    (
        ControlHandle {
            sender,
            ready: readiness,
        },
        receive,
        ready,
    )
}

// Fixture command receivers have no startup work to complete.
#[cfg(test)]
pub(crate) fn control_channel() -> (ControlHandle, mpsc::Receiver<ControlCommand>) {
    let (handle, commands, ready) = starting_control_channel();
    ready.send_replace(true);
    (handle, commands)
}

/// Resolve credentials and drive the app-server external-auth login for
/// `account`. Mutates only the live app-server; never touches durable binding
/// or next-intent state. Returns a fail-closed reason code on failure.
async fn drive_external_auth_login(
    websocket: &mut tokio_tungstenite::WebSocketStream<UnixStream>,
    request_id: &mut u64,
    account: &str,
) -> Result<(), &'static str> {
    let resolve_account = account.to_string();
    let credentials = tokio::task::spawn_blocking(move || {
        crate::codex_account::resolve_account(&resolve_account, false)
    })
    .await
    .map_err(|_| "broker_failed")?
    .map_err(|_| "broker_failed")?;

    *request_id = request_id.saturating_add(1);
    if send_json(
        websocket,
        external_auth_login_request(
            *request_id,
            &credentials.access_token,
            &credentials.chatgpt_account_id,
            credentials.chatgpt_plan_type.as_deref(),
        ),
    )
    .await
    .is_err()
    {
        return Err("apply_failed");
    }
    let result =
        receive_response_with_timeout(websocket, *request_id, None, None, CONTROL_RESPONSE_TIMEOUT)
            .await;
    if !result
        .as_ref()
        .is_ok_and(|result| result.get("type").and_then(Value::as_str) == Some("chatgptAuthTokens"))
    {
        return Err("apply_failed");
    }
    Ok(())
}

async fn apply_account_binding(
    websocket: &mut tokio_tungstenite::WebSocketStream<UnixStream>,
    context: &CliContext,
    record: &SessionRecord,
    request_id: &mut u64,
    account: &str,
    revision: u64,
) -> Result<crate::codex_account::CodexAccountView, String> {
    let launch_id = record
        .runtime
        .as_ref()
        .map(|runtime| runtime.launch_id.clone())
        .ok_or_else(|| "Codex runtime identity is missing".to_string())?;
    match drive_external_auth_login(websocket, request_id, account).await {
        Ok(()) => {
            finish_account_binding(context, record, &launch_id, account, revision, Ok(())).await
        }
        Err(reason) => {
            let _ =
                finish_account_binding(context, record, &launch_id, account, revision, Err(reason))
                    .await;
            Err(format!("Codex external-auth login failed: {reason}"))
        }
    }
}

/// At the idle boundary, apply a queued next-account intent before the next
/// prompt: transition it to `applying`, drive the app-server login, and record
/// success (which flips the applied binding and clears the intent) or failure
/// (which marks the intent failed and keeps the prompt fenced). Returns the
/// account now applied to the live runtime, if it changed. The control owner
/// must verify the bound thread is idle immediately before calling this.
async fn apply_pending_next_account_at_idle(
    websocket: &mut tokio_tungstenite::WebSocketStream<UnixStream>,
    context: &CliContext,
    record: &SessionRecord,
    request_id: &mut u64,
    expected_auto_failover: Option<&crate::codex_account::NextAccountIdentity>,
) -> Option<String> {
    let launch_id = record
        .runtime
        .as_ref()
        .map(|runtime| runtime.launch_id.clone())?;
    let begin_context = context.clone();
    let begin_id = record.id.clone();
    let begin_launch = launch_id.clone();
    let expected_auto_failover = expected_auto_failover.cloned();
    let queued = tokio::task::spawn_blocking(move || match expected_auto_failover.as_ref() {
        Some(expected) => crate::codex_account::begin_next_apply_if_unchanged(
            &begin_context,
            &begin_id,
            &begin_launch,
            expected,
        ),
        None => crate::codex_account::begin_next_apply(&begin_context, &begin_id, &begin_launch),
    })
    .await
    .ok()?;
    let next = match queued {
        Ok(Some(next)) => next,
        Ok(None) => return None,
        Err(_) => return None, // malformed intent stays fenced for explicit repair
    };
    let intent_id = next.intent_id?;
    let account = next.account;
    let revision = next.revision;

    let outcome = drive_external_auth_login(websocket, request_id, &account).await;
    let succeeded = outcome.is_ok();
    let finish_context = context.clone();
    let finish_id = record.id.clone();
    let finish_launch = launch_id.clone();
    let finish_account = account.clone();
    let finished = tokio::task::spawn_blocking(move || {
        crate::codex_account::finish_next_apply(
            &finish_context,
            &finish_id,
            &finish_launch,
            &finish_account,
            revision,
            &intent_id,
            outcome,
        )
    })
    .await;
    match finished {
        Ok(Ok(_)) if succeeded => Some(account),
        _ => None,
    }
}

async fn finish_account_binding(
    context: &CliContext,
    record: &SessionRecord,
    launch_id: &str,
    account: &str,
    revision: u64,
    result: Result<(), &'static str>,
) -> Result<crate::codex_account::CodexAccountView, String> {
    let finish_context = context.clone();
    let finish_id = record.id.clone();
    let finish_launch_id = launch_id.to_string();
    let finish_account = account.to_string();
    tokio::task::spawn_blocking(move || {
        crate::codex_account::finish_binding(
            &finish_context,
            &finish_id,
            &finish_launch_id,
            &finish_account,
            revision,
            result,
        )
    })
    .await
    .map_err(|_| "Codex account binding worker failed".to_string())?
    .map_err(|err| format!("Codex account binding persistence failed: {}", err.code()))
}

async fn restore_account_binding_after_refresh_failure(
    context: &CliContext,
    record: &SessionRecord,
    launch_id: &str,
    account: &str,
    attempt: crate::codex_account::RefreshBindingAttempt,
) -> Result<crate::codex_account::CodexAccountView, String> {
    let restore_context = context.clone();
    let restore_id = record.id.clone();
    let restore_launch_id = launch_id.to_string();
    let restore_account = account.to_string();
    tokio::task::spawn_blocking(move || {
        crate::codex_account::restore_binding_after_refresh_failure(
            &restore_context,
            &restore_id,
            &restore_launch_id,
            &restore_account,
            attempt,
        )
    })
    .await
    .map_err(|_| "Codex account refresh recovery worker failed".to_string())?
    .map_err(|err| format!("Codex account refresh recovery failed: {}", err.code()))
}

async fn control_account_ready(
    context: &CliContext,
    record: &SessionRecord,
    external_auth_account: Option<&str>,
) -> Result<(), String> {
    control_account_ready_with(context, record, external_auth_account, false).await
}

async fn control_terminal_account_ready(
    context: &CliContext,
    record: &SessionRecord,
    external_auth_account: Option<&str>,
) -> Result<(), String> {
    control_account_ready_with(context, record, external_auth_account, true).await
}

async fn control_account_ready_with(
    context: &CliContext,
    record: &SessionRecord,
    external_auth_account: Option<&str>,
    allow_pending_next: bool,
) -> Result<(), String> {
    let check_context = context.clone();
    let id = record.id.clone();
    let launch_id = record
        .runtime
        .as_ref()
        .map(|runtime| runtime.launch_id.clone())
        .ok_or_else(|| "Codex runtime identity is missing".to_string())?;
    let selected = tokio::task::spawn_blocking(move || {
        let current = crate::load_session_record(&check_context, &id)?;
        if current
            .runtime
            .as_ref()
            .is_none_or(|runtime| runtime.launch_id != launch_id)
        {
            return Err(CliError::runtime(
                "codex-account-runtime-changed",
                "Codex session runtime changed while checking its account binding",
                Some(json!({ "id": current.id })),
            ));
        }
        if allow_pending_next {
            crate::codex_account::ensure_terminal_input_allowed(&current)?;
        } else {
            crate::codex_account::ensure_input_allowed(&current)?;
        }
        Ok::<_, CliError>(crate::codex_account::selected_account(&current))
    })
    .await
    .map_err(|_| "Codex account binding worker failed".to_string())?
    .map_err(|err| format!("Codex account binding is not ready: {}", err.code()))?;
    if selected.as_deref() == external_auth_account
        || (selected.is_none() && external_auth_account.is_none())
    {
        Ok(())
    } else {
        Err("Codex account binding is not ready".to_string())
    }
}

async fn automatic_failover_has_authoritative_idle(
    context: &CliContext,
    record: &SessionRecord,
) -> Option<crate::codex_account::NextAccountIdentity> {
    let context = context.clone();
    let id = record.id.clone();
    let expected_launch_id = record
        .runtime
        .as_ref()
        .map(|runtime| runtime.launch_id.clone());
    tokio::task::spawn_blocking(move || {
        let current = crate::load_session_record(&context, &id).ok()?;
        if current
            .runtime
            .as_ref()
            .map(|runtime| runtime.launch_id.as_str())
            != expected_launch_id.as_deref()
        {
            return None;
        }
        let identity = crate::codex_account::pending_auto_failover_apply(&current).ok()??;
        crate::auto_resume::has_authoritative_usage_exhaustion_idle(&context, &current)
            .then_some(identity)
    })
    .await
    .ok()
    .flatten()
}

pub(crate) async fn run_control(
    context: CliContext,
    record: SessionRecord,
    mut commands: mpsc::Receiver<ControlCommand>,
    ready: watch::Sender<bool>,
) -> Result<(), String> {
    let socket = socket_path(&record)
        .map(PathBuf::from)
        .ok_or_else(|| "Codex app-server socket metadata is missing".to_string())?;
    let stream = connect_socket(&socket).await?;
    let (mut websocket, _) = tokio::time::timeout(
        CONTROL_RESPONSE_TIMEOUT,
        tokio_tungstenite::client_async("ws://localhost", stream),
    )
    .await
    .map_err(|_| "Codex app-server WebSocket handshake timed out".to_string())?
    .map_err(|err| format!("Codex app-server WebSocket handshake failed: {err}"))?;

    let mut request_id = 1_u64;
    send_json(&mut websocket, initialize_request(request_id)).await?;
    receive_response_with_timeout(
        &mut websocket,
        request_id,
        None,
        None,
        CONTROL_RESPONSE_TIMEOUT,
    )
    .await
    .map_err(|err| format!("initialize failed: {err}"))?;
    send_json(&mut websocket, initialized_notification()).await?;
    let mut external_auth_account = None;
    if let Some((account, revision)) = crate::codex_account::account_for_control_rebind(&record)
        .map_err(|err| format!("Codex account binding is invalid: {}", err.code()))?
    {
        match apply_account_binding(
            &mut websocket,
            &context,
            &record,
            &mut request_id,
            &account,
            revision,
        )
        .await
        {
            Ok(_) => external_auth_account = Some(account),
            Err(error) => eprintln!("warning: Codex account binding failed: {error}"),
        }
    }
    if crate::codex_account::binding_is_present(&record) && external_auth_account.is_none() {
        loop {
            let command = tokio::select! {
                command = commands.recv() => command,
                message = websocket.next() => {
                    let value = decode_message(message).await?;
                    if respond_to_external_auth_refresh(&mut websocket, &value, None).await? {
                        continue;
                    }
                    continue;
                }
            };
            let Some(command) = command else {
                return Ok(());
            };
            match command {
                ControlCommand::BindAccount {
                    account,
                    revision,
                    response,
                } => {
                    let result = apply_account_binding(
                        &mut websocket,
                        &context,
                        &record,
                        &mut request_id,
                        &account,
                        revision,
                    )
                    .await;
                    match result {
                        Ok(view) => {
                            external_auth_account = Some(account);
                            let _ = response.send(Ok(view));
                            break;
                        }
                        Err(error) => {
                            let _ = response.send(Err(error));
                        }
                    }
                }
                ControlCommand::Usage(response) => {
                    let _ = response.send(Err(
                        "Codex account binding is not ready; retry the account switch".to_string(),
                    ));
                }
                ControlCommand::Prompt { response, .. }
                | ControlCommand::Steer { response, .. }
                | ControlCommand::Continue { response, .. } => {
                    let _ = response.send(Err(
                        "Codex account binding is not ready; retry the account switch".to_string(),
                    ));
                }
                ControlCommand::ApplyNext { response } => {
                    let _ = response.send(Err(
                        "Codex account binding is not ready; retry the account switch".to_string(),
                    ));
                }
            }
        }
    }

    let mut discovery_attempts = 0_u8;
    let mut thread_id = loop {
        request_id = request_id.saturating_add(1);
        send_json(&mut websocket, loaded_threads_request(request_id)).await?;
        let result = receive_response_with_timeout(
            &mut websocket,
            request_id,
            None,
            external_auth_account
                .as_deref()
                .map(|account| (&context, &record, account)),
            CONTROL_RESPONSE_TIMEOUT,
        )
        .await
        .map_err(|err| format!("thread/loaded/list failed: {err}"))?;
        let ids = loaded_thread_ids(&result)
            .ok_or_else(|| "Codex loaded-thread response was malformed".to_string())?;
        if let Some(id) = attached_loaded_thread(&record, &ids)? {
            break id;
        }
        match ids.as_slice() {
            [id] => break id.clone(),
            _ if discovery_attempts < 100 => {
                discovery_attempts = discovery_attempts.saturating_add(1);
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            [] => return Err("Codex TUI did not create a loaded thread".to_string()),
            _ => return Err("Codex app-server exposed more than one loaded thread".to_string()),
        }
    };
    let reconnecting = thread_attached_path(&record).is_some_and(Path::is_file);
    bind_thread_and_persist_resume(&context, &record, &thread_id).await?;
    let mut thread_resumed = false;
    if reconnecting {
        request_id = request_id.saturating_add(1);
        send_json(
            &mut websocket,
            resume_thread_request(request_id, &thread_id, &record.cwd),
        )
        .await?;
        match receive_response_with_timeout(
            &mut websocket,
            request_id,
            None,
            external_auth_account
                .as_deref()
                .map(|account| (&context, &record, account)),
            CONTROL_RESPONSE_TIMEOUT,
        )
        .await
        {
            Ok(_) => thread_resumed = true,
            Err(err) if err.ends_with("(no_rollout)") => {}
            Err(err) => return Err(format!("thread/resume failed: {err}")),
        }
    }
    let mut reducer = FailureReducer::new(thread_id.clone());

    // A daemon may reconnect after an earned/manual provider reset moved the
    // account ahead of the reset epoch captured in durable state. Re-read the
    // exact bound account once so that an existing scheduled claim becomes due
    // without waiting for a notification that happened while disconnected.
    if control_account_ready(&context, &record, external_auth_account.as_deref())
        .await
        .is_ok()
    {
        request_id = request_id.saturating_add(1);
        send_json(&mut websocket, rate_limits_request(request_id)).await?;
        let initial_usage = receive_response_with_timeout(
            &mut websocket,
            request_id,
            Some((&context, &record, &mut reducer)),
            external_auth_account
                .as_deref()
                .map(|account| (&context, &record, account)),
            CONTROL_RESPONSE_TIMEOUT,
        )
        .await
        .map(|value| usage_snapshot(&value))?;
        wake_from_open_usage(&context, &record, &initial_usage).await?;
    }

    ready.send_replace(true);

    loop {
        tokio::select! {
            command = commands.recv() => {
                let Some(command) = command else { return Ok(()); };
                let current = crate::load_session_record(&context, &record.id).map_err(|err| format!("conversation binding read failed: {}", err.code()))?;
                crate::ensure_same_session_identity(&record, &current).map_err(|_| "conversation runtime changed".to_string())?;
                if let Some(resume) = current.provider_resume.as_ref() && resume.session_id != thread_id {
                    thread_id = resume.session_id.clone();
                    reducer = FailureReducer::new(thread_id.clone());
                    thread_resumed = true;
                }
                match command {
                    ControlCommand::Usage(response) => {
                        if let Err(error) = control_account_ready(
                            &context,
                            &record,
                            external_auth_account.as_deref(),
                        )
                        .await
                        {
                            let _ = response.send(Err(error));
                            continue;
                        }
                        request_id = request_id.saturating_add(1);
                        if let Err(err) = send_json(&mut websocket, rate_limits_request(request_id)).await {
                            let _ = response.send(Err(err));
                            return Err("Codex usage request write failed".to_string());
                        }
                        let result = receive_response_with_timeout(
                            &mut websocket,
                            request_id,
                            Some((&context, &record, &mut reducer)),
                            external_auth_account
                                .as_deref()
                                .map(|account| (&context, &record, account)),
                            CONTROL_RESPONSE_TIMEOUT,
                        ).await;
                        let _ = response.send(result.map(|value| usage_snapshot(&value)));
                    }
                    ControlCommand::Prompt { message, response } => {
                        if let Err(error) = control_account_ready(
                            &context,
                            &record,
                            external_auth_account.as_deref(),
                        )
                        .await
                        {
                            let _ = response.send(Err(error));
                            continue;
                        }
                        request_id = request_id.saturating_add(1);
                        if let Err(err) = send_json(
                            &mut websocket,
                            continuation_request(request_id, &thread_id, &message),
                        ).await {
                            let _ = response.send(Err(err));
                            return Err("Codex prompt request write failed".to_string());
                        }
                        let result = receive_response_with_timeout(
                            &mut websocket,
                            request_id,
                            Some((&context, &record, &mut reducer)),
                            external_auth_account
                                .as_deref()
                                .map(|account| (&context, &record, account)),
                            CONTROL_SUBMISSION_TIMEOUT,
                        ).await.and_then(|value| {
                            value.pointer("/turn/id")
                                .and_then(Value::as_str)
                                .map(str::to_string)
                                .ok_or_else(|| "Codex turn/start response omitted the acknowledged turn id".to_string())
                        });
                        if let Ok(turn_id) = result.as_deref() {
                            reducer.note_started(turn_id);
                        }
                        let _ = response.send(result);
                    }
                    ControlCommand::Steer {
                        message,
                        expected_turn_id,
                        response,
                    } => {
                        if let Err(error) = control_terminal_account_ready(
                            &context,
                            &record,
                            external_auth_account.as_deref(),
                        )
                        .await
                        {
                            let _ = response.send(Err(error));
                            continue;
                        }
                        if reducer.active_turn_id.is_none() {
                            request_id = request_id.saturating_add(1);
                            if let Err(error) = send_json(
                                &mut websocket,
                                latest_turn_request(request_id, &thread_id),
                            )
                            .await
                            {
                                let _ = response.send(Err(error));
                                return Err("Codex active turn recovery write failed".to_string());
                            }
                            let recovered = receive_response_with_timeout(
                                &mut websocket,
                                request_id,
                                Some((&context, &record, &mut reducer)),
                                external_auth_account
                                    .as_deref()
                                    .map(|account| (&context, &record, account)),
                                CONTROL_RESPONSE_TIMEOUT,
                            )
                            .await
                            .and_then(|value| {
                                latest_in_progress_turn_id(&value)
                                    .map(str::to_string)
                                    .ok_or_else(|| {
                                        "Codex active turn identity is unavailable".to_string()
                                    })
                            });
                            match recovered {
                                Ok(turn_id) => reducer.note_started(&turn_id),
                                Err(error) => {
                                    let _ = response.send(Err(error));
                                    continue;
                                }
                            }
                        }
                        let runtime_id = record
                            .runtime
                            .as_ref()
                            .map(|runtime| runtime.launch_id.as_str())
                            .ok_or_else(|| "Codex runtime identity is missing".to_string())?;
                        let raw_turn_id = match reducer
                            .raw_turn_for_projection(runtime_id, &expected_turn_id)
                        {
                            Ok(turn_id) => turn_id,
                            Err(error) => {
                                let _ = response.send(Err(error));
                                continue;
                            }
                        };
                        request_id = request_id.saturating_add(1);
                        if let Err(err) = send_json(
                            &mut websocket,
                            steering_request(
                                request_id,
                                &thread_id,
                                &raw_turn_id,
                                &message,
                            ),
                        )
                        .await
                        {
                            let _ = response.send(Err(err));
                            return Err("Codex turn steering request write failed".to_string());
                        }
                        let result = receive_response_with_timeout(
                            &mut websocket,
                            request_id,
                            Some((&context, &record, &mut reducer)),
                            external_auth_account
                                .as_deref()
                                .map(|account| (&context, &record, account)),
                            CONTROL_SUBMISSION_TIMEOUT,
                        )
                        .await
                        .and_then(|value| {
                            let acknowledged = value
                                .get("turnId")
                                .and_then(Value::as_str)
                                .ok_or_else(|| {
                                    "Codex turn/steer response omitted the acknowledged turn id"
                                        .to_string()
                                })?;
                            if acknowledged != raw_turn_id {
                                return Err(
                                    "Codex turn/steer response acknowledged a different turn id"
                                        .to_string(),
                                );
                            }
                            Ok(expected_turn_id)
                        });
                        let _ = response.send(result);
                    }
                    ControlCommand::Continue { message, response } => {
                        if let Err(error) = control_account_ready(
                            &context,
                            &record,
                            external_auth_account.as_deref(),
                        )
                        .await
                        {
                            let _ = response.send(Err(error));
                            continue;
                        }
                        if !thread_resumed {
                            request_id = request_id.saturating_add(1);
                            if let Err(err) = send_json(
                                &mut websocket,
                                resume_thread_request(request_id, &thread_id, &record.cwd),
                            ).await {
                                let _ = response.send(Err(err));
                                return Err("Codex continuation resume write failed".to_string());
                            }
                            if let Err(err) = receive_response_with_timeout(
                                &mut websocket,
                                request_id,
                                Some((&context, &record, &mut reducer)),
                                external_auth_account
                                    .as_deref()
                                    .map(|account| (&context, &record, account)),
                                CONTROL_RESPONSE_TIMEOUT,
                            ).await {
                                let _ = response.send(Err(err));
                                continue;
                            }
                            thread_resumed = true;
                        }
                        request_id = request_id.saturating_add(1);
                        if let Err(err) = send_json(
                            &mut websocket,
                            continuation_request(request_id, &thread_id, &message),
                        ).await {
                            let _ = response.send(Err(err));
                            return Err("Codex continuation request write failed".to_string());
                        }
                        let result = receive_response_with_timeout(
                            &mut websocket,
                            request_id,
                            Some((&context, &record, &mut reducer)),
                            external_auth_account
                                .as_deref()
                                .map(|account| (&context, &record, account)),
                            CONTROL_SUBMISSION_TIMEOUT,
                        ).await.and_then(|value| {
                            value.pointer("/turn/id")
                                .and_then(Value::as_str)
                                .map(str::to_string)
                                .ok_or_else(|| "Codex turn/start response omitted the acknowledged turn id".to_string())
                        });
                        if let Ok(turn_id) = result.as_deref() {
                            reducer.note_started(turn_id);
                        }
                        let _ = response.send(result);
                    }
                    ControlCommand::BindAccount { account, revision, response } => {
                        external_auth_account = None;
                        let result = apply_account_binding(
                            &mut websocket,
                            &context,
                            &record,
                            &mut request_id,
                            &account,
                            revision,
                        )
                        .await;
                        if result.is_ok() {
                            external_auth_account = Some(account);
                        }
                        let _ = response.send(result);
                    }
                    ControlCommand::ApplyNext { response } => {
                        // A structured `usageLimitExceeded` completion is an
                        // authoritative terminal boundary. Codex may stop
                        // answering `thread/turns/list` after that workspace
                        // rejection, so an auto-failover intent must be able to
                        // drain from the durable terminal evidence. All manual
                        // next-account intents retain the live idle probe and
                        // its interleaved-turn race protection.
                        let terminal_auto_failover =
                            automatic_failover_has_authoritative_idle(&context, &record).await;
                        let idle = if terminal_auto_failover.is_some() {
                            true
                        } else {
                            request_id = request_id.saturating_add(1);
                            if let Err(error) = send_json(
                                &mut websocket,
                                latest_turn_request(request_id, &thread_id),
                            )
                            .await
                            {
                                let _ = response.send(Err(error));
                                return Err("Codex idle-boundary read failed".to_string());
                            }
                            let latest = receive_response_with_timeout(
                                &mut websocket,
                                request_id,
                                Some((&context, &record, &mut reducer)),
                                external_auth_account
                                    .as_deref()
                                    .map(|account| (&context, &record, account)),
                                CONTROL_RESPONSE_TIMEOUT,
                            )
                            .await;
                            match latest {
                                Ok(result) => match latest_turn_state(&result) {
                                    Some(LatestTurnState::Idle(latest_turn_id)) => {
                                        // A missed completion leaves the reducer active even
                                        // after the provider has finished that exact turn. A
                                        // different active id means a new turn raced the probe.
                                        match reducer.active_turn_id.as_deref() {
                                            Some(active) if Some(active) != latest_turn_id => false,
                                            _ => {
                                                reducer.active_turn_id = None;
                                                true
                                            }
                                        }
                                    }
                                    Some(LatestTurnState::InProgress(turn_id)) => {
                                        reducer.note_started(turn_id);
                                        false
                                    }
                                    None => false,
                                },
                                Err(error) => {
                                    let _ = response.send(Err(error));
                                    continue;
                                }
                            }
                        };
                        if idle
                            && let Some(applied) = apply_pending_next_account_at_idle(
                                &mut websocket,
                                &context,
                                &record,
                                &mut request_id,
                                terminal_auto_failover.as_ref(),
                            )
                            .await
                        {
                            external_auth_account = Some(applied);
                        }
                        let _ = response.send(Ok(()));
                    }
                }
            }
            message = websocket.next() => {
                let value = decode_message(message).await?;
                if respond_to_external_auth_refresh(
                    &mut websocket,
                    &value,
                    external_auth_account
                        .as_deref()
                        .map(|account| (&context, &record, account)),
                )
                .await?
                {
                    continue;
                }
                if value.pointer("/params/threadId").and_then(Value::as_str).is_some_and(|id| id != thread_id) {
                    let current = crate::load_session_record(&context, &record.id).map_err(|err| format!("conversation binding read failed: {}", err.code()))?;
                    crate::ensure_same_session_identity(&record, &current).map_err(|_| "conversation runtime changed".to_string())?;
                    if let Some(resume) = current.provider_resume.as_ref() && resume.session_id != thread_id {
                        thread_id = resume.session_id.clone();
                        reducer = FailureReducer::new(thread_id.clone());
                        thread_resumed = true;
                    }
                }
                process_live_message(&context, &record, &mut reducer, None, &value).await?;
            }
        }
    }
}

fn bind_thread(record: &SessionRecord, thread_id: &str) -> Result<(), String> {
    let attached = thread_attached_path(record)
        .ok_or_else(|| "Codex attached marker metadata is missing".to_string())?;
    if attached.is_file() {
        let observed = fs::read_to_string(attached)
            .map_err(|_| "Codex attached thread binding was unreadable".to_string())?;
        return (observed == projected_thread_binding(thread_id))
            .then_some(())
            .ok_or_else(|| "Codex loaded thread did not match the attached runtime".to_string());
    }
    write_private_file(attached, projected_thread_binding(thread_id).as_bytes())
        .map_err(|err| format!("Codex thread binding failed: {}", err.code()))
}

pub(crate) fn replace_thread_binding(
    record: &SessionRecord,
    thread_id: &str,
) -> Result<(), CliError> {
    let path = thread_attached_path(record).ok_or_else(|| {
        CliError::data(
            "conversation-binding-missing",
            "Codex attached binding is missing",
            None,
        )
    })?;
    write_private_file(path, projected_thread_binding(thread_id).as_bytes())
}

/// Resolve only an unambiguous loaded primary conversation and require the
/// provider's own idle status. Never infer the active thread from history order.
pub(crate) fn probe_idle_conversation(
    context: &CliContext,
    record: &SessionRecord,
    candidate: Option<&str>,
) -> Result<String, CliError> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| {
            CliError::runtime(
                "conversation-probe-unavailable",
                "provider probe runtime is unavailable",
                None,
            )
        })?;
    runtime.block_on(async {
        let socket = socket_path(record).ok_or_else(|| "managed Codex app-server is unavailable".to_string())?;
        let stream = connect_socket(Path::new(socket)).await?;
        let (mut websocket, _) = tokio::time::timeout(CONTROL_RESPONSE_TIMEOUT, tokio_tungstenite::client_async("ws://localhost", stream))
            .await.map_err(|_| "provider handshake timed out".to_string())?
            .map_err(|_| "provider handshake failed".to_string())?;
        send_json(&mut websocket, initialize_request(1)).await?;
        receive_response_with_timeout(&mut websocket, 1, None, None, CONTROL_RESPONSE_TIMEOUT).await?;
        send_json(&mut websocket, initialized_notification()).await?;
        send_json(&mut websocket, loaded_threads_request(2)).await?;
        let result = receive_response_with_timeout(&mut websocket, 2, None, None, CONTROL_RESPONSE_TIMEOUT).await?;
        if result.get("nextCursor").is_some_and(|cursor| !cursor.is_null()) {
            return Err("loaded conversation identities are incomplete".to_string());
        }
        let ids = loaded_thread_ids(&result).ok_or_else(|| "loaded conversation identities were invalid".to_string())?;
        let mut primary = Vec::new();
        for id in ids {
            if !system_ephemeral_raw_session_is_registered(context, record, &id).map_err(|_| "auxiliary identity classification failed".to_string())? { primary.push(id); }
        }
        let thread_id = match candidate {
            Some(id) if primary.iter().any(|value| value == id) => id.to_string(),
            None if primary.len() == 1 => primary.remove(0),
            _ => return Err("live primary conversation cannot be identified unambiguously".to_string()),
        };
        send_json(&mut websocket, json!({"id":3,"method":"thread/read","params":{"threadId":thread_id,"includeTurns":false}})).await?;
        let result = receive_response_with_timeout(&mut websocket, 3, None, None, CONTROL_RESPONSE_TIMEOUT).await?;
        if result.pointer("/thread/id").and_then(Value::as_str) != Some(thread_id.as_str())
            || result.pointer("/thread/status/type").and_then(Value::as_str) != Some("idle") {
            return Err("live provider conversation is not verified idle".to_string());
        }
        Ok(thread_id)
    }).map_err(|_| CliError::runtime("conversation-live-identity-unverified", "live Codex conversation is busy, ambiguous, or unavailable; no binding was changed", Some(json!({"id":record.id}))))
}

async fn bind_thread_and_persist_resume(
    context: &CliContext,
    record: &SessionRecord,
    thread_id: &str,
) -> Result<(), String> {
    let context = context.clone();
    let record = record.clone();
    let thread_id = thread_id.to_string();
    tokio::task::spawn_blocking(move || persist_bound_thread_resume(&context, &record, &thread_id))
        .await
        .map_err(|error| format!("Codex provider resume persistence worker failed: {error}"))?
        .map_err(|error| format!("Codex provider resume persistence failed: {}", error.code()))
}

fn persist_bound_thread_resume(
    context: &CliContext,
    expected: &SessionRecord,
    thread_id: &str,
) -> Result<(), CliError> {
    if !protocol_id_is_valid(thread_id) {
        return Err(CliError::data(
            "codex-app-server-resume-identity-invalid",
            "Codex app-server returned an invalid thread identity",
            Some(json!({ "id": expected.id })),
        ));
    }
    let resume_args = canonical_provider_resume_args(AgentKind::Codex, &expected.cwd, thread_id)
        .expect("Codex resume arguments are supported");
    let captured_at = Timestamp::now().to_string();
    mutate_session_record(context, &expected.id, |current| {
        crate::ensure_same_session_identity(expected, current)?;
        if current.agent != AgentKind::Codex.as_str() || current.cwd != expected.cwd {
            return Err(CliError::data(
                "codex-app-server-resume-runtime-conflict",
                "Codex app-server runtime no longer matches the session",
                Some(json!({ "id": current.id })),
            ));
        }
        let has_matching_resume = if let Some(existing) = current.provider_resume.as_ref() {
            if existing.provider != AgentKind::Codex.as_str() || existing.session_id != thread_id {
                return Err(CliError::data(
                    "codex-app-server-resume-identity-conflict",
                    "Codex app-server thread conflicts with durable resume metadata",
                    Some(json!({
                        "id": current.id,
                        "provider": existing.provider,
                    })),
                ));
            }
            true
        } else {
            false
        };
        bind_thread(current, thread_id).map_err(|message| {
            CliError::runtime(
                "codex-app-server-thread-binding-failed",
                message,
                Some(json!({ "id": current.id })),
            )
        })?;
        if has_matching_resume {
            return Ok(());
        }
        current.provider_resume = Some(ProviderResume {
            provider: AgentKind::Codex.as_str().to_string(),
            session_id: thread_id.to_string(),
            captured_at: captured_at.clone(),
            capture_method: PROVIDER_RESUME_CAPTURE_METHOD.to_string(),
            resume_args: resume_args.clone(),
            extra: BTreeMap::new(),
        });
        current.updated_at = captured_at.clone();
        Ok(())
    })
}

fn attached_loaded_thread(
    record: &SessionRecord,
    loaded_thread_ids: &[String],
) -> Result<Option<String>, String> {
    let attached = thread_attached_path(record)
        .ok_or_else(|| "Codex attached marker metadata is missing".to_string())?;
    let observed = match fs::read_to_string(attached) {
        Ok(observed) => observed,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("Codex attached thread binding was unreadable".to_string()),
    };
    Ok(loaded_thread_ids
        .iter()
        .find(|thread_id| projected_thread_binding(thread_id) == observed)
        .cloned())
}

fn projected_thread_binding(thread_id: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(b"agent-session-codex-thread-v1\0");
    digest.update(thread_id.as_bytes());
    digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SystemEphemeralThreadRegistry {
    schema_version: String,
    runtime_id: String,
    runtime_generation: u64,
    identity_digests: Vec<String>,
}

fn projected_identity_digest_is_valid(value: &str) -> bool {
    value.len() == 73
        && value.starts_with("local:v1:")
        && value[9..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn system_ephemeral_thread_registry_path(context: &CliContext, record: &SessionRecord) -> PathBuf {
    crate::session_dir(context, &record.id).join(SYSTEM_EPHEMERAL_THREADS_FILE)
}

fn read_system_ephemeral_thread_registry(
    context: &CliContext,
    record: &SessionRecord,
) -> Result<Option<SystemEphemeralThreadRegistry>, CliError> {
    let path = system_ephemeral_thread_registry_path(context, record);
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(CliError::runtime(
                "codex-system-ephemeral-registry-read-failed",
                format!("failed to read the Codex system-ephemeral registry: {error}"),
                Some(json!({ "id": record.id })),
            ));
        }
    };
    if bytes.len() > MAX_SYSTEM_EPHEMERAL_THREADS_BYTES {
        return Err(CliError::data(
            "codex-system-ephemeral-registry-invalid",
            "Codex system-ephemeral registry exceeds its size limit",
            Some(json!({ "id": record.id })),
        ));
    }
    let registry: SystemEphemeralThreadRegistry = serde_json::from_slice(&bytes).map_err(|_| {
        CliError::data(
            "codex-system-ephemeral-registry-invalid",
            "Codex system-ephemeral registry is invalid",
            Some(json!({ "id": record.id })),
        )
    })?;
    let unique = registry
        .identity_digests
        .iter()
        .collect::<BTreeSet<_>>()
        .len();
    if registry.schema_version != SYSTEM_EPHEMERAL_THREADS_VERSION
        || registry.runtime_id.is_empty()
        || registry.identity_digests.len() > MAX_SYSTEM_EPHEMERAL_THREADS
        || unique != registry.identity_digests.len()
        || registry
            .identity_digests
            .iter()
            .any(|identity_digest| !projected_identity_digest_is_valid(identity_digest))
    {
        return Err(CliError::data(
            "codex-system-ephemeral-registry-invalid",
            "Codex system-ephemeral registry failed validation",
            Some(json!({ "id": record.id })),
        ));
    }
    Ok(Some(registry))
}

pub(crate) fn register_system_ephemeral_thread(
    context: &CliContext,
    expected: &SessionRecord,
    thread_id: &str,
) -> Result<(), CliError> {
    if !runtime_is_supported(expected) || !protocol_id_is_valid(thread_id) {
        return Err(CliError::data(
            "codex-system-ephemeral-thread-invalid",
            "Codex system-ephemeral thread metadata is invalid",
            Some(json!({ "id": expected.id })),
        ));
    }
    let _record_lock = crate::acquire_session_record_lock(context, &expected.id)?;
    let current = crate::load_session_record(context, &expected.id)?;
    crate::ensure_same_session_identity(expected, &current)?;
    let runtime = current.runtime.as_ref().ok_or_else(|| {
        CliError::data(
            "codex-system-ephemeral-runtime-mismatch",
            "Codex system-ephemeral thread does not belong to the active runtime",
            Some(json!({ "id": current.id })),
        )
    })?;
    let expected_runtime = expected.runtime.as_ref().expect("supported Codex runtime");
    if !runtime_is_supported(&current)
        || runtime.launch_id != expected_runtime.launch_id
        || runtime.generation != expected_runtime.generation
    {
        return Err(CliError::data(
            "codex-system-ephemeral-runtime-mismatch",
            "Codex system-ephemeral thread does not belong to the active runtime",
            Some(json!({ "id": current.id })),
        ));
    }
    let mut registry = match read_system_ephemeral_thread_registry(context, &current)? {
        Some(registry)
            if registry.runtime_id == runtime.launch_id
                && registry.runtime_generation == runtime.generation =>
        {
            registry
        }
        _ => SystemEphemeralThreadRegistry {
            schema_version: SYSTEM_EPHEMERAL_THREADS_VERSION.to_string(),
            runtime_id: runtime.launch_id.clone(),
            runtime_generation: runtime.generation,
            identity_digests: Vec::new(),
        },
    };
    let identity_digest =
        crate::activity::projected_codex_session_identifier(&runtime.launch_id, thread_id)?;
    if registry.identity_digests.contains(&identity_digest) {
        return Ok(());
    }
    if registry.identity_digests.len() >= MAX_SYSTEM_EPHEMERAL_THREADS {
        registry.identity_digests.remove(0);
    }
    registry.identity_digests.push(identity_digest);
    let bytes = serde_json::to_vec(&registry).map_err(|_| {
        CliError::runtime(
            "codex-system-ephemeral-registry-render-failed",
            "failed to render the Codex system-ephemeral registry",
            Some(json!({ "id": current.id })),
        )
    })?;
    write_private_file(
        &system_ephemeral_thread_registry_path(context, &current),
        &bytes,
    )
}

fn system_ephemeral_identity_digest_is_registered(
    context: &CliContext,
    record: &SessionRecord,
    identity_digest: &str,
) -> Result<bool, CliError> {
    if !runtime_is_supported(record) || !projected_identity_digest_is_valid(identity_digest) {
        return Ok(false);
    }
    let Some(registry) = read_system_ephemeral_thread_registry(context, record)? else {
        return Ok(false);
    };
    let Some(runtime) = record.runtime.as_ref() else {
        return Ok(false);
    };
    if registry.runtime_id != runtime.launch_id || registry.runtime_generation != runtime.generation
    {
        return Ok(false);
    }
    Ok(registry
        .identity_digests
        .iter()
        .any(|candidate| candidate == identity_digest))
}

pub(crate) fn system_ephemeral_normalized_session_is_registered(
    context: &CliContext,
    record: &SessionRecord,
    identity_digest: &str,
) -> Result<bool, CliError> {
    system_ephemeral_identity_digest_is_registered(context, record, identity_digest)
}

pub(crate) fn system_ephemeral_raw_session_is_registered(
    context: &CliContext,
    record: &SessionRecord,
    raw_provider_session_id: &str,
) -> Result<bool, CliError> {
    if !runtime_is_supported(record) || !protocol_id_is_valid(raw_provider_session_id) {
        return Ok(false);
    }
    let Some(runtime) = record.runtime.as_ref() else {
        return Ok(false);
    };
    let identity_digest = crate::activity::projected_codex_session_identifier(
        &runtime.launch_id,
        raw_provider_session_id,
    )?;
    system_ephemeral_identity_digest_is_registered(context, record, &identity_digest)
}

pub(crate) fn run_proxy(context: &CliContext, args: crate::cli::CodexAppServerProxyArgs) -> i32 {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(err) => {
            eprintln!("error: failed to start Codex app-server proxy runtime: {err}");
            return nils_common::cli_contract::exit::RUNTIME;
        }
    };
    match runtime.block_on(run_proxy_session(context.clone(), args)) {
        Ok(()) => nils_common::cli_contract::exit::SUCCESS,
        Err(err) => {
            eprintln!("error: Codex app-server proxy failed: {err}");
            nils_common::cli_contract::exit::RUNTIME
        }
    }
}

struct ProxySocketGuard(PathBuf);

impl Drop for ProxySocketGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

struct ProxyObserver {
    pending_model_settings: BTreeMap<String, (String, Value)>,
    model_settings_writer: Option<tokio::task::JoinHandle<()>>,
    pending_thread_starts: BTreeSet<String>,
    pending_system_ephemeral_thread_starts: BTreeSet<String>,
    system_ephemeral_threads: BTreeSet<String>,
    system_ephemeral_thread_order: VecDeque<String>,
    pending_attention_requests: BTreeMap<String, String>,
    reducer: Option<FailureReducer>,
}

impl ProxyObserver {
    fn new() -> Self {
        Self {
            pending_model_settings: BTreeMap::new(),
            model_settings_writer: None,
            pending_thread_starts: BTreeSet::new(),
            pending_system_ephemeral_thread_starts: BTreeSet::new(),
            system_ephemeral_threads: BTreeSet::new(),
            system_ephemeral_thread_order: VecDeque::new(),
            pending_attention_requests: BTreeMap::new(),
            reducer: None,
        }
    }

    fn queue_model_settings(
        &mut self,
        context: &CliContext,
        record: &SessionRecord,
        thread_id: &str,
        settings: &Value,
    ) {
        let settings = project_model_settings(settings);
        if settings
            .as_object()
            .is_none_or(|settings| settings.is_empty())
        {
            return;
        }
        let previous = self.model_settings_writer.take();
        let context = context.clone();
        let record = record.clone();
        let thread_id = thread_id.to_string();
        // Keep observations ordered without delaying protocol/activity processing.
        self.model_settings_writer = Some(tokio::spawn(async move {
            if let Some(previous) = previous {
                let _ = previous.await;
            }
            let _ = tokio::task::spawn_blocking(move || {
                crate::session_model::observe_primary_codex_thread(
                    &context,
                    &record.id,
                    &record.runtime.as_ref().expect("bound runtime").launch_id,
                    &thread_id,
                    &settings,
                )
            })
            .await;
        }));
    }

    async fn finish_model_settings(&mut self) {
        if let Some(writer) = self.model_settings_writer.take() {
            let _ = writer.await;
        }
    }

    fn observe_client(&mut self, record: &SessionRecord, value: &Value) -> Result<(), String> {
        match value.get("method").and_then(Value::as_str) {
            Some("thread/start") => {
                track_thread_start(&mut self.pending_thread_starts, value);
                if value
                    .pointer("/params/systemEphemeral")
                    .and_then(Value::as_bool)
                    == Some(true)
                {
                    track_thread_start(&mut self.pending_system_ephemeral_thread_starts, value);
                }
            }
            Some("turn/start") => {
                if let Some(thread_id) = value
                    .pointer("/params/threadId")
                    .and_then(Value::as_str)
                    .filter(|id| protocol_id_is_valid(id))
                    && !self.system_ephemeral_threads.contains(thread_id)
                {
                    self.bind(record, thread_id)?;
                    if let Some(key) = value.get("id").and_then(json_id_key) {
                        if self.pending_model_settings.len() >= MAX_REDUCER_PENDING_TURNS {
                            self.pending_model_settings.clear();
                        }
                        self.pending_model_settings.remove(&key);
                        let settings = project_model_settings(&value["params"]);
                        if settings
                            .as_object()
                            .is_some_and(|settings| !settings.is_empty())
                        {
                            self.pending_model_settings
                                .insert(key, (thread_id.to_string(), settings));
                        }
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    async fn observe_server(
        &mut self,
        context: &CliContext,
        record: &SessionRecord,
        value: &Value,
        persisted_thread: Option<&str>,
    ) -> Result<(), String> {
        let response_key = value.get("id").and_then(json_id_key);
        let system_ephemeral = response_key
            .as_ref()
            .is_some_and(|key| self.pending_system_ephemeral_thread_starts.remove(key));
        match (
            completed_thread_start(&mut self.pending_thread_starts, value),
            persisted_thread,
            system_ephemeral,
        ) {
            (Some(thread_id), Some(persisted_thread), false) if thread_id == persisted_thread => {
                self.reducer = Some(FailureReducer::new(thread_id));
                self.pending_attention_requests.clear();
                self.queue_model_settings(context, record, thread_id, &value["result"]);
            }
            (Some(thread_id), None, true) => {
                if self.reducer.is_none() {
                    return Err(
                        "Codex system-ephemeral thread arrived before the primary binding"
                            .to_string(),
                    );
                }
                let worker_context = context.clone();
                let worker_record = record.clone();
                let worker_thread = thread_id.to_string();
                tokio::task::spawn_blocking(move || {
                    register_system_ephemeral_thread(
                        &worker_context,
                        &worker_record,
                        &worker_thread,
                    )
                })
                .await
                .map_err(|error| format!("Codex system-ephemeral registry worker failed: {error}"))?
                .map_err(|error| {
                    format!(
                        "Codex system-ephemeral registry update failed: {}",
                        error.code()
                    )
                })?;
                insert_bounded_id(
                    &mut self.system_ephemeral_threads,
                    &mut self.system_ephemeral_thread_order,
                    thread_id,
                );
            }
            (Some(thread_id), None, false) => {
                if self.reducer.is_none() {
                    self.bind(record, thread_id)?;
                }
                self.reducer = Some(FailureReducer::new(thread_id));
                self.pending_attention_requests.clear();
            }
            (None, None, false) => {}
            _ => {
                return Err("Codex persisted thread binding did not match the response".to_string());
            }
        }
        if value.get("error").is_some()
            && let Some(key) = response_key.as_ref()
        {
            self.pending_model_settings.remove(key);
        }
        if value
            .pointer("/result/turn/id")
            .and_then(Value::as_str)
            .is_some()
            && let Some(key) = response_key
            && let Some((thread_id, settings)) = self.pending_model_settings.remove(&key)
        {
            self.queue_model_settings(context, record, &thread_id, &settings);
        }
        if matches!(
            value.get("method").and_then(Value::as_str),
            Some("agent-session/attention/requested" | "agent-session/attention/resolved")
        ) && attention_authority(record) != ATTENTION_AUTHORITY_PROTOCOL
        {
            // Transport support and exact-attention completeness are separate
            // capabilities. Hook-authoritative app-server runtimes ignore the
            // private attention projection and keep lifecycle hooks as their
            // sole source.
            return Ok(());
        }
        if let Some(reducer) = self.reducer.as_mut() {
            process_live_message(
                context,
                record,
                reducer,
                Some(&mut self.pending_attention_requests),
                value,
            )
            .await?;
        } else if matches!(
            value.get("method").and_then(Value::as_str),
            Some("agent-session/attention/requested" | "agent-session/attention/resolved")
        ) {
            return Err(
                "Codex attention request arrived before the runtime thread was bound".to_string(),
            );
        }
        Ok(())
    }

    fn bind(&mut self, record: &SessionRecord, thread_id: &str) -> Result<(), String> {
        if self.reducer.is_some() {
            return self.bind_persisted(thread_id);
        }
        bind_thread(record, thread_id)?;
        self.bind_persisted(thread_id)
    }

    fn bind_persisted(&mut self, thread_id: &str) -> Result<(), String> {
        if let Some(reducer) = self.reducer.as_ref() {
            return (reducer.thread_id == thread_id)
                .then_some(())
                .ok_or_else(|| "Codex TUI proxy switched to a different thread".to_string());
        }
        self.reducer = Some(FailureReducer::new(thread_id));
        Ok(())
    }
}

fn track_thread_start(pending: &mut BTreeSet<String>, value: &Value) {
    let Some(key) = value.get("id").and_then(json_id_key) else {
        return;
    };
    if pending.len() >= MAX_REDUCER_PENDING_TURNS
        && !pending.contains(&key)
        && let Some(oldest) = pending.iter().next().cloned()
    {
        pending.remove(&oldest);
    }
    pending.insert(key);
}

fn completed_thread_start<'a>(pending: &mut BTreeSet<String>, value: &'a Value) -> Option<&'a str> {
    let key = value.get("id").and_then(json_id_key)?;
    if !pending.remove(&key) {
        return None;
    }
    value
        .pointer("/result/thread/id")
        .and_then(Value::as_str)
        .filter(|id| protocol_id_is_valid(id))
}

fn json_id_key(value: &Value) -> Option<String> {
    match value {
        Value::String(id) if id.len() <= MAX_PROTOCOL_ID_BYTES => Some(value.to_string()),
        Value::Number(_) => Some(value.to_string()),
        _ => None,
    }
}

fn json_rpc_response_id_key(value: &Value) -> Option<String> {
    if value.get("method").is_some() {
        return None;
    }
    let has_result = value.get("result").is_some();
    let has_error = value.get("error").is_some();
    if has_result == has_error {
        return None;
    }
    value.get("id").and_then(json_id_key)
}

fn attention_request_id_key(value: &Value) -> Option<String> {
    match value {
        Value::String(id) if !id.is_empty() && id.len() <= MAX_PROTOCOL_ID_BYTES => {
            Some(format!("string:{id}"))
        }
        Value::Number(number) => number.as_i64().map(|id| format!("int64:{id}")),
        _ => None,
    }
}

fn message_value(message: &Message) -> Result<Option<Value>, String> {
    match message {
        Message::Text(text) => serde_json::from_str(text)
            .map(Some)
            .map_err(|_| "proxy observed malformed JSON text".to_string()),
        Message::Binary(bytes) => serde_json::from_slice(bytes)
            .map(Some)
            .map_err(|_| "proxy observed malformed JSON binary data".to_string()),
        _ => Ok(None),
    }
}

enum ProxyObservation {
    Client(Value),
    Server {
        value: Value,
        persisted_thread: Option<String>,
        binding_ack: Option<oneshot::Sender<Result<(), String>>>,
    },
}

enum ServerProjection {
    Irrelevant,
    Repeatable(Value),
    Unique(Value),
    RejectedUnique,
}

type SharedFailCloseTask = Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>;

fn start_fail_close_task(task: &SharedFailCloseTask, context: &CliContext, record: &SessionRecord) {
    let mut task = task.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if task.is_none() {
        let context = context.clone();
        let record = record.clone();
        *task = Some(tokio::spawn(async move {
            fail_closed_projection(&context, &record).await;
        }));
    }
}

struct ProxyProjection {
    sender: Option<mpsc::Sender<ProxyObservation>>,
    task: Option<tokio::task::JoinHandle<()>>,
    fail_close_task: SharedFailCloseTask,
    pending_thread_starts: BTreeSet<String>,
    pending_system_ephemeral_thread_starts: BTreeSet<String>,
    requires_thread_binding: bool,
    context: CliContext,
    record: SessionRecord,
}

impl ProxyProjection {
    fn new(context: CliContext, record: SessionRecord) -> Self {
        let (sender, mut receiver) = mpsc::channel(MAX_PROXY_OBSERVATIONS);
        let worker_context = context.clone();
        let worker_record = record.clone();
        let fail_close_task = Arc::new(Mutex::new(None));
        let worker_fail_close_task = fail_close_task.clone();
        let task = tokio::spawn(async move {
            let mut observer = ProxyObserver::new();
            while let Some(observation) = receiver.recv().await {
                let (result, binding_ack) = match observation {
                    ProxyObservation::Client(value) => {
                        (observer.observe_client(&worker_record, &value), None)
                    }
                    ProxyObservation::Server {
                        value,
                        persisted_thread,
                        binding_ack,
                    } => (
                        observer
                            .observe_server(
                                &worker_context,
                                &worker_record,
                                &value,
                                persisted_thread.as_deref(),
                            )
                            .await,
                        binding_ack,
                    ),
                };
                if let Some(binding_ack) = binding_ack {
                    let _ = binding_ack.send(result.clone());
                }
                if let Err(error) = result {
                    eprintln!("warning: Codex projection disabled: {error}");
                    start_fail_close_task(&worker_fail_close_task, &worker_context, &worker_record);
                    break;
                }
            }
            observer.finish_model_settings().await;
        });
        Self {
            sender: Some(sender),
            task: Some(task),
            fail_close_task,
            pending_thread_starts: BTreeSet::new(),
            pending_system_ephemeral_thread_starts: BTreeSet::new(),
            requires_thread_binding: thread_attached_path(&record)
                .is_some_and(|path| !path.is_file()),
            context,
            record,
        }
    }

    fn observe_client(&mut self, value: &Value) {
        if !self.is_active() {
            return;
        }
        if let Some(value) = client_observation(value) {
            if value.get("method").and_then(Value::as_str) == Some("thread/start") {
                track_thread_start(&mut self.pending_thread_starts, &value);
                if value
                    .pointer("/params/systemEphemeral")
                    .and_then(Value::as_bool)
                    == Some(true)
                {
                    track_thread_start(&mut self.pending_system_ephemeral_thread_starts, &value);
                }
            }
            self.enqueue(ProxyObservation::Client(value));
        }
    }

    #[cfg(test)]
    fn observe_server(&mut self, value: &Value) {
        match server_observation(value) {
            ServerProjection::Irrelevant => {}
            ServerProjection::Repeatable(value) | ServerProjection::Unique(value) => {
                self.enqueue(ProxyObservation::Server {
                    value,
                    persisted_thread: None,
                    binding_ack: None,
                });
            }
            ServerProjection::RejectedUnique => {
                eprintln!("warning: Codex projection disabled: unique observation was invalid");
                self.disable();
            }
        }
    }

    async fn observe_server_before_forward(&mut self, value: &Value) -> Result<(), String> {
        if !self.is_active() {
            if self.requires_thread_binding {
                self.disable();
                return Err("Codex projection binding queue unavailable".to_string());
            }
            return Ok(());
        }
        match server_observation(value) {
            ServerProjection::Irrelevant => {}
            ServerProjection::Repeatable(value) => {
                self.enqueue(ProxyObservation::Server {
                    value,
                    persisted_thread: None,
                    binding_ack: None,
                });
            }
            ServerProjection::Unique(value) => {
                // A fresh TUI may submit its first turn as soon as it receives
                // the thread/start response. First require the projection
                // worker to accept the bound identity, then publish the marker
                // before forwarding the response. No new observation can make
                // the acknowledged worker fail while this proxy branch waits.
                let response_key = value.get("id").and_then(json_id_key);
                let system_ephemeral = response_key
                    .as_ref()
                    .is_some_and(|key| self.pending_system_ephemeral_thread_starts.remove(key));
                let completed_thread =
                    completed_thread_start(&mut self.pending_thread_starts, &value)
                        .map(str::to_string);
                if system_ephemeral {
                    let Some(_) = completed_thread else {
                        self.disable();
                        return Err(
                            "Codex system-ephemeral thread response was invalid".to_string()
                        );
                    };
                    if let Err(error) = self.enqueue_acknowledged_server(value, None).await {
                        eprintln!("warning: Codex projection disabled: {error}");
                        self.disable();
                        return Err(error);
                    }
                    return Ok(());
                }
                let Some(persisted_thread) = completed_thread else {
                    self.enqueue(ProxyObservation::Server {
                        value,
                        persisted_thread: None,
                        binding_ack: None,
                    });
                    return Ok(());
                };
                // A resumed TUI starts with thread/resume rather than
                // thread/start. Its binding can be published after this
                // projection was constructed; do not mistake a later /new
                // for the initial bind and reject the resumed thread marker.
                if !self.requires_thread_binding
                    || (self.record.provider_resume.is_some()
                        && thread_attached_path(&self.record).is_some_and(Path::is_file))
                {
                    let context = self.context.clone();
                    let record = self.record.clone();
                    let thread_id = persisted_thread.clone();
                    tokio::task::spawn_blocking(move || {
                        crate::conversation::observe_native(&context, &record, &thread_id)
                    })
                    .await
                    .map_err(|_| "conversation rebind worker failed".to_string())?
                    .map_err(|err| format!("conversation rebind failed: {}", err.code()))?;
                    self.enqueue_acknowledged_server(value, Some(persisted_thread))
                        .await?;
                    self.requires_thread_binding = false;
                    return Ok(());
                }
                if let Err(error) = self
                    .enqueue_acknowledged_server(value, Some(persisted_thread.clone()))
                    .await
                {
                    eprintln!("warning: Codex projection disabled: {error}");
                    self.disable();
                    return Err(error);
                }
                let record = self.record.clone();
                let worker_thread = persisted_thread;
                let result =
                    tokio::task::spawn_blocking(move || bind_thread(&record, &worker_thread))
                        .await
                        .map_err(|error| format!("Codex thread binding worker failed: {error}"))
                        .and_then(|result| result);
                if let Err(error) = result {
                    eprintln!("warning: Codex projection disabled: {error}");
                    self.disable();
                    return Err(error);
                }
                self.requires_thread_binding = false;
            }
            ServerProjection::RejectedUnique => {
                eprintln!("warning: Codex projection disabled: unique observation was invalid");
                let required_binding_rejected = self.requires_thread_binding;
                self.disable();
                if required_binding_rejected {
                    return Err("Codex projection required thread binding was invalid".to_string());
                }
            }
        }
        Ok(())
    }

    fn is_active(&mut self) -> bool {
        match self.sender.as_ref() {
            Some(sender) if !sender.is_closed() => true,
            Some(_) => {
                self.disable();
                false
            }
            None => false,
        }
    }

    fn enqueue(&mut self, observation: ProxyObservation) -> bool {
        let Some(sender) = self.sender.as_ref() else {
            return false;
        };
        match sender.try_send(observation) {
            Ok(()) => true,
            Err(mpsc::error::TrySendError::Full(ProxyObservation::Server {
                value,
                persisted_thread: _,
                binding_ack: None,
            })) if value.get("method").and_then(Value::as_str)
                == Some("account/rateLimits/updated") =>
            {
                // Rate-limit updates are advisory and repeatable. The
                // scheduler rechecks scheduled claims, so a saturated queue
                // may coalesce this update without losing a unique event.
                true
            }
            Err(_) => {
                eprintln!("warning: Codex projection disabled: observation queue unavailable");
                self.disable();
                false
            }
        }
    }

    async fn enqueue_binding(&mut self, observation: ProxyObservation) -> Result<(), String> {
        let sender = self
            .sender
            .as_ref()
            .cloned()
            .ok_or_else(|| "Codex projection binding queue unavailable".to_string())?;
        tokio::time::timeout(CONTROL_RESPONSE_TIMEOUT, sender.send(observation))
            .await
            .map_err(|_| "Codex projection binding queue timed out".to_string())?
            .map_err(|_| "Codex projection binding queue unavailable".to_string())
    }

    async fn enqueue_acknowledged_server(
        &mut self,
        value: Value,
        persisted_thread: Option<String>,
    ) -> Result<(), String> {
        let (binding_ack, receive_ack) = oneshot::channel();
        self.enqueue_binding(ProxyObservation::Server {
            value,
            persisted_thread,
            binding_ack: Some(binding_ack),
        })
        .await?;
        tokio::time::timeout(CONTROL_RESPONSE_TIMEOUT, receive_ack)
            .await
            .map_err(|_| "Codex projection acknowledgement timed out".to_string())?
            .map_err(|_| "Codex projection acknowledgement was unavailable".to_string())?
    }

    fn disable(&mut self) {
        self.sender = None;
        self.pending_thread_starts.clear();
        self.pending_system_ephemeral_thread_starts.clear();
        if let Some(task) = self.task.take() {
            task.abort();
        }
        start_fail_close_task(&self.fail_close_task, &self.context, &self.record);
    }

    fn has_fail_close_task(&self) -> bool {
        self.fail_close_task
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_some()
    }

    fn take_fail_close_task(&self) -> Option<tokio::task::JoinHandle<()>> {
        self.fail_close_task
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
    }

    async fn finish_fail_close(&mut self) {
        self.disable();
        let mut retry = false;
        if let Some(mut task) = self.take_fail_close_task() {
            match tokio::time::timeout(CONTROL_RESPONSE_TIMEOUT, &mut task).await {
                Ok(Ok(())) => {}
                Ok(Err(_)) => retry = true,
                Err(_) => {
                    // Dropping a JoinHandle detaches the retry. The durable
                    // unhealthy marker already makes activity and auto-resume
                    // fail closed while a contended session lock converges.
                }
            }
        }
        if retry {
            let context = self.context.clone();
            let record = self.record.clone();
            let mut task = tokio::spawn(async move {
                fail_closed_projection(&context, &record).await;
            });
            let _ = tokio::time::timeout(CONTROL_RESPONSE_TIMEOUT, &mut task).await;
        }
    }

    async fn finish(&mut self) {
        if self.has_fail_close_task() {
            self.finish_fail_close().await;
            return;
        }
        self.sender = None;
        if let Some(mut task) = self.task.take()
            && tokio::time::timeout(CONTROL_RESPONSE_TIMEOUT, &mut task)
                .await
                .is_err()
        {
            task.abort();
        }
        if self.has_fail_close_task() {
            self.finish_fail_close().await;
        }
    }
}

impl Drop for ProxyProjection {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

fn client_observation(value: &Value) -> Option<Value> {
    let observation = match value.get("method").and_then(Value::as_str)? {
        "thread/start" => {
            let id = value.get("id")?;
            json_id_key(id)?;
            let system_ephemeral = value.pointer("/params/ephemeral").and_then(Value::as_bool)
                == Some(true)
                && matches!(
                    value
                        .pointer("/params/threadSource")
                        .and_then(Value::as_str),
                    Some("system" | "thread_title")
                );
            json!({
                "id": id,
                "method": "thread/start",
                "params": { "systemEphemeral": system_ephemeral }
            })
        }
        "turn/start" => {
            let thread_id = value.pointer("/params/threadId")?.as_str()?;
            if !protocol_id_is_valid(thread_id) {
                return None;
            }
            let mut observation =
                json!({"method": "turn/start", "params": {"threadId": thread_id}});
            if let Some(id) = value.get("id").filter(|id| json_id_key(id).is_some()) {
                observation["id"] = id.clone();
            }
            let mut settings = project_model_settings(&value["params"]);
            // Null overrides mean inherit on turn/start; rejected non-null
            // labels remain an explicit unknown after a successful response.
            for (source, projected) in [
                ("model", "model"),
                ("effort", "reasoning_effort"),
                ("reasoning_effort", "reasoning_effort"),
            ] {
                if value["params"].get(source).is_some_and(Value::is_null) {
                    settings.as_object_mut()?.remove(projected);
                }
            }
            observation["params"]
                .as_object_mut()?
                .extend(settings.as_object()?.clone());
            observation
        }
        _ => return None,
    };
    bounded_observation(observation)
}

fn project_model_settings(value: &Value) -> Value {
    let mut out = json!({});
    if let Some(value) = value.get("model") {
        out["model"] = json!(value.as_str().and_then(crate::session_model::model_label));
    }
    if let Some(value) = value
        .get("reasoning_effort")
        .or_else(|| value.get("reasoningEffort"))
        .or_else(|| value.get("effort"))
    {
        out["reasoning_effort"] =
            json!(value.as_str().and_then(crate::session_model::effort_label));
    }
    out
}

fn server_observation(value: &Value) -> ServerProjection {
    if let Some(id) = value.get("id").filter(|id| json_id_key(id).is_some())
        && value.get("error").is_some()
    {
        return ServerProjection::Unique(json!({"id": id, "error": {}}));
    }
    if let (Some(id), Some(turn_id)) = (
        value.get("id"),
        value.pointer("/result/turn/id").and_then(Value::as_str),
    ) && json_id_key(id).is_some()
        && protocol_id_is_valid(turn_id)
    {
        return ServerProjection::Unique(json!({"id": id, "result": {"turn": {"id": turn_id}}}));
    }
    if let (Some(id), Some(thread_id)) = (value.get("id"), value.pointer("/result/thread/id")) {
        let Some(thread_id) = thread_id.as_str() else {
            return ServerProjection::RejectedUnique;
        };
        if json_id_key(id).is_none() || !protocol_id_is_valid(thread_id) {
            return ServerProjection::RejectedUnique;
        }
        let mut result = json!({"thread": {"id": thread_id}});
        result.as_object_mut().expect("object").extend(
            project_model_settings(&value["result"])
                .as_object()
                .expect("object")
                .clone(),
        );
        return bounded_observation(json!({"id": id, "result": result}))
            .map(ServerProjection::Unique)
            .unwrap_or(ServerProjection::RejectedUnique);
    }
    let Some(method) = value.get("method").and_then(Value::as_str) else {
        return ServerProjection::Irrelevant;
    };
    if let Some(kind) = match method {
        "item/commandExecution/requestApproval"
        | "item/fileChange/requestApproval"
        | "item/permissions/requestApproval" => Some("approval"),
        "item/tool/requestUserInput" => Some("clarification"),
        "mcpServer/elicitation/request" => {
            match value.pointer("/params/mode").and_then(Value::as_str) {
                Some("form" | "openai/form") => Some("clarification"),
                Some("url") => Some("authentication"),
                _ => return ServerProjection::RejectedUnique,
            }
        }
        _ => None,
    } {
        let (Some(request_id), Some(thread_id)) = (
            value.get("id"),
            value.pointer("/params/threadId").and_then(Value::as_str),
        ) else {
            return ServerProjection::RejectedUnique;
        };
        let turn_id = value.pointer("/params/turnId");
        let turn_required = method != "mcpServer/elicitation/request";
        if attention_request_id_key(request_id).is_none()
            || !protocol_id_is_valid(thread_id)
            || (turn_required
                && !turn_id
                    .and_then(Value::as_str)
                    .is_some_and(protocol_id_is_valid))
            || (!turn_required
                && turn_id.is_some_and(|turn_id| {
                    !turn_id.is_null() && !turn_id.as_str().is_some_and(protocol_id_is_valid)
                }))
        {
            return ServerProjection::RejectedUnique;
        }
        return bounded_observation(json!({
            "method": "agent-session/attention/requested",
            "params": {
                "requestId": request_id,
                "threadId": thread_id,
                "turnId": turn_id,
                "kind": kind
            }
        }))
        .map(ServerProjection::Unique)
        .unwrap_or(ServerProjection::RejectedUnique);
    }
    if method == "serverRequest/resolved" {
        let (Some(request_id), Some(thread_id)) = (
            value.pointer("/params/requestId"),
            value.pointer("/params/threadId").and_then(Value::as_str),
        ) else {
            return ServerProjection::RejectedUnique;
        };
        if attention_request_id_key(request_id).is_none() || !protocol_id_is_valid(thread_id) {
            return ServerProjection::RejectedUnique;
        }
        return bounded_observation(json!({
            "method": "agent-session/attention/resolved",
            "params": {"requestId": request_id, "threadId": thread_id}
        }))
        .map(ServerProjection::Unique)
        .unwrap_or(ServerProjection::RejectedUnique);
    }
    let observation = match method {
        "error" => {
            let (Some(thread_id), Some(turn_id)) = (
                value.pointer("/params/threadId").and_then(Value::as_str),
                value.pointer("/params/turnId").and_then(Value::as_str),
            ) else {
                return ServerProjection::RejectedUnique;
            };
            if !protocol_id_is_valid(thread_id) || !protocol_id_is_valid(turn_id) {
                return ServerProjection::RejectedUnique;
            }
            json!({
                "method": "error",
                "params": {
                    "threadId": thread_id,
                    "turnId": turn_id,
                    "willRetry": value.pointer("/params/willRetry"),
                    "error": {
                        "codexErrorInfo": value.pointer("/params/error/codexErrorInfo")
                    }
                }
            })
        }
        "turn/completed" => {
            let (Some(thread_id), Some(turn_id)) = (
                value.pointer("/params/threadId").and_then(Value::as_str),
                value.pointer("/params/turn/id").and_then(Value::as_str),
            ) else {
                return ServerProjection::RejectedUnique;
            };
            if !protocol_id_is_valid(thread_id) || !protocol_id_is_valid(turn_id) {
                return ServerProjection::RejectedUnique;
            }
            json!({
                "method": "turn/completed",
                "params": {
                    "threadId": thread_id,
                    "turn": {
                        "id": turn_id,
                        "status": value.pointer("/params/turn/status"),
                        "error": {
                            "codexErrorInfo": value.pointer("/params/turn/error/codexErrorInfo")
                        }
                    }
                }
            })
        }
        "account/rateLimits/updated" => json!({
            "method": "account/rateLimits/updated",
            "params": {
                "rateLimits": value.pointer("/params/rateLimits"),
                "rateLimitsByLimitId": value.pointer("/params/rateLimitsByLimitId")
            }
        }),
        _ => return ServerProjection::Irrelevant,
    };
    match bounded_observation(observation) {
        Some(observation) if method == "account/rateLimits/updated" => {
            ServerProjection::Repeatable(observation)
        }
        Some(observation) => ServerProjection::Unique(observation),
        None if method == "account/rateLimits/updated" => ServerProjection::Irrelevant,
        None => ServerProjection::RejectedUnique,
    }
}

fn bounded_observation(value: Value) -> Option<Value> {
    (serde_json::to_vec(&value).ok()?.len() <= MAX_PROXY_OBSERVATION_BYTES).then_some(value)
}

async fn fail_closed_projection(context: &CliContext, record: &SessionRecord) {
    let context = context.clone();
    let id = record.id.clone();
    let Some(launch_id) = record
        .runtime
        .as_ref()
        .map(|runtime| runtime.launch_id.clone())
    else {
        return;
    };
    let mut retry_delay = Duration::from_millis(100);
    loop {
        let context = context.clone();
        let id = id.clone();
        let launch_id = launch_id.clone();
        match tokio::task::spawn_blocking(move || {
            crate::activity::mark_runtime_unhealthy(
                &context,
                &id,
                &launch_id,
                "codex_projection_unavailable",
            )?;
            crate::auto_resume::fail_closed_projection_for_runtime(
                &context,
                &id,
                &launch_id,
                &Timestamp::now().to_string(),
            )
        })
        .await
        {
            Ok(Ok(())) => return,
            Ok(Err(error)) if error.code() == "session-record-lock-timeout" => {
                tokio::time::sleep(retry_delay).await;
                retry_delay = (retry_delay * 2).min(Duration::from_secs(1));
            }
            Ok(Err(error)) => {
                eprintln!(
                    "warning: Codex projection fail-close stopped after permanent error: {}",
                    error.code()
                );
                return;
            }
            Err(error) => {
                eprintln!(
                    "warning: Codex projection fail-close worker failed permanently: {error}"
                );
                return;
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum FreshBootstrap {
    ThreadStart,
    ThreadResponse { request_id: String },
    FirstTurn { thread_id: String },
    Closed,
}

impl FreshBootstrap {
    fn for_runtime(context: &CliContext, record: &SessionRecord) -> Self {
        let starting =
            crate::activity::activity_status(context, &record.id).is_ok_and(|activity| {
                activity.turn_state.phase == crate::activity::TurnPhase::Starting
            });
        let auto_resume_idle = auto_resume_is_healthy_idle(context, record);
        if record.provider_resume.is_none()
            && thread_attached_path(record).is_some_and(|path| !path.is_file())
            && starting
            && auto_resume_idle
            && create_bootstrap_is_live(record)
        {
            Self::ThreadStart
        } else {
            Self::Closed
        }
    }

    fn bypasses_create_lock(
        &mut self,
        context: &CliContext,
        record: &SessionRecord,
        value: &Value,
    ) -> bool {
        if matches!(self, Self::Closed) {
            return false;
        }
        if !auto_resume_is_healthy_idle(context, record) {
            *self = Self::Closed;
            return false;
        }
        match self {
            Self::ThreadStart
                if value.get("method").and_then(Value::as_str) == Some("thread/start") =>
            {
                let Some(request_id) = value.get("id").and_then(json_id_key) else {
                    *self = Self::Closed;
                    return false;
                };
                *self = Self::ThreadResponse { request_id };
                true
            }
            Self::FirstTurn { thread_id }
                if value.get("method").and_then(Value::as_str) == Some("turn/start") =>
            {
                let matches_bound_thread = value
                    .pointer("/params/threadId")
                    .and_then(Value::as_str)
                    .is_some_and(|candidate| candidate == thread_id);
                *self = Self::Closed;
                matches_bound_thread
            }
            Self::ThreadResponse { .. } | Self::FirstTurn { .. } | Self::ThreadStart => {
                *self = Self::Closed;
                false
            }
            Self::Closed => false,
        }
    }

    fn observe_server(&mut self, value: &Value) {
        let Self::ThreadResponse { request_id } = self else {
            return;
        };
        if value.get("id").and_then(json_id_key).as_deref() != Some(request_id.as_str()) {
            return;
        }
        *self = value
            .pointer("/result/thread/id")
            .and_then(Value::as_str)
            .filter(|thread_id| protocol_id_is_valid(thread_id))
            .map(|thread_id| Self::FirstTurn {
                thread_id: thread_id.to_string(),
            })
            .unwrap_or(Self::Closed);
    }

    fn close(&mut self) {
        *self = Self::Closed;
    }
}

struct MutationAuthorization {
    _bootstrap_gate: Option<CreateBootstrapGate>,
    _turn_start_gate: Option<ManualInputGate>,
    _account_authority: Option<crate::LockedSessionAuthority>,
}

fn auto_resume_is_healthy_idle(context: &CliContext, record: &SessionRecord) -> bool {
    let view = crate::auto_resume::view_for_record(context, record);
    let idle_state_matches_enablement = match view.state.as_str() {
        "disabled" => !view.enabled,
        "enabled" => view.enabled,
        _ => false,
    };
    // A profile may disable automatic continuation while still using the
    // managed protocol. Its healthy disabled state has no continuation to
    // cancel; it must not deny the create-owned initial thread and turn.
    (view.supported || (!view.enabled && view.state == "disabled"))
        && idle_state_matches_enablement
        && view.scheduled_at.is_none()
        && view.failure_reason.is_none()
}

async fn ensure_turn_start_account_ready(context: &CliContext, record: &SessionRecord) -> bool {
    // The long-lived control connection is the sole owner of live account
    // mutation. Hold this exact turn/start while its idle-boundary drive drains
    // a queued/applying intent, then revalidate under the record lock below.
    let deadline = Instant::now() + CONTROL_SUBMIT_TOTAL_TIMEOUT;
    loop {
        let load_context = context.clone();
        let id = record.id.clone();
        let Ok(Ok(current)) =
            tokio::task::spawn_blocking(move || crate::load_session_record(&load_context, &id))
                .await
        else {
            return false;
        };
        if crate::ensure_same_session_identity(record, &current).is_err() {
            return false;
        }
        if crate::codex_account::ensure_proxy_input_allowed(&current).is_ok() {
            return true;
        }
        let pending = crate::codex_account::proxy_next_account_is_pending(&current);
        if !pending || Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn lock_turn_start_account_authority(
    context: &CliContext,
    expected: &SessionRecord,
) -> Option<crate::LockedSessionAuthority> {
    let authority_context = context.clone();
    let authority_id = expected.id.clone();
    let expected = expected.clone();
    tokio::task::spawn_blocking(move || {
        crate::lock_exact_session_authority(&authority_context, &authority_id)
    })
    .await
    .ok()
    .and_then(Result::ok)
    .flatten()
    .filter(|authority| {
        crate::ensure_same_session_identity(&expected, &authority.record).is_ok()
            && crate::codex_account::ensure_proxy_input_allowed(&authority.record).is_ok()
    })
}

async fn cancel_before_tui_mutation_detailed(
    context: &CliContext,
    record: &SessionRecord,
    bootstrap: &mut FreshBootstrap,
    value: &Value,
) -> Result<MutationAuthorization, TuiMutationRejection> {
    let method = value.get("method").and_then(Value::as_str);
    if (crate::codex_account::binding_is_present(record)
        || crate::codex_account::view_for_record(record).supported)
        && matches!(
            method,
            Some("account/login/start" | "account/login/cancel" | "account/logout")
        )
    {
        bootstrap.close();
        return Err(TuiMutationRejection::AccountMutationForbidden);
    }
    if !matches!(method, Some("thread/start" | "turn/start")) {
        return Ok(MutationAuthorization {
            _bootstrap_gate: None,
            _turn_start_gate: None,
            _account_authority: None,
        });
    }
    if method == Some("turn/start") && !ensure_turn_start_account_ready(context, record).await {
        bootstrap.close();
        return Err(TuiMutationRejection::AccountNotReady);
    }
    let turn_start_gate = if method == Some("turn/start") {
        let gate_context = context.clone();
        let gate_record = record.clone();
        let gate_value = value.clone();
        tokio::task::spawn_blocking(move || {
            acquire_turn_start_gate(&gate_context, &gate_record, &gate_value)
        })
        .await
        .map_err(|_| TuiMutationRejection::TurnGateOpenFailed)??
        .into()
    } else {
        None
    };
    // A fresh Codex TUI emits `thread/start`, then the initial prompt emits
    // `turn/start`, while the parent create path still owns the lifecycle lock.
    // The create-owned marker and exact healthy idle auto-resume state are
    // checked live for both requests. A per-device default may opt in before
    // the TUI creates its thread, but armed or failed continuation state must
    // still fail closed. The first turn is authorized only by a successful
    // matching thread/start response and must target the returned thread id.
    #[cfg(test)]
    {
        let mut attempts = normal_cancellation_attempts()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *attempts.entry(record.id.clone()).or_default() += 1;
    }
    let bootstrap_gate = if matches!(bootstrap, FreshBootstrap::Closed) {
        None
    } else {
        let gate_record = record.clone();
        tokio::task::spawn_blocking(move || acquire_create_bootstrap_gate(&gate_record))
            .await
            .ok()
            .flatten()
    };
    let cancellation_context = context.clone();
    let id = record.id.clone();
    let launch_id = record
        .runtime
        .as_ref()
        .map(|runtime| runtime.launch_id.clone())
        .ok_or(TuiMutationRejection::RuntimeIdentityMissing)?;
    let sender_owns_record_authority = turn_start_gate
        .as_ref()
        .is_some_and(|gate| gate._owner_file.is_some());
    let wait_for_transient_lock = bootstrap_gate.is_none() && !sender_owns_record_authority;
    let record_turn_fence = method == Some("turn/start");
    let cancellation = tokio::task::spawn_blocking(move || {
        let now = Timestamp::now().to_string();
        if wait_for_transient_lock {
            crate::auto_resume::cancel_for_manual_input_for_runtime_with_timeout(
                &cancellation_context,
                &id,
                &launch_id,
                &now,
                record_turn_fence,
            )
        } else {
            crate::auto_resume::try_cancel_for_manual_input_for_runtime(
                &cancellation_context,
                &id,
                &launch_id,
                &now,
                record_turn_fence,
            )
        }
    })
    .await
    .map_err(|_| TuiMutationRejection::AccountAuthorityUnavailable)?
    .map_err(|_| TuiMutationRejection::AccountAuthorityUnavailable)?;
    // Either gate proves that the outer sender still owns the matching
    // record/lifecycle authority. Reacquiring the record lock while holding
    // the gate would invert marker teardown's lock order.
    let sender_owns_record_authority = bootstrap_gate.is_some() || sender_owns_record_authority;
    let mut authorization = match cancellation {
        crate::auto_resume::ManualInputCancelOutcome::Ready => {
            bootstrap.close();
            Ok(MutationAuthorization {
                _bootstrap_gate: bootstrap_gate,
                _turn_start_gate: turn_start_gate,
                _account_authority: None,
            })
        }
        crate::auto_resume::ManualInputCancelOutcome::Busy
            if bootstrap_gate.is_some()
                && bootstrap.bypasses_create_lock(context, record, value) =>
        {
            Ok(MutationAuthorization {
                _bootstrap_gate: bootstrap_gate,
                _turn_start_gate: turn_start_gate,
                _account_authority: None,
            })
        }
        crate::auto_resume::ManualInputCancelOutcome::Busy
            if turn_start_gate
                .as_ref()
                .is_some_and(|gate| gate._owner_file.is_some()) =>
        {
            bootstrap.close();
            Ok(MutationAuthorization {
                _bootstrap_gate: bootstrap_gate,
                _turn_start_gate: turn_start_gate,
                _account_authority: None,
            })
        }
        crate::auto_resume::ManualInputCancelOutcome::Busy => {
            Err(TuiMutationRejection::ManualCancellationBusy)
        }
        crate::auto_resume::ManualInputCancelOutcome::RuntimeChanged => {
            bootstrap.close();
            Err(TuiMutationRejection::RuntimeChanged)
        }
    };
    if method == Some("turn/start") && !sender_owns_record_authority {
        let authorization = match authorization.as_mut() {
            Ok(authorization) => authorization,
            Err(rejection) => return Err(*rejection),
        };
        let authority = lock_turn_start_account_authority(context, record)
            .await
            .ok_or(TuiMutationRejection::AccountAuthorityUnavailable)?;
        authorization._account_authority = Some(authority);
    }
    authorization
}

#[cfg(test)]
async fn cancel_before_tui_mutation(
    context: &CliContext,
    record: &SessionRecord,
    bootstrap: &mut FreshBootstrap,
    value: &Value,
) -> Option<MutationAuthorization> {
    cancel_before_tui_mutation_detailed(context, record, bootstrap, value)
        .await
        .ok()
}

fn proxy_websocket_config() -> WebSocketConfig {
    WebSocketConfig::default()
        .max_message_size(Some(MAX_PROXY_MESSAGE_BYTES))
        .max_frame_size(Some(MAX_PROXY_FRAME_BYTES))
}

fn tui_busy_response(id: &Value, rejection: TuiMutationRejection) -> Message {
    Message::Text(
        json!({
            "id": id,
            "error": {
                "code": -32001,
                "message": "agent-session state is busy; retry the request",
                "data": { "reason": rejection.code() }
            }
        })
        .to_string()
        .into(),
    )
}

async fn send_proxy_upstream<S>(
    upstream: &mut S,
    message: Message,
    timeout: Duration,
) -> Result<(), String>
where
    S: futures_util::Sink<Message> + Unpin,
    S::Error: std::fmt::Display,
{
    tokio::time::timeout(timeout, upstream.send(message))
        .await
        .map_err(|_| "upstream app-server write timed out".to_string())?
        .map_err(|err| format!("upstream app-server write failed: {err}"))
}

async fn run_proxy_session(
    context: CliContext,
    args: crate::cli::CodexAppServerProxyArgs,
) -> Result<(), String> {
    let record = crate::load_session_record(&context, &args.id)
        .map_err(|err| format!("session load failed: {}", err.code()))?;
    if !runtime_is_supported(&record)
        || socket_path(&record).map(Path::new) != Some(args.upstream.as_path())
        || proxy_path(&record) != Some(args.listen.as_path())
    {
        return Err("proxy paths did not match the active Codex runtime".to_string());
    }
    match fs::remove_file(&args.listen) {
        Ok(()) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => return Err(format!("failed to remove stale proxy socket: {err}")),
    }
    let listener = UnixListener::bind(&args.listen)
        .map_err(|err| format!("failed to bind private TUI proxy: {err}"))?;
    fs::set_permissions(&args.listen, fs::Permissions::from_mode(0o600))
        .map_err(|err| format!("failed to secure private TUI proxy: {err}"))?;
    let _guard = ProxySocketGuard(args.listen.clone());
    let (tui_stream, _) = tokio::time::timeout(CONTROL_RESPONSE_TIMEOUT, listener.accept())
        .await
        .map_err(|_| "remote TUI connection timed out".to_string())?
        .map_err(|err| format!("failed to accept remote TUI: {err}"))?;
    let upstream_stream = connect_socket(&args.upstream).await?;
    let mut tui = tokio::time::timeout(
        CONTROL_RESPONSE_TIMEOUT,
        tokio_tungstenite::accept_async_with_config(tui_stream, Some(proxy_websocket_config())),
    )
    .await
    .map_err(|_| "remote TUI WebSocket handshake timed out".to_string())?
    .map_err(|err| format!("remote TUI WebSocket handshake failed: {err}"))?;
    let (mut upstream, _) = tokio::time::timeout(
        CONTROL_RESPONSE_TIMEOUT,
        tokio_tungstenite::client_async_with_config(
            "ws://localhost",
            upstream_stream,
            Some(proxy_websocket_config()),
        ),
    )
    .await
    .map_err(|_| "upstream app-server WebSocket handshake timed out".to_string())?
    .map_err(|err| format!("upstream app-server handshake failed: {err}"))?;
    let _capability = begin_proxy_capability(&context, &record)
        .map_err(|err| format!("failed to advertise proxy capability: {}", err.code()))?;
    // A resumed TUI uses thread/resume, so it has no initial thread/start
    // response to publish the manual-input binding. Seed only the exact
    // durable resume identity before accepting its first turn request.
    if let Some(resume) = record.provider_resume.as_ref() {
        if resume.provider != AgentKind::Codex.as_str() || !protocol_id_is_valid(&resume.session_id)
        {
            return Err("resumed provider identity was invalid".to_string());
        }
        bind_thread(&record, &resume.session_id)?;
    }
    let _ = crate::write_private_file(
        &crate::session_dir(&context, &record.id).join(crate::STARTUP_STAGE_FILE),
        b"initial_connection\n",
    );
    let _ = fs::remove_file(
        crate::session_dir(&context, &record.id).join(crate::STARTUP_DIAGNOSTIC_FILE),
    );
    let mut projection = ProxyProjection::new(context.clone(), record.clone());
    let mut bootstrap = FreshBootstrap::for_runtime(&context, &record);
    let mut pending_turn_authorizations = BTreeMap::<String, MutationAuthorization>::new();
    let mut pending_turn_authorization_deadline = None;
    let mut result = async {
        loop {
            tokio::select! {
            _ = async {
                match pending_turn_authorization_deadline {
                    Some(deadline) => tokio::time::sleep_until(deadline).await,
                    None => std::future::pending::<()>().await,
                }
            } => {
                return Err("Codex turn/start response timed out".to_string());
            }
            message = tui.next() => {
                let message = message
                    .ok_or_else(|| "remote TUI closed the proxy".to_string())?
                    .map_err(|err| format!("remote TUI read failed: {err}"))?;
                let (authorization, turn_start_key) = if let Some(value) = message_value(&message)? {
                    let turn_start_key = (value.get("method").and_then(Value::as_str)
                        == Some("turn/start"))
                        .then(|| value.get("id").and_then(json_id_key))
                        .flatten();
                    if turn_start_key.as_ref().is_some_and(|key| {
                        pending_turn_authorizations.contains_key(key)
                            || pending_turn_authorizations.len() >= MAX_REDUCER_PENDING_TURNS
                    }) {
                        if let Some(id) = value.get("id") {
                            tui.send(tui_busy_response(
                                id,
                                TuiMutationRejection::TurnAlreadyPending,
                            ))
                            .await
                            .map_err(|err| format!("remote TUI write failed: {err}"))?;
                        }
                        continue;
                    }
                    let authorization = match cancel_before_tui_mutation_detailed(
                        &context,
                        &record,
                        &mut bootstrap,
                        &value,
                    ).await {
                        Ok(authorization) => authorization,
                        Err(rejection) => {
                            if let Some(id) = value
                                .get("id")
                                .filter(|id| json_id_key(id).is_some())
                            {
                                tui.send(tui_busy_response(id, rejection))
                                .await
                                .map_err(|err| format!("remote TUI write failed: {err}"))?;
                            }
                            continue;
                        }
                    };
                    projection.observe_client(&value);
                    (authorization, turn_start_key)
                } else {
                    (MutationAuthorization {
                        _bootstrap_gate: None,
                        _turn_start_gate: None,
                        _account_authority: None,
                    }, None)
                };
                let closed = matches!(message, Message::Close(_));
                send_proxy_upstream(&mut upstream, message, CONTROL_RESPONSE_TIMEOUT).await?;
                if let Some(key) = turn_start_key {
                    pending_turn_authorizations.insert(key, authorization);
                    pending_turn_authorization_deadline =
                        Some(tokio::time::Instant::now() + CONTROL_SUBMISSION_TIMEOUT);
                } else {
                    drop(authorization);
                }
                if closed {
                    return Ok(());
                }
            }
            message = upstream.next() => {
                let message = message
                    .ok_or_else(|| "upstream app-server closed the proxy".to_string())?
                    .map_err(|err| format!("upstream app-server read failed: {err}"))?;
                let observed = message.clone();
                let closed = matches!(message, Message::Close(_));
                if let Some(value) = message_value(&observed)? {
                    if let Some(key) = json_rpc_response_id_key(&value) {
                        pending_turn_authorizations.remove(&key);
                        if pending_turn_authorizations.is_empty() {
                            pending_turn_authorization_deadline = None;
                        }
                    }
                    bootstrap.observe_server(&value);
                    projection.observe_server_before_forward(&value).await?;
                }
                tui.send(message).await
                    .map_err(|err| format!("remote TUI write failed: {err}"))?;
                if closed {
                    return Ok(());
                }
            }
        }
        }
    }
    .await;
    let pending_turn_uncertain = !pending_turn_authorizations.is_empty();
    if pending_turn_uncertain {
        let unhealthy_context = context.clone();
        let unhealthy_id = record.id.clone();
        let unhealthy_launch_id = record
            .runtime
            .as_ref()
            .map(|runtime| runtime.launch_id.clone())
            .ok_or_else(|| "Codex runtime identity is missing".to_string())?;
        let unhealthy_result = tokio::task::spawn_blocking(move || {
            crate::activity::mark_runtime_unhealthy(
                &unhealthy_context,
                &unhealthy_id,
                &unhealthy_launch_id,
                "codex_turn_start_outcome_uncertain",
            )
        })
        .await;
        if let Err(error) = unhealthy_result
            .map_err(|_| "Codex runtime health update task failed".to_string())
            .and_then(|result| {
                result.map_err(|err| {
                    format!(
                        "failed to mark uncertain Codex turn admission: {}",
                        err.code()
                    )
                })
            })
        {
            result = Err(error);
        }
    }
    pending_turn_authorizations.clear();
    drop(listener);
    drop(_guard);
    drop(upstream);
    drop(tui);
    if result.is_err() || pending_turn_uncertain {
        projection.finish_fail_close().await;
    } else {
        projection.finish().await;
    }
    result
}

async fn connect_socket(path: &Path) -> Result<UnixStream, String> {
    let mut attempts = 0_u16;
    loop {
        match UnixStream::connect(path).await {
            Ok(stream) => return Ok(stream),
            Err(err) if attempts < 100 => {
                attempts += 1;
                tokio::time::sleep(Duration::from_millis(100)).await;
                if attempts == 100 {
                    return Err(format!("Codex app-server socket unavailable: {err}"));
                }
            }
            Err(err) => return Err(format!("Codex app-server socket unavailable: {err}")),
        }
    }
}

async fn send_json<S>(websocket: &mut S, value: Value) -> Result<(), String>
where
    S: futures_util::Sink<Message> + Unpin,
    S::Error: std::fmt::Display,
{
    websocket
        .send(Message::Text(value.to_string().into()))
        .await
        .map_err(|err| format!("Codex app-server write failed: {err}"))
}

async fn receive_response_with_timeout<S>(
    websocket: &mut S,
    id: u64,
    live: Option<(&CliContext, &SessionRecord, &mut FailureReducer)>,
    external_auth: Option<(&CliContext, &SessionRecord, &str)>,
    timeout: Duration,
) -> Result<Value, String>
where
    S: futures_util::Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>>
        + futures_util::Sink<Message, Error = tokio_tungstenite::tungstenite::Error>
        + Unpin,
{
    tokio::time::timeout(
        timeout,
        receive_response(websocket, id, live, external_auth),
    )
    .await
    .map_err(|_| "Codex app-server request timed out".to_string())?
}

async fn receive_response<S>(
    websocket: &mut S,
    id: u64,
    mut live: Option<(&CliContext, &SessionRecord, &mut FailureReducer)>,
    external_auth: Option<(&CliContext, &SessionRecord, &str)>,
) -> Result<Value, String>
where
    S: futures_util::Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>>
        + futures_util::Sink<Message, Error = tokio_tungstenite::tungstenite::Error>
        + Unpin,
{
    loop {
        let value = decode_message(websocket.next().await).await?;
        if respond_to_external_auth_refresh(websocket, &value, external_auth).await? {
            continue;
        }
        if value.get("id").and_then(Value::as_u64) == Some(id) {
            if let Some(error) = value.get("error") {
                let category = error
                    .get("message")
                    .and_then(Value::as_str)
                    .map(protocol_error_category)
                    .unwrap_or("unknown");
                return Err(format!(
                    "Codex app-server rejected request: {} ({category})",
                    error
                        .get("code")
                        .and_then(Value::as_i64)
                        .unwrap_or_default()
                ));
            }
            return value
                .get("result")
                .cloned()
                .ok_or_else(|| "Codex app-server response omitted result".to_string());
        }
        if let Some((context, record, reducer)) = live.as_mut() {
            process_live_message(context, record, reducer, None, &value).await?;
        }
    }
}

async fn respond_to_external_auth_refresh<S>(
    websocket: &mut S,
    value: &Value,
    external_auth: Option<(&CliContext, &SessionRecord, &str)>,
) -> Result<bool, String>
where
    S: futures_util::Sink<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin,
{
    let result = respond_to_external_auth_refresh_inner(websocket, value, external_auth).await;
    if value.get("method").and_then(Value::as_str) == Some("account/chatgptAuthTokens/refresh")
        && value.pointer("/params/reason").and_then(Value::as_str) == Some("unauthorized")
        && let Some((context, record, _)) = external_auth
    {
        let recovery = if matches!(result, Ok(true)) {
            "credentials_refreshed"
        } else {
            "refresh_failed"
        };
        let context = context.clone();
        let record = record.clone();
        if !matches!(
            tokio::task::spawn_blocking(move || crate::auth_incident::recover(
                &context,
                &record,
                recovery,
                &jiff::Timestamp::now().to_string()
            ))
            .await,
            Ok(Ok(()))
        ) {
            eprintln!("warning: provider authentication recovery reporting degraded");
        }
    }
    result
}

async fn respond_to_external_auth_refresh_inner<S>(
    websocket: &mut S,
    value: &Value,
    external_auth: Option<(&CliContext, &SessionRecord, &str)>,
) -> Result<bool, String>
where
    S: futures_util::Sink<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin,
{
    if value.get("method").and_then(Value::as_str) != Some("account/chatgptAuthTokens/refresh") {
        return Ok(false);
    }
    if value.pointer("/params/reason").and_then(Value::as_str) != Some("unauthorized") {
        return Err("Codex requested an unsupported external-auth refresh".to_string());
    }
    let id = value
        .get("id")
        .filter(|id| json_id_key(id).is_some())
        .cloned()
        .ok_or_else(|| "Codex external-auth refresh request omitted a valid id".to_string())?;
    let (context, record, account) = external_auth.ok_or_else(|| {
        "Codex requested an external-auth refresh without a bound account".to_string()
    })?;
    let launch_id = record
        .runtime
        .as_ref()
        .map(|runtime| runtime.launch_id.clone())
        .ok_or_else(|| "Codex runtime identity is missing".to_string())?;
    let incident_context = context.clone();
    let incident_record = record.clone();
    let incident_event = format!(
        "external-refresh:{}:{}",
        json_id_key(&id).unwrap_or_default(),
        uuid::Uuid::new_v4()
    );
    if !matches!(
        tokio::task::spawn_blocking(move || crate::auth_incident::observe(
            &incident_context,
            &incident_record,
            &incident_event,
            crate::auth_incident::AuthSource::CodexExternalRefresh,
            crate::activity::Confidence::Authoritative,
            &jiff::Timestamp::now().to_string()
        ))
        .await,
        Ok(Ok(()))
    ) {
        eprintln!("warning: provider authentication incident persistence degraded");
    }
    let begin_context = context.clone();
    let begin_id = record.id.clone();
    let begin_launch_id = launch_id.clone();
    let begin_account = account.to_string();
    let attempt = tokio::task::spawn_blocking(move || {
        crate::codex_account::begin_refresh_binding(
            &begin_context,
            &begin_id,
            &begin_launch_id,
            &begin_account,
        )
    })
    .await
    .map_err(|_| "Codex account refresh worker failed".to_string())?
    .map_err(|err| format!("Codex account refresh rejected: {}", err.code()))?;
    let revision = attempt.revision();
    let refresh_account = account.to_string();
    let credentials_result = tokio::task::spawn_blocking(move || {
        crate::codex_account::resolve_account(&refresh_account, true)
    })
    .await
    .map_err(|_| "Codex account refresh worker failed".to_string());
    let credentials = match credentials_result {
        Ok(Ok(credentials)) => credentials,
        Ok(Err(err)) => {
            let _ = restore_account_binding_after_refresh_failure(
                context, record, &launch_id, account, attempt,
            )
            .await;
            return Err(format!("Codex account refresh failed: {}", err.code()));
        }
        Err(error) => {
            let _ = restore_account_binding_after_refresh_failure(
                context, record, &launch_id, account, attempt,
            )
            .await;
            return Err(error);
        }
    };
    let fence_context = context.clone();
    let fence_id = record.id.clone();
    let fence_launch_id = launch_id.clone();
    let fence_account = account.to_string();
    let refresh_fence = tokio::task::spawn_blocking(move || {
        let lock = crate::acquire_session_record_lock(&fence_context, &fence_id)?;
        let current = crate::load_session_record(&fence_context, &fence_id)?;
        if current
            .runtime
            .as_ref()
            .is_none_or(|runtime| runtime.launch_id != fence_launch_id)
        {
            return Err(CliError::runtime(
                "codex-account-refresh-superseded",
                "Codex runtime changed during credential refresh",
                Some(json!({ "id": current.id })),
            ));
        }
        let view = crate::codex_account::view_for_record(&current);
        if view.state != "pending"
            || view.selected_account.as_deref() != Some(fence_account.as_str())
            || view.revision != revision
        {
            return Err(CliError::runtime(
                "codex-account-refresh-superseded",
                "Codex account binding changed during credential refresh",
                Some(json!({ "id": current.id })),
            ));
        }
        Ok::<_, CliError>(lock)
    })
    .await
    .map_err(|_| "Codex account refresh fence worker failed".to_string())?
    .map_err(|err| format!("Codex account refresh rejected: {}", err.code()))?;
    let send_result = send_json(
        websocket,
        external_auth_refresh_response(
            id,
            &credentials.access_token,
            &credentials.chatgpt_account_id,
            credentials.chatgpt_plan_type.as_deref(),
        ),
    )
    .await;
    drop(refresh_fence);
    if let Err(error) = send_result {
        let _ = restore_account_binding_after_refresh_failure(
            context, record, &launch_id, account, attempt,
        )
        .await;
        return Err(error);
    }
    finish_account_binding(context, record, &launch_id, account, revision, Ok(())).await?;
    Ok(true)
}

fn protocol_error_category(message: &str) -> &'static str {
    for (needle, category) in [
        ("no rollout", "no_rollout"),
        ("already running", "already_running"),
        ("different rollout path", "rollout_path_mismatch"),
        ("stale path", "stale_rollout_path"),
        ("not found", "not_found"),
        ("missing field", "missing_field"),
        ("unknown field", "unknown_field"),
        ("invalid type", "invalid_type"),
        ("AbsolutePathBuf", "invalid_absolute_path"),
    ] {
        if message.contains(needle) {
            return category;
        }
    }
    "other"
}

async fn decode_message(
    message: Option<Result<Message, tokio_tungstenite::tungstenite::Error>>,
) -> Result<Value, String> {
    let message = message
        .ok_or_else(|| "Codex app-server connection closed".to_string())?
        .map_err(|err| format!("Codex app-server read failed: {err}"))?;
    match message {
        Message::Text(text) => serde_json::from_str(&text)
            .map_err(|_| "Codex app-server emitted malformed JSON".to_string()),
        Message::Binary(bytes) => serde_json::from_slice(&bytes)
            .map_err(|_| "Codex app-server emitted malformed JSON".to_string()),
        Message::Close(_) => Err("Codex app-server connection closed".to_string()),
        Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => Ok(json!({})),
    }
}

async fn process_live_message(
    context: &CliContext,
    record: &SessionRecord,
    reducer: &mut FailureReducer,
    pending_attention_requests: Option<&mut BTreeMap<String, String>>,
    value: &Value,
) -> Result<(), String> {
    if matches!(
        value.get("method").and_then(Value::as_str),
        Some("agent-session/attention/requested" | "agent-session/attention/resolved")
    ) {
        let method = value
            .get("method")
            .and_then(Value::as_str)
            .expect("matched attention method");
        let thread_id = value
            .pointer("/params/threadId")
            .and_then(Value::as_str)
            .ok_or_else(|| "Codex attention projection omitted thread scope".to_string())?;
        if thread_id != reducer.thread_id {
            return Err("Codex attention projection changed runtime thread scope".to_string());
        }
        let typed_request_id = value
            .pointer("/params/requestId")
            .and_then(attention_request_id_key)
            .ok_or_else(|| "Codex attention projection omitted typed request id".to_string())?;
        let requested_kind = (method == "agent-session/attention/requested")
            .then(|| {
                value
                    .pointer("/params/kind")
                    .and_then(Value::as_str)
                    .ok_or_else(|| "Codex attention projection omitted kind".to_string())
            })
            .transpose()?;
        let pending_attention_requests = pending_attention_requests.ok_or_else(|| {
            "Codex attention projection reached a non-authoritative channel".to_string()
        })?;
        let correlation_token = if requested_kind.is_some() {
            if pending_attention_requests.contains_key(&typed_request_id) {
                return Ok(());
            }
            if pending_attention_requests.len() >= MAX_PENDING_ATTENTION_REQUESTS {
                return Err(
                    "Codex attention request cardinality exceeded the bounded projection"
                        .to_string(),
                );
            }
            let token = uuid::Uuid::new_v4().to_string();
            pending_attention_requests.insert(typed_request_id, token.clone());
            token
        } else {
            let Some(token) = pending_attention_requests.remove(&typed_request_id) else {
                return Ok(());
            };
            token
        };
        let turn_id = value.pointer("/params/turnId").and_then(Value::as_str);
        let context = context.clone();
        let id = record.id.clone();
        let runtime_id = record
            .runtime
            .as_ref()
            .map(|runtime| runtime.launch_id.clone())
            .ok_or_else(|| "Codex runtime identity is missing".to_string())?;
        let thread_id = thread_id.to_string();
        let turn_id = turn_id.map(str::to_string);
        let requested_kind = requested_kind.map(str::to_string);
        tokio::task::spawn_blocking(move || {
            crate::activity::ingest_codex_app_server_attention(
                &context,
                &id,
                &runtime_id,
                &thread_id,
                turn_id.as_deref(),
                &correlation_token,
                requested_kind.as_deref(),
            )
        })
        .await
        .map_err(|_| "Codex attention ingestion worker failed".to_string())?
        .map_err(|err| format!("Codex attention ingestion failed: {}", err.code()))?;
        return Ok(());
    }
    if value.get("method").and_then(Value::as_str) == Some("account/rateLimits/updated") {
        let snapshot = value
            .get("params")
            .map(usage_snapshot)
            .unwrap_or(UsageSnapshot {
                authoritative: false,
                has_exhausted_windows: false,
                exhausted_reset_epochs: Vec::new(),
                soonest_reset_epoch: None,
            });
        wake_from_open_usage(context, record, &snapshot).await?;
    }
    if value.get("method").and_then(Value::as_str) == Some("turn/completed")
        && value.pointer("/params/threadId").and_then(Value::as_str)
            == Some(reducer.thread_id.as_str())
        && value.pointer("/params/turn/status").and_then(Value::as_str) == Some("completed")
        && crate::auth_incident::recovery_pending(context, record)
    {
        let recovery_context = context.clone();
        let recovery_record = record.clone();
        if !matches!(
            tokio::task::spawn_blocking(move || crate::auth_incident::recover(
                &recovery_context,
                &recovery_record,
                "healthy",
                &jiff::Timestamp::now().to_string()
            ))
            .await,
            Ok(Ok(()))
        ) {
            eprintln!("warning: provider authentication recovery reporting degraded");
        }
    }
    let Some(failure) = reducer.ingest(value) else {
        return Ok(());
    };
    let context = context.clone();
    let id = record.id.clone();
    let runtime_id = record
        .runtime
        .as_ref()
        .map(|runtime| runtime.launch_id.clone())
        .ok_or_else(|| "Codex runtime identity is missing".to_string())?;
    tokio::task::spawn_blocking(move || {
        crate::activity::ingest_codex_app_server_failure_with_kind(
            &context,
            &id,
            &runtime_id,
            &failure.thread_id,
            &failure.turn_id,
            failure.kind,
        )
    })
    .await
    .map_err(|_| "Codex failure ingestion worker failed".to_string())?
    .map_err(|err| format!("Codex failure ingestion failed: {}", err.code()))?;
    Ok(())
}

async fn wake_from_open_usage(
    context: &CliContext,
    record: &SessionRecord,
    usage: &UsageSnapshot,
) -> Result<(), String> {
    if !usage.authoritative || usage.has_exhausted_windows {
        return Ok(());
    }
    let context = context.clone();
    let id = record.id.clone();
    let runtime_id = record
        .runtime
        .as_ref()
        .map(|runtime| runtime.launch_id.clone())
        .ok_or_else(|| "Codex runtime identity is missing".to_string())?;
    let wake = tokio::task::spawn_blocking(move || {
        crate::auto_resume::wake_scheduled_if_usage_open_for_runtime(
            &context,
            &id,
            &runtime_id,
            Timestamp::now().as_second(),
        )
    })
    .await
    .map_err(|_| "Codex usage wake worker failed".to_string())?;
    wake.map(|_| ())
        .map_err(|err| format!("Codex usage wake failed: {}", err.code()))
}

#[cfg(test)]
mod tests;
