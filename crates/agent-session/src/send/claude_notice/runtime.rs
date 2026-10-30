//! Fresh-launch module installation and bounded private readiness observations.
use std::fs::{self, DirBuilder, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::json;

use super::{Binding, Launch, PLUGIN, PROVIDER_VERSION, sha256};
use crate::{CliContext, CliError, SessionRecord};

pub(crate) const MODULE: &str = include_str!("register.js");
const BOOT_SCHEMA: &str = "agent-session.claude-notice-runtime.v1";
pub(crate) const READY_SCHEMA: &str = "agent-session.claude-notice-ready.v1";
pub(crate) const OFFER_SCHEMA: &str = "agent-session.claude-notice-offer.v1";

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Bootstrap {
    pub(crate) schema_version: String,
    pub(crate) launch: Launch,
    pub(crate) module_sha256: String,
    pub(crate) socket_path: PathBuf,
    pub(crate) nonce: String,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Ready {
    pub(crate) schema_version: String,
    pub(crate) binding: Binding,
    pub(crate) nonce: String,
    pub(crate) terminal: bool,
    pub(crate) timer_started: bool,
}

pub(crate) fn unavailable() -> CliError {
    CliError::runtime(
        "notice-native-unavailable",
        "native notice capability is unavailable",
        None,
    )
}

pub(crate) fn private_dir(path: &Path) -> Result<(), CliError> {
    match DirBuilder::new().mode(0o700).create(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(_) => return Err(unavailable()),
    }
    let meta = fs::symlink_metadata(path).map_err(|_| unavailable())?;
    // SAFETY: geteuid has no preconditions.
    if !meta.is_dir() || meta.uid() != unsafe { libc::geteuid() } || meta.mode() & 0o777 != 0o700 {
        return Err(unavailable());
    }
    Ok(())
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<(), CliError> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .map_err(|_| unavailable())?;
    file.write_all(bytes).map_err(|_| unavailable())
}

pub(crate) fn private_read(path: &Path, cap: usize) -> Result<Vec<u8>, CliError> {
    let before = fs::symlink_metadata(path).map_err(|_| unavailable())?;
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .map_err(|_| unavailable())?;
    let meta = file.metadata().map_err(|_| unavailable())?;
    // SAFETY: geteuid has no preconditions.
    if !meta.is_file()
        || meta.uid() != unsafe { libc::geteuid() }
        || meta.mode() & 0o777 != 0o600
        || meta.len() > cap as u64
        || (meta.dev(), meta.ino()) != (before.dev(), before.ino())
    {
        return Err(unavailable());
    }
    let mut bytes = Vec::new();
    file.take(cap as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| unavailable())?;
    if bytes.len() > cap {
        return Err(unavailable());
    }
    Ok(bytes)
}

pub(crate) fn module_dir(state: &Path, record: &SessionRecord) -> Result<PathBuf, CliError> {
    let launch = Launch::from_record(record)?;
    crate::validate_id(&launch.session_id)?;
    if launch.incarnation.is_empty()
        || launch.incarnation.len() > 128
        || !launch
            .incarnation
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
        || launch.incarnation == "."
        || launch.incarnation == ".."
    {
        return Err(unavailable());
    }
    Ok(state
        .join("sessions")
        .join(&record.id)
        .join("claude-notice")
        .join(launch.incarnation))
}

fn socket_path(state: &Path, record: &SessionRecord) -> Result<PathBuf, CliError> {
    let root = crate::codex_runtime_dir::runtime_root(state).map_err(|_| unavailable())?;
    private_dir(&root)?;
    let parent = root.join("agent-session");
    private_dir(&parent)?;
    let path = parent.join(socket_name(state, record)?);
    if path.as_os_str().as_encoded_bytes().len() > 96 {
        return Err(unavailable());
    }
    Ok(path)
}

fn socket_name(state: &Path, record: &SessionRecord) -> Result<String, CliError> {
    let namespace = sha256(
        format!(
            "{}\0{}\0{}",
            state.display(),
            record.id,
            crate::coordination::incarnation(record)?
        )
        .as_bytes(),
    );
    Ok(format!("cn-{}.sock", &namespace[..16]))
}

fn existing_private_dir(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|meta| {
        // SAFETY: geteuid has no preconditions.
        meta.is_dir() && meta.uid() == unsafe { libc::geteuid() } && meta.mode() & 0o777 == 0o700
    })
}

pub(crate) fn prepare_launch(
    state: &Path,
    record: &SessionRecord,
    agent_bin: &Path,
) -> Result<Option<PathBuf>, CliError> {
    if record.agent != "claude" || !crate::notice::writers_enabled() {
        return Ok(None);
    }
    let mut command = Command::new(agent_bin);
    command.arg("--version");
    let version =
        crate::run_output_with_timeout_and_strict_cap(command, Duration::from_secs(2), 256);
    if !version.is_ok_and(|output| {
        output.status.success()
            && String::from_utf8_lossy(&output.stdout).trim() == "2.1.296 (Claude Code)"
    }) {
        return Ok(None);
    }
    let launch = Launch::from_record(record)?;
    let dir = module_dir(state, record)?;
    private_dir(dir.parent().ok_or_else(unavailable)?)?;
    private_dir(&dir)?;
    private_dir(&dir.join(".claude-plugin"))?;
    private_dir(&dir.join("hooks"))?;
    let bootstrap = Bootstrap {
        schema_version: BOOT_SCHEMA.into(),
        launch,
        module_sha256: sha256(MODULE.as_bytes()),
        socket_path: socket_path(state, record)?,
        nonce: format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        ),
    };
    for (path, bytes) in [
        (".claude-plugin/plugin.json", serde_json::to_vec(&json!({"name":PLUGIN,"version":"1.0.0","description":"Managed native session notices"})).map_err(|_| unavailable())?),
        ("hooks/hooks.json", b"{\"modules\":[\"./register.js\"]}".to_vec()),
        ("hooks/register.js", MODULE.as_bytes().to_vec()),
        ("bootstrap.json", serde_json::to_vec(&bootstrap).map_err(|_| unavailable())?),
        ("ready.json", serde_json::to_vec(&json!({"schema_version":READY_SCHEMA,"state":"unavailable"})).map_err(|_| unavailable())?),
        ("offer.json", serde_json::to_vec(&json!({"schema_version":OFFER_SCHEMA,"state":"idle"})).map_err(|_| unavailable())?),
    ] { write_new(&dir.join(path), &bytes)?; }
    Ok(Some(dir))
}

pub(crate) fn readiness(
    context: &CliContext,
    record: &SessionRecord,
) -> Option<(Bootstrap, Ready)> {
    if record.agent != "claude" {
        return None;
    }
    let dir = module_dir(&context.state_dir, record).ok()?;
    if !existing_private_dir(&dir) {
        return None;
    }
    let bootstrap: Bootstrap =
        serde_json::from_slice(&private_read(&dir.join("bootstrap.json"), 4096).ok()?).ok()?;
    let ready: Ready =
        serde_json::from_slice(&private_read(&dir.join("ready.json"), 4096).ok()?).ok()?;
    let provider = record.provider_resume.as_ref()?;
    let launch = Launch::from_record(record).ok()?;
    if bootstrap.schema_version != BOOT_SCHEMA
        || bootstrap.launch != launch
        || bootstrap.module_sha256 != sha256(MODULE.as_bytes())
        || !bootstrap.socket_path.is_absolute()
        || bootstrap.socket_path.as_os_str().as_encoded_bytes().len() > 96
        || bootstrap.socket_path.file_name()?.to_str()?
            != socket_name(&context.state_dir, record).ok()?
        || !bootstrap
            .socket_path
            .parent()
            .is_some_and(existing_private_dir)
        || !bootstrap
            .socket_path
            .parent()
            .and_then(Path::parent)
            .is_some_and(existing_private_dir)
        || sha256(&private_read(&dir.join("hooks/register.js"), MODULE.len()).ok()?)
            != bootstrap.module_sha256
        || bootstrap.nonce.len() != 64
        || !bootstrap.nonce.bytes().all(|b| b.is_ascii_hexdigit())
        || ready.schema_version != READY_SCHEMA
        || !ready.terminal
        || !ready.timer_started
        || ready.nonce != bootstrap.nonce
        || ready.binding.launch != launch
        || ready.binding.provider != "claude"
        || provider.provider != "claude"
        || ready.binding.provider_session_id != provider.session_id
        || ready.binding.provider_version != PROVIDER_VERSION
        || ready.binding.module_sha256 != bootstrap.module_sha256
    {
        return None;
    }
    Some((bootstrap, ready))
}
