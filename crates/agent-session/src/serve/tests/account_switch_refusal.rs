//! Switch preflight refusals retain typed HTTP diagnostics and the live runtime.
use super::*;
use pretty_assertions::assert_eq;

#[tokio::test]
async fn claude_switch_http_preserves_preflight_cause_and_reason_without_stopping() {
    let lock = GlobalStateLock::new();
    for failure in [
        "wrong_schema",
        "wrong_provider",
        "malformed_list",
        "invalid_capabilities",
        "missing_accounts",
        "duplicate_account",
        "invalid_account",
        "invalid_metadata",
        "unavailable_list",
        "materialize",
        "unsafe_directory",
        "missing_publication",
        "invalid_publication",
    ] {
        let tmp = tempfile::TempDir::new().unwrap();
        let (st, broker, record, _probe) = idle_account_fixture(&lock, tmp.path());
        let mut inventory = json!({
            "schema_version": "agent-session.account-broker.v2", "provider": "claude",
            "accounts": [{"account": "alpha"}, {"account": "beta"}]
        });
        match failure {
            "wrong_schema" => inventory["schema_version"] = json!("invalid"),
            "wrong_provider" => inventory["provider"] = json!("codex"),
            "invalid_capabilities" => inventory["capabilities"] = json!(null),
            "missing_accounts" => {
                inventory.as_object_mut().unwrap().remove("accounts");
            }
            "duplicate_account" => {
                inventory["accounts"] = json!([{"account": "beta"}, {"account": "beta"}]);
            }
            "invalid_account" => {
                inventory["accounts"][0]["account"] = json!("../alpha");
            }
            "invalid_metadata" => {
                inventory["accounts"][0]["label"] = json!("invalid\nlabel");
            }
            "unsafe_directory" => {
                fs::set_permissions(broker.config_dir("beta"), fs::Permissions::from_mode(0o777))
                    .unwrap()
            }
            "missing_publication" | "invalid_publication" => {
                let mut publication = crate::claude_account::generation::tests::envelope(1);
                publication["account"] = json!("beta");
                if failure == "missing_publication" {
                    publication["state"]["publication"]["status"] = json!("missing");
                } else {
                    publication["state"]["publication"]["stores"][0]["status"] = json!("pending");
                }
                fs::write(
                    tmp.path().join("beta-publication.json"),
                    publication.to_string(),
                )
                .unwrap();
            }
            _ => {}
        }
        let invalid_inventory = matches!(
            failure,
            "wrong_schema"
                | "wrong_provider"
                | "malformed_list"
                | "invalid_capabilities"
                | "missing_accounts"
                | "duplicate_account"
                | "invalid_account"
                | "invalid_metadata"
                | "unavailable_list"
        );
        let _override = if invalid_inventory {
            fs::write(
                tmp.path().join("inventory.json"),
                if failure == "malformed_list" {
                    "invalid JSON".into()
                } else {
                    inventory.to_string()
                },
            )
            .unwrap();
            // Queue admission sees a valid list. The preflight list after valid
            // materialization fails, so this exercises the capability decision.
            let root = shell_words::quote(&tmp.path().to_string_lossy()).into_owned();
            let script = executable(
                &tmp.path().join("inventory-broker"),
                &format!(
                    r#"#!/bin/sh
if [ "$1" = list ] && [ -f {root}/materialized ]; then
  cat {root}/inventory.json
  exit 0
fi
if [ "$1" = materialize ]; then touch {root}/materialized; {unavailable} fi
exec {root}/claude-broker "$@"
"#,
                    unavailable = if failure == "unavailable_list" {
                        "chmod 600 \"$0\";"
                    } else {
                        ""
                    }
                ),
            );
            Some(EnvGuard::set(
                &lock,
                "AGENT_SESSION_CLAUDE_ACCOUNT_BROKER",
                &json!([script]).to_string(),
            ))
        } else {
            None
        };
        for (id, kind) in [("start", "turn_started"), ("done", "turn_completed")] {
            crate::activity::ingest_event(
                &st.context,
                &record.id,
                idle_account_event(&record, id, kind),
            )
            .unwrap();
        }
        let account = if failure == "materialize" {
            "gamma"
        } else {
            "beta"
        };
        let request = json!({"account": account, "expected_session_incarnation": record.runtime.as_ref().unwrap().launch_id});
        let path = crate::session_dir(&st.context, &record.id).join("session.json");
        let before = fs::read(&path).unwrap();
        let (unauthorized, _) = call(
            router(st.clone()),
            put_json("/sessions/idle-switch/account", None, request.clone()),
        )
        .await;
        assert_eq!(unauthorized, StatusCode::UNAUTHORIZED);
        assert!(broker.calls().is_empty());
        let (status, body) = call(
            router(st.clone()),
            put_json("/sessions/idle-switch/account", Some(TOKEN), request),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{failure}: {body}"
        );
        assert_eq!(
            body["error"]["code"], "claude-account-switch-refused",
            "{failure}: {body}"
        );
        let cause = match failure {
            "materialize" => "claude-account-broker-rejected",
            "unavailable_list" => "claude-account-broker-unavailable",
            "unsafe_directory" => "claude-account-dir-unsafe",
            "missing_publication" => "claude-account-credentials-unverified",
            "invalid_publication" => "claude-publication-unavailable",
            _ => "claude-account-broker-invalid-response",
        };
        assert_eq!(
            body["error"]["details"]["cause"], cause,
            "{failure}: {body}"
        );
        if failure == "unsafe_directory" {
            assert_eq!(body["error"]["details"]["reason"], "world_writable");
        } else if failure == "invalid_publication" {
            assert_eq!(body["error"]["details"]["reason"], "invalid_response");
        }
        assert_eq!(fs::read(&path).unwrap(), before, "{failure}");
        assert!(broker.calls().contains(&format!(
            "materialize --provider claude --account {account}"
        )));
        let log = fs::read_to_string(tmp.path().join("tmux.log")).unwrap_or_default();
        assert!(
            !log.contains("new-session")
                && !log.contains("if-shell")
                && !log.contains("kill-session"),
            "{failure}: {log}"
        );
        let encoded = body.to_string();
        assert!(!encoded.contains(&broker.config_dir("beta").to_string_lossy().to_string()));
        assert!(!encoded.contains(".credentials.json"));
    }
}
