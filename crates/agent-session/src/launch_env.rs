//! Public, non-secret launch switches shared by every provider lifecycle.
use crate::{CliError, SessionRecord};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) const ALLOWLIST_ENV: &str = "AGENT_SESSION_LAUNCH_ENV_ALLOWLIST";
const DEFAULT_KEYS: &[&str] = &[
    "AGENT_RUNTIME_SUPPRESS_MEMORY",
    "AGENT_RUNTIME_SUPPRESS_HEALTH",
    "AGENT_RUNTIME_SUPPRESS_PREFLIGHT",
    "AGENT_RUNTIME_SUPPRESS_FINISH_GATE",
];
const MAX_KEYS: usize = 16;
const MAX_VALUE_BYTES: usize = 1024;
pub(crate) type LaunchEnv = BTreeMap<String, String>;

fn safe_key(key: &str) -> bool {
    key.len() <= 128
        && key.starts_with("AGENT_RUNTIME_")
        && key.len() > "AGENT_RUNTIME_".len()
        && key
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
        && ![
            "SECRET",
            "TOKEN",
            "PASSWORD",
            "PASSWD",
            "CREDENTIAL",
            "API_KEY",
            "PRIVATE_KEY",
        ]
        .iter()
        .any(|word| key.contains(word))
}

fn allowlist() -> Result<BTreeSet<String>, CliError> {
    let keys: Vec<String> = match std::env::var(ALLOWLIST_ENV) {
        Ok(raw) => serde_json::from_str(&raw).map_err(|_| invalid_config())?,
        Err(std::env::VarError::NotPresent) => {
            DEFAULT_KEYS.iter().map(|key| (*key).into()).collect()
        }
        Err(_) => return Err(invalid_config()),
    };
    if keys.len() > MAX_KEYS || keys.iter().any(|key| !safe_key(key)) {
        return Err(invalid_config());
    }
    Ok(keys.into_iter().collect())
}

fn invalid_config() -> CliError {
    CliError::usage(
        "launch-env-allowlist-invalid",
        "launch env allowlist must be a JSON array of at most 16 non-secret AGENT_RUNTIME_ switch names",
        None,
    )
}

pub(crate) fn validate(values: &LaunchEnv) -> Result<(), CliError> {
    if values.is_empty() {
        return Ok(());
    }
    if values.len() > MAX_KEYS {
        return Err(CliError::usage(
            "launch-env-too-many",
            "at most 16 launch env switches are accepted",
            None,
        ));
    }
    let allowed = allowlist()?;
    for (key, value) in values {
        if !safe_key(key) || !allowed.contains(key) {
            // Never echo the rejected assignment: it may contain a credential.
            return Err(CliError::usage(
                "launch-env-key-refused",
                "launch env key is not an approved non-secret runtime switch",
                None,
            ));
        }
        if value.len() > MAX_VALUE_BYTES || value.chars().any(char::is_control) {
            return Err(CliError::usage(
                "launch-env-value-invalid",
                "launch env values must be at most 1024 bytes without control characters",
                None,
            ));
        }
    }
    Ok(())
}

pub(crate) fn from_flags(assignments: &[String]) -> Result<LaunchEnv, CliError> {
    if assignments.len() > MAX_KEYS {
        return Err(CliError::usage(
            "launch-env-too-many",
            "at most 16 launch env switches are accepted",
            None,
        ));
    }
    let mut values = LaunchEnv::new();
    for assignment in assignments {
        let (key, value) = assignment.split_once('=').ok_or_else(|| {
            CliError::usage(
                "launch-env-assignment-invalid",
                "launch env must use KEY=VALUE",
                None,
            )
        })?;
        if values.insert(key.into(), value.into()).is_some() {
            return Err(CliError::usage(
                "launch-env-key-duplicate",
                "launch env keys must be unique",
                None,
            ));
        }
    }
    validate(&values)?;
    Ok(values)
}

pub(crate) fn from_record(record: &SessionRecord) -> Result<LaunchEnv, CliError> {
    from_json(record.extra.get("launch_env"))
}

pub(crate) fn from_json(value: Option<&Value>) -> Result<LaunchEnv, CliError> {
    let values = match value {
        None => LaunchEnv::new(),
        Some(value) => serde_json::from_value(value.clone()).map_err(|_| {
            CliError::usage(
                "launch-env-invalid",
                "launch env must be an object of switch names and string values",
                None,
            )
        })?,
    };
    validate(&values)?;
    Ok(values)
}

pub(crate) fn store(record: &mut SessionRecord, values: &LaunchEnv) {
    if !values.is_empty() {
        record.extra.insert("launch_env".into(), json!(values));
    }
}

pub(crate) fn projection(value: Option<&Value>) -> LaunchEnv {
    from_json(value).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use nils_test_support::{EnvGuard, GlobalStateLock};
    use pretty_assertions::assert_eq;

    #[test]
    fn launch_env_assignments_keep_literal_values_and_reject_duplicates() {
        let lock = GlobalStateLock::new();
        let _policy = EnvGuard::remove(&lock, ALLOWLIST_ENV);
        let values =
            from_flags(&["AGENT_RUNTIME_SUPPRESS_MEMORY=1=literal $(false)".into()]).unwrap();
        assert_eq!(
            values["AGENT_RUNTIME_SUPPRESS_MEMORY"],
            "1=literal $(false)"
        );
        for (args, code) in [
            (
                vec!["AGENT_RUNTIME_SUPPRESS_MEMORY".into()],
                "launch-env-assignment-invalid",
            ),
            (
                vec![
                    "AGENT_RUNTIME_SUPPRESS_MEMORY=1".into(),
                    "AGENT_RUNTIME_SUPPRESS_MEMORY=0".into(),
                ],
                "launch-env-key-duplicate",
            ),
            (
                vec!["AGENT_RUNTIME_SUPPRESS_MEMORY=bad\nvalue".into()],
                "launch-env-value-invalid",
            ),
        ] {
            assert_eq!(from_flags(&args).unwrap_err().code(), code);
        }
    }

    #[test]
    fn launch_env_is_present_in_each_provider_process() {
        let lock = GlobalStateLock::new();
        let _policy = EnvGuard::remove(&lock, ALLOWLIST_ENV);
        use std::{
            path::PathBuf,
            process::Command,
            time::{Duration, Instant},
        };
        let Some(tmux) = crate::binary_on_path("tmux") else {
            eprintln!("tmux unavailable; skipping isolated provider-process fixture");
            return;
        };
        struct Server {
            tmux: PathBuf,
            socket: PathBuf,
        }
        impl Drop for Server {
            fn drop(&mut self) {
                let _ = Command::new(&self.tmux)
                    .arg("-S")
                    .arg(&self.socket)
                    .arg("kill-server")
                    .output();
            }
        }
        for agent in ["codex", "claude", "dsh"] {
            let tmp = tempfile::TempDir::new().unwrap();
            let output = tmp.path().join("provider-env");
            let server = Server {
                tmux: tmux.clone(),
                socket: tmp.path().join("tmux.sock"),
            };
            let record: SessionRecord = serde_json::from_value(json!({
                "schema_version":crate::SESSION_DOCUMENT_VERSION, "id":"env-fixture", "agent":agent,
                "mode":"interactive", "title":null, "cwd":tmp.path(), "tmux_session":"env-fixture",
                "prompt_file":null, "log_file":null, "created_at":"2030-01-01T00:00:00Z", "updated_at":"2030-01-01T00:00:00Z",
                "launch_env":{"AGENT_RUNTIME_SUPPRESS_MEMORY":"1"},
                "runtime":{"kind":"tmux", "tmux_session":"env-fixture", "generation":1, "started_at":"2030-01-01T00:00:00Z", "launch_id":"fixture-runtime"}
            })).unwrap();
            let mut command = Command::new(&server.tmux);
            command
                .args(["-f", "/dev/null", "-S"])
                .arg(&server.socket)
                .args(["new-session", "-d", "-s", "env-fixture"])
                .env("AGENT_RUNTIME_SUPPRESS_MEMORY", "0");
            crate::add_runtime_tmux_environment(&mut command, tmp.path(), &record).unwrap();
            let launched = command
                .args([
                    "sh",
                    "-c",
                    "printf '%s' \"$AGENT_RUNTIME_SUPPRESS_MEMORY\" > \"$1\"; exec sleep 10",
                    "provider-fixture",
                ])
                .arg(&output)
                .output()
                .unwrap();
            assert!(
                launched.status.success(),
                "{agent}: {}",
                String::from_utf8_lossy(&launched.stderr)
            );
            let deadline = Instant::now() + Duration::from_secs(5);
            while std::fs::read_to_string(&output).ok().as_deref() != Some("1")
                && Instant::now() < deadline
            {
                std::thread::sleep(Duration::from_millis(10));
            }
            assert_eq!(std::fs::read_to_string(&output).unwrap(), "1", "{agent}");
        }
    }

    #[test]
    fn launch_env_projection_never_exports_unapproved_stored_values() {
        assert_eq!(
            projection(Some(&json!({"API_TOKEN":"private-fixture"}))),
            LaunchEnv::new()
        );
        assert_eq!(projection(None), LaunchEnv::new());
    }
}
