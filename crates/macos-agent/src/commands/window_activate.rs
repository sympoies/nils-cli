use std::time::Instant;

use crate::backend::applescript::{self, ActivationTarget};
use crate::backend::process::ProcessRunner;
use crate::cli::{OutputFormat, WindowActivateArgs};
use crate::error::CliError;
use crate::model::{SuccessEnvelope, WindowActivateResult};
use crate::retry::run_with_retry;
use crate::run::{action_policy_result, build_action_meta, next_action_id, ActionPolicy};
use crate::targets::{self, TargetSelector};
use crate::test_mode;
use crate::wait;

pub fn run(
    format: OutputFormat,
    args: &WindowActivateArgs,
    policy: ActionPolicy,
    runner: &dyn ProcessRunner,
) -> Result<(), CliError> {
    let (target, selected_app, selected_window_id) = resolve_target(args)?;
    let action_id = next_action_id("window.activate");
    let started = Instant::now();

    if !policy.dry_run {
        let retry = policy.retry_policy();
        run_with_retry(retry, || {
            applescript::activate(runner, &target, policy.timeout_ms)
        })?;

        if let Some(wait_ms) = args.wait_ms {
            wait_for_active_confirmation(runner, &target, wait_ms)?;
        }
    }

    let result = WindowActivateResult {
        selected_app,
        selected_window_id,
        wait_ms: args.wait_ms,
        policy: action_policy_result(policy),
        meta: build_action_meta(action_id, started, policy),
    };

    match format {
        OutputFormat::Json => {
            let payload = SuccessEnvelope::new("window.activate", result);
            println!(
                "{}",
                serde_json::to_string(&payload).map_err(|err| CliError::runtime(format!(
                    "failed to serialize json output: {err}"
                )))?
            );
        }
        OutputFormat::Text => {
            println!(
                "window.activate\taction_id={}\tapp={}\twindow_id={}\telapsed_ms={}",
                result.meta.action_id,
                result.selected_app,
                result
                    .selected_window_id
                    .map(|id| id.to_string())
                    .unwrap_or_else(|| "-".to_string()),
                result.meta.elapsed_ms,
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

fn resolve_target(
    args: &WindowActivateArgs,
) -> Result<(ActivationTarget, String, Option<u32>), CliError> {
    if let Some(bundle_id) = args.bundle_id.as_ref() {
        return Ok((
            ActivationTarget::BundleId(bundle_id.clone()),
            bundle_id.clone(),
            None,
        ));
    }

    if let Some(app) = args.app.as_ref() {
        return Ok((ActivationTarget::App(app.clone()), app.clone(), None));
    }

    let selector = TargetSelector {
        window_id: args.window_id,
        active_window: args.active_window,
        app: args.app.clone(),
        window_name: args.window_name.clone(),
    };

    let window = targets::resolve_window(&selector).map_err(|err| {
        CliError::runtime(format!(
            "window activate failed for selector `{}`: {}; try --window-id <id> or --app <name> --window-name <title>",
            selector_label(args),
            err
        ))
    })?;

    Ok((
        ActivationTarget::App(window.owner_name.clone()),
        window.owner_name,
        Some(window.id),
    ))
}

fn selector_label(args: &WindowActivateArgs) -> String {
    if let Some(window_id) = args.window_id {
        return format!("--window-id {window_id}");
    }
    if args.active_window {
        return "--active-window".to_string();
    }
    if let Some(app) = args.app.as_deref() {
        if let Some(window_name) = args.window_name.as_deref() {
            return format!("--app {app} --window-name {window_name}");
        }
        return format!("--app {app}");
    }
    if let Some(bundle_id) = args.bundle_id.as_deref() {
        return format!("--bundle-id {bundle_id}");
    }
    "<unknown-selector>".to_string()
}

fn wait_for_active_confirmation(
    runner: &dyn ProcessRunner,
    target: &ActivationTarget,
    wait_ms: u64,
) -> Result<(), CliError> {
    if wait_ms == 0 {
        return Ok(());
    }

    if test_mode::enabled() {
        wait::sleep_ms(wait_ms.min(10));
        return Ok(());
    }

    wait::wait_until("window activation", wait_ms, 50, || match target {
        ActivationTarget::App(app) => applescript::frontmost_app_name(runner, wait_ms)
            .map(|frontmost| frontmost.eq_ignore_ascii_case(app)),
        ActivationTarget::BundleId(bundle_id) => targets::app_active_by_bundle_id(bundle_id),
    })
    .map(|_| ())
}

#[cfg(test)]
mod tests {
    use nils_test_support::{EnvGuard, GlobalStateLock};
    use pretty_assertions::assert_eq;

    use super::{resolve_target, selector_label, wait_for_active_confirmation, ActivationTarget};
    use crate::backend::process::{ProcessFailure, ProcessOutput, ProcessRequest, ProcessRunner};
    use crate::cli::WindowActivateArgs;

    #[derive(Debug)]
    struct PanicRunner;

    impl ProcessRunner for PanicRunner {
        fn run(&self, _request: &ProcessRequest) -> Result<ProcessOutput, ProcessFailure> {
            panic!("runner should not be called in this test")
        }
    }

    #[test]
    fn selector_label_prefers_window_id() {
        let args = WindowActivateArgs {
            window_id: Some(42),
            active_window: false,
            app: Some("Terminal".to_string()),
            window_name: None,
            bundle_id: None,
            wait_ms: None,
        };
        assert_eq!(selector_label(&args), "--window-id 42");
    }

    #[test]
    fn selector_label_formats_other_selectors() {
        let active = WindowActivateArgs {
            window_id: None,
            active_window: true,
            app: None,
            window_name: None,
            bundle_id: None,
            wait_ms: None,
        };
        assert_eq!(selector_label(&active), "--active-window");

        let app_window = WindowActivateArgs {
            window_id: None,
            active_window: false,
            app: Some("Terminal".to_string()),
            window_name: Some("Inbox".to_string()),
            bundle_id: None,
            wait_ms: None,
        };
        assert_eq!(
            selector_label(&app_window),
            "--app Terminal --window-name Inbox"
        );

        let bundle = WindowActivateArgs {
            window_id: None,
            active_window: false,
            app: None,
            window_name: None,
            bundle_id: Some("com.apple.Terminal".to_string()),
            wait_ms: None,
        };
        assert_eq!(selector_label(&bundle), "--bundle-id com.apple.Terminal");
    }

    #[test]
    fn resolve_target_accepts_bundle_id_and_app_without_lookup() {
        let bundle_args = WindowActivateArgs {
            window_id: None,
            active_window: false,
            app: None,
            window_name: None,
            bundle_id: Some("com.apple.Terminal".to_string()),
            wait_ms: None,
        };
        let (target, selected_app, selected_window_id) =
            resolve_target(&bundle_args).expect("bundle selector should resolve");
        assert_eq!(
            target,
            ActivationTarget::BundleId("com.apple.Terminal".to_string())
        );
        assert_eq!(selected_app, "com.apple.Terminal");
        assert_eq!(selected_window_id, None);

        let app_args = WindowActivateArgs {
            window_id: None,
            active_window: false,
            app: Some("Terminal".to_string()),
            window_name: None,
            bundle_id: None,
            wait_ms: None,
        };
        let (target, selected_app, selected_window_id) =
            resolve_target(&app_args).expect("app selector should resolve");
        assert_eq!(target, ActivationTarget::App("Terminal".to_string()));
        assert_eq!(selected_app, "Terminal");
        assert_eq!(selected_window_id, None);
    }

    #[test]
    fn resolve_target_uses_target_lookup_for_active_window() {
        let lock = GlobalStateLock::new();
        let _mode = EnvGuard::set(&lock, "CODEX_MACOS_AGENT_TEST_MODE", "1");

        let args = WindowActivateArgs {
            window_id: None,
            active_window: true,
            app: None,
            window_name: None,
            bundle_id: None,
            wait_ms: None,
        };
        let (target, selected_app, selected_window_id) =
            resolve_target(&args).expect("active-window selector should resolve");
        assert_eq!(target, ActivationTarget::App("Terminal".to_string()));
        assert_eq!(selected_app, "Terminal");
        assert_eq!(selected_window_id, Some(100));
    }

    #[test]
    fn wait_for_active_confirmation_short_circuits_on_zero_or_test_mode() {
        wait_for_active_confirmation(
            &PanicRunner,
            &ActivationTarget::App("Terminal".to_string()),
            0,
        )
        .expect("zero wait should be a no-op");

        let lock = GlobalStateLock::new();
        let _mode = EnvGuard::set(&lock, "CODEX_MACOS_AGENT_TEST_MODE", "1");
        wait_for_active_confirmation(
            &PanicRunner,
            &ActivationTarget::App("Terminal".to_string()),
            25,
        )
        .expect("test mode should skip runner polling");
    }
}
