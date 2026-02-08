use std::time::Instant;

use crate::backend::process::ProcessRunner;
use crate::backend::{AutoAxBackend, AxBackendAdapter};
use crate::cli::{AxActionPerformArgs, OutputFormat};
use crate::commands::ax_common::{build_selector_from_args, build_target_from_args};
use crate::commands::{emit_json_success, reject_tsv_for_list_only};
use crate::error::CliError;
use crate::model::{AxActionPerformCommandResult, AxActionPerformRequest, AxActionPerformResult};
use crate::retry::run_with_retry;
use crate::run::{
    action_policy_result, build_action_meta_with_attempts, next_action_id, ActionPolicy,
};

pub fn run_perform(
    format: OutputFormat,
    args: &AxActionPerformArgs,
    policy: ActionPolicy,
    runner: &dyn ProcessRunner,
) -> Result<(), CliError> {
    let request = AxActionPerformRequest {
        target: build_target_from_args(&args.target)?,
        selector: build_selector_from_args(&args.selector)?,
        name: args.name.clone(),
    };

    let action_id = next_action_id("ax.action.perform");
    let started = Instant::now();
    let mut attempts_used = 0u8;
    let mut detail = AxActionPerformResult {
        node_id: request.selector.node_id.clone(),
        matched_count: 0,
        name: request.name.clone(),
        performed: false,
    };

    if !policy.dry_run {
        let backend = AutoAxBackend::default();
        let retry = policy.retry_policy();
        let (backend_result, attempts) = run_with_retry(retry, || {
            backend.action_perform(runner, &request, policy.timeout_ms)
        })?;
        attempts_used = attempts;
        detail = backend_result;
    }

    let result = AxActionPerformCommandResult {
        detail,
        policy: action_policy_result(policy),
        meta: build_action_meta_with_attempts(action_id, started, policy, attempts_used),
    };

    match format {
        OutputFormat::Json => {
            emit_json_success("ax.action.perform", result)?;
        }
        OutputFormat::Text => {
            println!(
                "ax.action.perform\tnode_id={}\tname={}\tmatched_count={}\tperformed={}\taction_id={}\telapsed_ms={}",
                result.detail.node_id.clone().unwrap_or_default(),
                result.detail.name,
                result.detail.matched_count,
                result.detail.performed,
                result.meta.action_id,
                result.meta.elapsed_ms,
            );
        }
        OutputFormat::Tsv => {
            return reject_tsv_for_list_only();
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
            selector: crate::cli::AxSelectorArgs {
                node_id: Some("1.1".to_string()),
                ..crate::cli::AxSelectorArgs::default()
            },
            target: crate::cli::AxTargetArgs::default(),
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
