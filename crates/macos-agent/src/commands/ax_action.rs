use crate::backend::process::ProcessRunner;
use crate::backend::{AutoAxBackend, AxBackendAdapter};
use crate::cli::{AxActionPerformArgs, OutputFormat};
use crate::commands::ax_common::{build_selector, build_target, AxSelectorInput};
use crate::error::CliError;
use crate::model::{AxActionPerformRequest, AxActionPerformResult, SuccessEnvelope};
use crate::run::ActionPolicy;

pub fn run_perform(
    format: OutputFormat,
    args: &AxActionPerformArgs,
    policy: ActionPolicy,
    runner: &dyn ProcessRunner,
) -> Result<(), CliError> {
    let request = AxActionPerformRequest {
        target: build_target(
            args.session_id.clone(),
            args.app.clone(),
            args.bundle_id.clone(),
            args.window_title_contains.clone(),
        )?,
        selector: build_selector(AxSelectorInput {
            node_id: args.node_id.clone(),
            role: args.role.clone(),
            title_contains: args.title_contains.clone(),
            identifier_contains: args.identifier_contains.clone(),
            value_contains: args.value_contains.clone(),
            subrole: args.subrole.clone(),
            focused: args.focused,
            enabled: args.enabled,
            nth: args.nth,
        })?,
        name: args.name.clone(),
    };

    let result = if policy.dry_run {
        AxActionPerformResult {
            node_id: request.selector.node_id.clone(),
            matched_count: 0,
            name: request.name.clone(),
            performed: false,
        }
    } else {
        let backend = AutoAxBackend::default();
        backend.action_perform(runner, &request, policy.timeout_ms)?
    };

    match format {
        OutputFormat::Json => {
            let payload = SuccessEnvelope::new("ax.action.perform", result);
            println!(
                "{}",
                serde_json::to_string(&payload).map_err(|err| CliError::runtime(format!(
                    "failed to serialize json output: {err}"
                )))?
            );
        }
        OutputFormat::Text => {
            println!(
                "ax.action.perform\tnode_id={}\tname={}\tmatched_count={}\tperformed={}",
                result.node_id.unwrap_or_default(),
                result.name,
                result.matched_count,
                result.performed
            );
        }
        OutputFormat::Tsv => {
            return Err(CliError::usage(
                "--format tsv is only supported for `windows list` and `apps list`",
            ));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use nils_test_support::{EnvGuard, GlobalStateLock};

    use super::run_perform;
    use crate::backend::process::RealProcessRunner;
    use crate::cli::{AxActionPerformArgs, OutputFormat};
    use crate::run::ActionPolicy;

    fn policy(dry_run: bool) -> ActionPolicy {
        ActionPolicy {
            dry_run,
            retries: 0,
            retry_delay_ms: 150,
            timeout_ms: 1000,
        }
    }

    fn sample_args() -> AxActionPerformArgs {
        AxActionPerformArgs {
            node_id: Some("1.1".to_string()),
            role: None,
            title_contains: None,
            identifier_contains: None,
            value_contains: None,
            subrole: None,
            focused: None,
            enabled: None,
            nth: None,
            session_id: None,
            app: None,
            bundle_id: None,
            window_title_contains: None,
            name: "AXPress".to_string(),
        }
    }

    #[test]
    fn run_perform_dry_run_supports_text_and_json() {
        let lock = GlobalStateLock::new();
        let _mode = EnvGuard::set(&lock, "CODEX_MACOS_AGENT_TEST_MODE", "1");
        let runner = RealProcessRunner;

        run_perform(OutputFormat::Text, &sample_args(), policy(true), &runner)
            .expect("text dry-run should succeed");
        run_perform(OutputFormat::Json, &sample_args(), policy(true), &runner)
            .expect("json dry-run should succeed");
    }

    #[test]
    fn run_perform_rejects_tsv() {
        let lock = GlobalStateLock::new();
        let _mode = EnvGuard::set(&lock, "CODEX_MACOS_AGENT_TEST_MODE", "1");
        let runner = RealProcessRunner;

        let err = run_perform(OutputFormat::Tsv, &sample_args(), policy(true), &runner)
            .expect_err("tsv should be rejected");
        assert!(err.to_string().contains("windows list"));
    }
}
