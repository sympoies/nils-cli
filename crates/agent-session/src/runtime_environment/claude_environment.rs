//! Private, launch-only transport for the Claude environment snapshot.
use std::io::{self, Write};
use std::os::unix::{ffi::OsStrExt, fs::OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant};

use super::provider_facts::CLAUDE_KEYS;
use crate::SessionRecord;

const TRANSPORT_ENV: &str = "AGENT_SESSION_PRIVATE_PROVIDER_ENV";

pub(super) fn prepare(command: &mut Command, state_dir: &Path, record: &SessionRecord) {
    let path = state_dir
        .join("sessions")
        .join(&record.id)
        .join(format!(".provider-env-{}", uuid::Uuid::new_v4()));
    command.env(TRANSPORT_ENV, &path);

    if CLAUDE_KEYS
        .iter()
        .all(|key| std::env::var_os(key).is_none_or(|value| value.is_empty()))
    {
        command.arg("-e").arg(format!("{TRANSPORT_ENV}="));
        for key in CLAUDE_KEYS {
            command.arg("-e").arg(format!("{key}="));
        }
        return;
    }
    let mut assignment = std::ffi::OsString::from(format!("{TRANSPORT_ENV}="));
    assignment.push(path);
    command.arg("-e").arg(assignment);
    for key in CLAUDE_KEYS {
        // Capture now, including absence, rather than using tmux's old values.
        command.env(key, std::env::var_os(key).unwrap_or_default());
    }
}

pub(crate) fn output(mut command: Command, timeout: Duration) -> io::Result<Output> {
    let path = command.get_envs().find_map(|(key, value)| {
        (key == TRANSPORT_ENV)
            .then(|| value.map(PathBuf::from))
            .flatten()
    });
    let Some(path) = path else {
        return crate::run_output_with_timeout(command, timeout);
    };
    command.env_remove(TRANSPORT_ENV);
    // A prior interrupted launch may have left a private transport. The
    // lifecycle lock excludes another launch for this session.
    for entry in std::fs::read_dir(path.parent().expect("session directory"))? {
        let entry = entry?;
        let name = entry.file_name();
        if name
            .to_str()
            .and_then(|name| name.strip_prefix(".provider-env-"))
            .is_some_and(|id| uuid::Uuid::parse_str(id).is_ok())
            && entry.file_type()?.is_file()
        {
            std::fs::remove_file(entry.path())?;
        }
    }
    // Empty snapshots still carry the private cleanup target, but their
    // literal empty tmux assignments need no file or consumption handshake.
    if CLAUDE_KEYS
        .iter()
        .all(|key| !command.get_envs().any(|(name, _)| name == *key))
    {
        return crate::run_output_with_timeout(command, timeout);
    }
    let mut script = Vec::new();
    for key in CLAUDE_KEYS {
        let value = command
            .get_envs()
            .find_map(|(name, value)| (name == key).then_some(value).flatten());
        script.extend_from_slice(format!("export {key}='").as_bytes());
        for byte in value.map_or(&[][..], |value| value.as_bytes()) {
            if *byte == b'\'' {
                script.extend_from_slice(b"'\\''");
            } else {
                script.push(*byte);
            }
        }
        script.extend_from_slice(b"'\n");
    }
    // The lifecycle owner already validated this private session directory.
    // O_EXCL prevents a retained path or symlink from being overwritten.
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)?;
    let result = file.write_all(&script).and_then(|()| {
        drop(file);
        let started = Instant::now();
        let output = crate::run_output_with_timeout(command, timeout)?;
        if output.status.success() {
            while path.exists() && started.elapsed() < timeout {
                std::thread::sleep(Duration::from_millis(5));
            }
            if path.exists() {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "provider environment was not consumed",
                ));
            }
        }
        Ok(output)
    });
    // Successful launches hand unlink ownership to held-launch.sh, which reads
    // and removes the file before waiting for any gate or starting a provider.
    if !result.as_ref().is_ok_and(|output| output.status.success()) {
        let _ = std::fs::remove_file(path);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use nils_test_support::{EnvGuard, GlobalStateLock};
    use pretty_assertions::assert_eq;

    #[test]
    fn successful_tmux_without_consumption_fails_and_cleans_current_and_stale_transports() {
        let lock = GlobalStateLock::new();
        let _clear: Vec<_> = CLAUDE_KEYS
            .iter()
            .map(|key| EnvGuard::remove(&lock, key))
            .collect();
        let _auth = EnvGuard::set(&lock, "ANTHROPIC_AUTH_TOKEN", "synthetic-auth-canary");
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("sessions/fixture");
        std::fs::create_dir_all(&dir).unwrap();
        let stale = dir.join(format!(".provider-env-{}", uuid::Uuid::new_v4()));
        std::fs::write(&stale, "synthetic-stale-canary").unwrap();
        let record: SessionRecord = serde_json::from_value(serde_json::json!({
            "schema_version":crate::SESSION_DOCUMENT_VERSION, "id":"fixture", "agent":"claude",
            "mode":"interactive", "cwd":"/workspace/project", "tmux_session":"fixture",
            "created_at":"2030-01-01T00:00:00Z", "updated_at":"2030-01-01T00:00:00Z"
        }))
        .unwrap();
        // This synthetic tmux succeeds without starting any pane/consumer.
        let mut command = Command::new("true");
        prepare(&mut command, tmp.path(), &record);
        assert_eq!(
            output(command, Duration::from_millis(100))
                .unwrap_err()
                .kind(),
            io::ErrorKind::TimedOut
        );
        assert_eq!(std::fs::read_dir(dir).unwrap().count(), 0);
    }
    #[test]
    fn empty_environment_launch_removes_stale_transport_files() {
        let lock = GlobalStateLock::new();
        let _clear: Vec<_> = CLAUDE_KEYS
            .iter()
            .map(|key| EnvGuard::remove(&lock, key))
            .collect();
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("sessions/fixture");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(format!(".provider-env-{}", uuid::Uuid::new_v4())),
            "synthetic-stale-canary",
        )
        .unwrap();
        let record: SessionRecord = serde_json::from_value(serde_json::json!({
            "schema_version":crate::SESSION_DOCUMENT_VERSION, "id":"fixture", "agent":"claude",
            "mode":"interactive", "cwd":"/workspace/project", "tmux_session":"fixture",
            "created_at":"2030-01-01T00:00:00Z", "updated_at":"2030-01-01T00:00:00Z"
        }))
        .unwrap();
        let mut command = Command::new("true");
        prepare(&mut command, tmp.path(), &record);
        assert!(
            output(command, Duration::from_secs(1))
                .unwrap()
                .status
                .success()
        );
        assert_eq!(std::fs::read_dir(dir).unwrap().count(), 0);
    }
}
