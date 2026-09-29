//! Provider-neutral access-only remote auth transport.
//!
//! A token authority host keeps each provider's refresh-capable credentials.
//! Replicas run `ssh <authority> <provider-cli> auth remote export ...`, and
//! this module owns the shared part of that exchange: argument validation, the
//! SSH invocation, the optional refresh-then-fallback retry, JSON parsing, and
//! the access-only enforcement. Each provider supplies an
//! [`AccessOnlyAdapter`] that knows its credential shape.

use serde_json::Value;
use std::process::{Command, Output};

/// Provider-specific knowledge the shared remote transport needs.
pub trait AccessOnlyAdapter {
    /// Message prefix, for example `codex-remote-pull`.
    fn log_prefix(&self) -> &str;

    /// Remote command run over SSH for one export, excluding the SSH host.
    fn export_command(&self, selector: &RemoteSelector, refresh: bool) -> Vec<String>;

    /// Keep only the fields a replica may hold. Must drop every refresh token.
    fn sanitize_access_only(&self, value: Value) -> Value;

    /// Whether a sanitized payload carries a usable access token.
    fn has_access_token(&self, value: &Value) -> bool;
}

/// Which stored credential the authority should export.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteSelector {
    /// A named stored secret.
    Name(String),
    /// The authority's current default.
    Current,
}

impl RemoteSelector {
    pub fn describe(&self) -> &str {
        match self {
            Self::Name(name) => name,
            Self::Current => "current",
        }
    }
}

#[derive(Debug, Clone)]
pub struct RemotePullFailure {
    pub code: &'static str,
    pub message: String,
    pub details: Option<Value>,
    pub exit_code: i32,
}

/// A sanitized access-only payload fetched from the authority.
#[derive(Debug, Clone)]
pub struct RemoteExport {
    pub value: Value,
    pub refresh_attempted: bool,
    pub refresh_fallback: bool,
    pub refresh_error_code: Option<String>,
}

/// Fetch and sanitize an access-only payload from `ssh_host`.
///
/// When `refresh` is requested and the refreshing export fails, one plain
/// export is attempted and reported as a refresh fallback.
pub fn fetch_access_only(
    adapter: &dyn AccessOnlyAdapter,
    ssh_host: &str,
    selector: &RemoteSelector,
    refresh: bool,
) -> Result<RemoteExport, RemotePullFailure> {
    let (output, refresh_fallback, refresh_error_code) =
        match run_export(adapter, ssh_host, selector, refresh)? {
            output if output.status.success() => (output, false, None),
            output => {
                let primary = export_status_failure(adapter, ssh_host, selector, &output);
                if !refresh {
                    return Err(primary);
                }
                match run_export(adapter, ssh_host, selector, false) {
                    Ok(fallback) if fallback.status.success() => {
                        (fallback, true, Some(primary.code.to_string()))
                    }
                    Ok(_) | Err(_) => return Err(primary),
                }
            }
        };

    let value = sanitize_output(adapter, &output, ssh_host, selector)?;
    Ok(RemoteExport {
        value,
        refresh_attempted: refresh,
        refresh_fallback,
        refresh_error_code,
    })
}

fn run_export(
    adapter: &dyn AccessOnlyAdapter,
    ssh_host: &str,
    selector: &RemoteSelector,
    refresh: bool,
) -> Result<Output, RemotePullFailure> {
    let mut command = Command::new("ssh");
    command
        .arg(ssh_host)
        .args(adapter.export_command(selector, refresh));
    command.output().map_err(|err| RemotePullFailure {
        code: "ssh-exec-failed",
        message: format!("{}: failed to run ssh: {err}", adapter.log_prefix()),
        details: None,
        exit_code: 1,
    })
}

fn export_status_failure(
    adapter: &dyn AccessOnlyAdapter,
    ssh_host: &str,
    selector: &RemoteSelector,
    output: &Output,
) -> RemotePullFailure {
    let exit_code = output.status.code().unwrap_or(1);
    RemotePullFailure {
        code: "remote-export-failed",
        message: format!(
            "{}: remote export failed (exit {exit_code})",
            adapter.log_prefix()
        ),
        details: Some(serde_json::json!({
            "ssh": ssh_host,
            "name": selector.describe(),
            "exit_code": exit_code,
        })),
        exit_code: 1,
    }
}

fn sanitize_output(
    adapter: &dyn AccessOnlyAdapter,
    output: &Output,
    ssh_host: &str,
    selector: &RemoteSelector,
) -> Result<Value, RemotePullFailure> {
    let details = || {
        Some(serde_json::json!({
            "ssh": ssh_host,
            "name": selector.describe(),
        }))
    };
    let imported: Value =
        serde_json::from_slice(&output.stdout).map_err(|_| RemotePullFailure {
            code: "remote-export-invalid-json",
            message: format!(
                "{}: remote export returned invalid JSON",
                adapter.log_prefix()
            ),
            details: details(),
            exit_code: 1,
        })?;
    let imported = adapter.sanitize_access_only(imported);
    if !adapter.has_access_token(&imported) {
        return Err(RemotePullFailure {
            code: "remote-export-missing-access-token",
            message: format!(
                "{}: remote export did not include an OAuth access token",
                adapter.log_prefix()
            ),
            details: details(),
            exit_code: 1,
        });
    }
    Ok(imported)
}

/// An SSH destination that cannot be read as an option or shell syntax.
pub fn is_valid_ssh_host(host: &str) -> bool {
    !host.is_empty()
        && !host.starts_with('-')
        && !host
            .chars()
            .any(|ch| ch.is_whitespace() || matches!(ch, '\'' | '"' | '`' | '$' | ';' | '&' | '|'))
}

/// A stored secret name: a single path segment of `[A-Za-z0-9._-]`.
pub fn is_valid_secret_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('-')
        && !name.contains("..")
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use serde_json::json;

    struct FakeAdapter {
        script: String,
    }

    impl AccessOnlyAdapter for FakeAdapter {
        fn log_prefix(&self) -> &str {
            "fake-remote-pull"
        }

        fn export_command(&self, selector: &RemoteSelector, refresh: bool) -> Vec<String> {
            vec![
                self.script.clone(),
                selector.describe().to_string(),
                refresh.to_string(),
            ]
        }

        fn sanitize_access_only(&self, value: Value) -> Value {
            json!({ "access": value.get("access").cloned().unwrap_or(Value::Null) })
        }

        fn has_access_token(&self, value: &Value) -> bool {
            value
                .get("access")
                .and_then(Value::as_str)
                .is_some_and(|value| !value.is_empty())
        }
    }

    /// A fake `ssh` on PATH that drops the host and runs the remote command.
    struct FakeSsh {
        _dir: tempfile::TempDir,
        _path: nils_test_support::EnvGuard,
        script: String,
    }

    fn fake_ssh(lock: &nils_test_support::GlobalStateLock, remote_body: &str) -> FakeSsh {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::TempDir::new().expect("tempdir");
        let ssh = dir.path().join("ssh");
        std::fs::write(&ssh, "#!/bin/sh\nshift\nexec \"$@\"\n").expect("ssh");
        let remote = dir.path().join("remote-export");
        std::fs::write(&remote, format!("#!/bin/sh\n{remote_body}\n")).expect("remote");
        for path in [&ssh, &remote] {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        }
        let path_value = format!(
            "{}:{}",
            dir.path().display(),
            std::env::var("PATH").unwrap_or_default()
        );
        let path = nils_test_support::EnvGuard::set(lock, "PATH", &path_value);
        FakeSsh {
            script: remote.display().to_string(),
            _dir: dir,
            _path: path,
        }
    }

    #[test]
    fn remote_fetch_sanitizes_the_exported_payload() {
        let lock = nils_test_support::GlobalStateLock::new();
        let ssh = fake_ssh(
            &lock,
            r#"printf '{"access":"a-token","refresh":"r-token","who":"%s"}' "$1""#,
        );
        let adapter = FakeAdapter { script: ssh.script };

        let export =
            fetch_access_only(&adapter, "authority", &RemoteSelector::Current, false).expect("ok");

        assert_eq!(export.value, json!({ "access": "a-token" }));
        assert!(!export.refresh_attempted);
        assert!(!export.refresh_fallback);
    }

    #[test]
    fn remote_fetch_falls_back_to_a_plain_export_when_refresh_fails() {
        let lock = nils_test_support::GlobalStateLock::new();
        let ssh = fake_ssh(
            &lock,
            r#"[ "$2" = true ] && exit 7; printf '{"access":"a-token"}'"#,
        );
        let adapter = FakeAdapter { script: ssh.script };

        let export = fetch_access_only(
            &adapter,
            "authority",
            &RemoteSelector::Name("team".to_string()),
            true,
        )
        .expect("fallback");

        assert!(export.refresh_attempted);
        assert!(export.refresh_fallback);
        assert_eq!(
            export.refresh_error_code.as_deref(),
            Some("remote-export-failed")
        );
    }

    #[test]
    fn remote_fetch_rejects_invalid_json_and_missing_access_tokens() {
        let lock = nils_test_support::GlobalStateLock::new();
        let ssh = fake_ssh(&lock, "printf 'not json'");
        let adapter = FakeAdapter {
            script: ssh.script.clone(),
        };
        let err = fetch_access_only(&adapter, "authority", &RemoteSelector::Current, false)
            .expect_err("invalid json");
        assert_eq!(err.code, "remote-export-invalid-json");
        assert_eq!(
            err.message,
            "fake-remote-pull: remote export returned invalid JSON"
        );
        drop(ssh);

        let ssh = fake_ssh(&lock, r#"printf '{"refresh":"r-token"}'"#);
        let adapter = FakeAdapter { script: ssh.script };
        let err = fetch_access_only(&adapter, "authority", &RemoteSelector::Current, false)
            .expect_err("missing access");
        assert_eq!(err.code, "remote-export-missing-access-token");
    }

    #[test]
    fn remote_fetch_reports_a_failed_export_without_retry_when_not_refreshing() {
        let lock = nils_test_support::GlobalStateLock::new();
        let ssh = fake_ssh(&lock, "exit 3");
        let adapter = FakeAdapter { script: ssh.script };

        let err = fetch_access_only(
            &adapter,
            "authority",
            &RemoteSelector::Name("team".to_string()),
            false,
        )
        .expect_err("export failed");

        assert_eq!(err.code, "remote-export-failed");
        assert_eq!(
            err.details,
            Some(json!({ "ssh": "authority", "name": "team", "exit_code": 3 }))
        );
    }

    #[test]
    fn remote_validates_ssh_hosts_and_secret_names() {
        assert!(is_valid_ssh_host("operator@auth-host"));
        assert!(!is_valid_ssh_host("-oProxyCommand=bad"));
        assert!(!is_valid_ssh_host("auth-host;bad"));
        assert!(!is_valid_ssh_host("auth host"));

        assert!(is_valid_secret_name("team.json"));
        assert!(!is_valid_secret_name("../bad"));
        assert!(!is_valid_secret_name("a/b"));
        assert!(!is_valid_secret_name("-bad"));
        assert!(!is_valid_secret_name("a$bad"));
    }
}
