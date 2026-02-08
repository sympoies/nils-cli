use agentctl::workflow::run::{execute_workflow_document, load_workflow_file, StepStatus};
use agentctl::workflow::schema::{
    AutomationStep, AutomationTool, ProviderStep, RetryPolicy, WorkflowDocument, WorkflowOnError,
    WorkflowStep, WORKFLOW_SCHEMA_VERSION,
};
use nils_test_support::{prepend_path, EnvGuard, GlobalStateLock, StubBinDir};
use pretty_assertions::assert_eq;
use std::path::PathBuf;

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("workflow")
        .join(name)
}

fn build_mode_workflow(on_error: WorkflowOnError) -> WorkflowDocument {
    WorkflowDocument {
        schema_version: WORKFLOW_SCHEMA_VERSION.to_string(),
        name: Some("mode-check".to_string()),
        on_error,
        steps: vec![
            WorkflowStep::Automation(AutomationStep {
                id: "fail-step".to_string(),
                tool: AutomationTool::FzfCli,
                args: vec!["--fail".to_string()],
                timeout_ms: Some(5000),
                retry: RetryPolicy::default(),
            }),
            WorkflowStep::Automation(AutomationStep {
                id: "success-step".to_string(),
                tool: AutomationTool::ImageProcessing,
                args: vec!["info".to_string()],
                timeout_ms: Some(5000),
                retry: RetryPolicy::default(),
            }),
        ],
    }
}

#[test]
fn workflow_run_schema_supports_provider_and_automation_steps_in_one_file() {
    let workflow = load_workflow_file(fixture_path("minimal.json").as_path()).expect("workflow");

    assert_eq!(workflow.steps.len(), 2);
    assert!(matches!(workflow.steps[0], WorkflowStep::Provider(_)));
    assert!(matches!(workflow.steps[1], WorkflowStep::Automation(_)));
}

#[test]
fn workflow_run_supports_fail_fast_and_continue_on_error_modes() {
    let lock = GlobalStateLock::new();
    let stub = StubBinDir::new();
    stub.write_exe(
        "fzf-cli",
        "#!/bin/sh\necho fail-out\necho fail-err 1>&2\nexit 7\n",
    );
    stub.write_exe(
        "image-processing",
        "#!/bin/sh\necho image-out\necho image-err 1>&2\nexit 0\n",
    );
    let path_guard = prepend_path(&lock, stub.path());
    let codex_home = stub.path().join("codex-home");
    std::fs::create_dir_all(&codex_home).expect("create codex home");
    let codex_home_str = codex_home.to_string_lossy().to_string();
    let codex_home_guard = EnvGuard::set(&lock, "CODEX_HOME", codex_home_str.as_str());
    let env_guards = (path_guard, codex_home_guard);

    let fail_fast = execute_workflow_document(&build_mode_workflow(WorkflowOnError::FailFast));
    assert_eq!(fail_fast.summary.executed_steps, 1);
    assert_eq!(fail_fast.summary.failed_steps, 1);
    assert_eq!(fail_fast.summary.skipped_steps, 1);
    assert_eq!(fail_fast.ledger.len(), 1);
    assert_eq!(fail_fast.ledger[0].status, StepStatus::Failed);

    let continue_mode =
        execute_workflow_document(&build_mode_workflow(WorkflowOnError::ContinueOnError));
    assert_eq!(continue_mode.summary.executed_steps, 2);
    assert_eq!(continue_mode.summary.succeeded_steps, 1);
    assert_eq!(continue_mode.summary.failed_steps, 1);
    assert_eq!(continue_mode.summary.skipped_steps, 0);
    assert_eq!(continue_mode.ledger.len(), 2);
    assert_eq!(continue_mode.ledger[0].status, StepStatus::Failed);
    assert_eq!(continue_mode.ledger[1].status, StepStatus::Succeeded);
    std::hint::black_box(&env_guards);
}

#[test]
fn workflow_run_step_ledger_includes_stdout_stderr_exit_code_and_elapsed_ms() {
    let lock = GlobalStateLock::new();
    let stub = StubBinDir::new();
    stub.write_exe("codex", "#!/bin/sh\nexit 0\n");
    stub.write_exe(
        "fzf-cli",
        "#!/bin/sh\necho workflow-automation-stdout\necho workflow-automation-stderr 1>&2\nexit 0\n",
    );
    let path_guard = prepend_path(&lock, stub.path());
    let codex_home = stub.path().join("codex-home");
    std::fs::create_dir_all(&codex_home).expect("create codex home");
    let codex_home_str = codex_home.to_string_lossy().to_string();
    let codex_home_guard = EnvGuard::set(&lock, "CODEX_HOME", codex_home_str.as_str());
    let auth_file = stub.path().join("auth.json");
    std::fs::write(&auth_file, "{}").expect("write auth file");
    let auth_file_str = auth_file.to_string_lossy().to_string();
    let auth_file_guard = EnvGuard::set(&lock, "CODEX_AUTH_FILE", auth_file_str.as_str());
    let dangerous_guard = EnvGuard::set(&lock, "CODEX_ALLOW_DANGEROUS_ENABLED", "true");
    let env_guards = (
        path_guard,
        codex_home_guard,
        auth_file_guard,
        dangerous_guard,
    );

    let workflow = WorkflowDocument {
        schema_version: WORKFLOW_SCHEMA_VERSION.to_string(),
        name: Some("ledger-check".to_string()),
        on_error: WorkflowOnError::FailFast,
        steps: vec![
            WorkflowStep::Provider(ProviderStep {
                id: "provider-step".to_string(),
                provider: Some("codex".to_string()),
                task: "ping".to_string(),
                input: Some("workflow provider execution smoke".to_string()),
                timeout_ms: Some(5000),
                retry: RetryPolicy::default(),
            }),
            WorkflowStep::Automation(AutomationStep {
                id: "automation-step".to_string(),
                tool: AutomationTool::FzfCli,
                args: vec!["help".to_string()],
                timeout_ms: Some(5000),
                retry: RetryPolicy::default(),
            }),
        ],
    };

    let report = execute_workflow_document(&workflow);
    assert_eq!(report.summary.total_steps, 2);
    assert_eq!(report.summary.executed_steps, 2);
    assert_eq!(report.summary.failed_steps, 0);

    let provider_step = report
        .ledger
        .iter()
        .find(|entry| entry.step_id == "provider-step")
        .expect("provider ledger");
    assert_eq!(provider_step.status, StepStatus::Succeeded);
    assert_eq!(provider_step.exit_code, 0);
    assert!(provider_step.stderr.is_empty());
    assert!(provider_step.elapsed_ms <= report.summary.elapsed_ms);

    let automation_step = report
        .ledger
        .iter()
        .find(|entry| entry.step_id == "automation-step")
        .expect("automation ledger");
    assert_eq!(automation_step.status, StepStatus::Succeeded);
    assert_eq!(automation_step.exit_code, 0);
    assert!(
        automation_step
            .stdout
            .contains("workflow-automation-stdout"),
        "stdout={}",
        automation_step.stdout
    );
    assert!(
        automation_step
            .stderr
            .contains("workflow-automation-stderr"),
        "stderr={}",
        automation_step.stderr
    );
    let command = automation_step
        .command
        .as_ref()
        .expect("command provenance");
    assert_eq!(command.tool, "fzf-cli");
    assert_eq!(command.command, "fzf-cli");
    assert_eq!(command.args, vec!["help".to_string()]);
    assert_eq!(automation_step.artifact_paths.len(), 2);
    for path in &automation_step.artifact_paths {
        assert!(
            std::path::Path::new(path).is_file(),
            "artifact path should exist: {path}"
        );
    }
    assert!(automation_step.elapsed_ms <= report.summary.elapsed_ms);
    std::hint::black_box(&env_guards);
}
