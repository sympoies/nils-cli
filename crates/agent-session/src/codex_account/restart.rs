use std::path::Path;

use crate::{CliContext, CliError, SessionRecord};

/// Prepare the next Codex runtime and recover durable account intent before
/// launch. The order is observable: account repair must see the runtime that
/// will own the new app-server control connection.
pub(crate) fn prepare_resume_runtime(
    context: &CliContext,
    agent_bin: &Path,
    record: &mut SessionRecord,
    previous_record: &SessionRecord,
) -> Result<(), CliError> {
    super::mark_runtime_pending(record)?;
    crate::codex_app_server::configure_runtime(context, agent_bin, record, true)?;
    if let Err(cause) = super::recover_next_after_restart(context, record) {
        *record = previous_record.clone();
        if let Err(restore) = crate::write_session_record(context, record) {
            return Err(CliError::runtime(
                "codex-account-restart-recovery-rollback-failed",
                "Codex account recovery failed and the prior session record could not be restored",
                Some(serde_json::json!({
                    "id": record.id,
                    "cause": cause.code(),
                    "restore_cause": restore.code(),
                })),
            ));
        }
        return Err(cause);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{fs, os::unix::fs::PermissionsExt};

    use nils_test_support::{EnvGuard, GlobalStateLock};
    use serde_json::json;

    use super::*;

    fn listed_account_broker(dir: &tempfile::TempDir, account: &str) -> String {
        let broker = dir.path().join("account-broker");
        let response = json!({
            "schema_version": super::super::BROKER_SCHEMA_VERSION,
            "accounts": [{"account": account}]
        });
        fs::write(
            &broker,
            format!("#!/bin/sh\n[ \"$1\" = list ] || exit 2\nprintf '%s\\n' '{response}'\n"),
        )
        .unwrap();
        fs::set_permissions(&broker, fs::Permissions::from_mode(0o700)).unwrap();
        serde_json::to_string(&[broker.to_string_lossy()]).unwrap()
    }

    fn capable_codex_cli(dir: &tempfile::TempDir) -> std::path::PathBuf {
        let agent = dir.path().join("codex");
        fs::write(
            &agent,
            "#!/bin/sh\nif [ \"$1\" = --version ]; then printf '%s\\n' 'codex-cli 0.145.0'; exit 0; fi\nif [ \"$1\" = app-server ] && [ \"$2\" = --help ]; then printf '%s\\n' '  --listen <URL>  Supported values: stdio://, unix://PATH'; exit 0; fi\nexit 1\n",
        )
        .unwrap();
        fs::set_permissions(&agent, fs::Permissions::from_mode(0o700)).unwrap();
        agent
    }

    fn failed_binding_with_queued_account() -> SessionRecord {
        let mut record = super::super::record_with_binding_value(json!({
            "schema_version": super::super::BINDING_SCHEMA_VERSION,
            "selected_account": "acct1",
            "revision": 3,
            "state": "failed",
            "failure_reason": "apply_failed",
            "updated_at": "2030-01-01T00:00:00Z"
        }));
        let runtime = record.runtime.as_mut().unwrap();
        runtime.kind = "tmux".to_string();
        runtime.extra.clear();
        super::super::store_next(
            &mut record,
            &super::super::DurableNextAccount {
                schema_version: super::super::NEXT_SCHEMA_VERSION.to_string(),
                account: "acct2".to_string(),
                selection_source: Some("explicit".to_string()),
                revision: 2,
                intent_id: Some("manual-switch".to_string()),
                state: "queued".to_string(),
                applying_runtime_id: None,
                failure_reason: None,
                updated_at: "2030-01-01T00:00:00Z".to_string(),
            },
        )
        .unwrap();
        record
    }

    #[test]
    fn resume_preparation_configures_app_server_before_recovering_explicit_queue() {
        let lock = GlobalStateLock::new();
        let tmp = tempfile::TempDir::new().unwrap();
        let broker_dir = tempfile::TempDir::new().unwrap();
        let _broker = EnvGuard::set(
            &lock,
            super::super::BROKER_ENV,
            &listed_account_broker(&broker_dir, "acct2"),
        );
        let context = CliContext {
            state_dir: tmp.path().join("state"),
            host: None,
        };
        fs::create_dir_all(&context.state_dir).unwrap();
        let mut record = failed_binding_with_queued_account();
        fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
        crate::write_session_record(&context, &record).unwrap();
        let agent = capable_codex_cli(&tmp);

        let previous_record = record.clone();
        prepare_resume_runtime(&context, &agent, &mut record, &previous_record).unwrap();

        assert!(crate::codex_app_server::runtime_is_supported(&record));
        let view = super::super::view_for_record(&record);
        assert_eq!(view.selected_account.as_deref(), Some("acct2"));
        assert_eq!(view.revision, 4);
        assert_eq!(view.state, "pending");
        assert!(view.next.is_none());
        assert_eq!(
            super::super::account_for_control_rebind(&record).unwrap(),
            Some(("acct2".to_string(), 4))
        );
    }

    #[test]
    fn resume_preparation_preserves_failed_binding_and_queue_when_account_is_unlisted() {
        let lock = GlobalStateLock::new();
        let tmp = tempfile::TempDir::new().unwrap();
        let broker_dir = tempfile::TempDir::new().unwrap();
        let _broker = EnvGuard::set(
            &lock,
            super::super::BROKER_ENV,
            &listed_account_broker(&broker_dir, "acct1"),
        );
        let context = CliContext {
            state_dir: tmp.path().join("state"),
            host: None,
        };
        fs::create_dir_all(&context.state_dir).unwrap();
        let mut record = failed_binding_with_queued_account();
        fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
        crate::write_session_record(&context, &record).unwrap();
        let original_binding = record.extra[super::super::BINDING_KEY].clone();
        let original_next = record.extra[super::super::NEXT_KEY].clone();
        let agent = capable_codex_cli(&tmp);

        let previous_record = record.clone();
        let error =
            prepare_resume_runtime(&context, &agent, &mut record, &previous_record).unwrap_err();

        assert_eq!(error.code(), "codex-account-unknown");
        assert_eq!(record.runtime.as_ref().unwrap().kind, "tmux");
        assert_eq!(record.extra[super::super::BINDING_KEY], original_binding);
        assert_eq!(record.extra[super::super::NEXT_KEY], original_next);
        let persisted = crate::load_session_record(&context, &record.id).unwrap();
        assert_eq!(persisted.runtime.as_ref().unwrap().kind, "tmux");
        assert_eq!(persisted.extra[super::super::BINDING_KEY], original_binding);
        assert_eq!(persisted.extra[super::super::NEXT_KEY], original_next);
    }
}
