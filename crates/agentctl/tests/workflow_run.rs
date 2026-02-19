use agentctl::workflow::run::{
    RunArgs, RunOutputFormat, StepStatus, execute_workflow_document, load_workflow_file, run,
};
use agentctl::workflow::schema::{
    AutomationStep, AutomationTool, ProviderStep, RetryPolicy, WORKFLOW_SCHEMA_VERSION,
    WorkflowDocument, WorkflowOnError, WorkflowStep,
};
use nils_test_support::http::{HttpResponse, LoopbackServer};
use nils_test_support::{EnvGuard, GlobalStateLock, StubBinDir, prepend_path};
use pretty_assertions::assert_eq;
use serde_json::json;
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
    let agent_home = stub.path().join("agent-home");
    std::fs::create_dir_all(&agent_home).expect("create agent home");
    let agent_home_str = agent_home.to_string_lossy().to_string();
    let agent_home_guard = EnvGuard::set(&lock, "AGENT_HOME", agent_home_str.as_str());
    let env_guards = (path_guard, agent_home_guard);

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
    let agent_home = stub.path().join("agent-home");
    std::fs::create_dir_all(&agent_home).expect("create agent home");
    let agent_home_str = agent_home.to_string_lossy().to_string();
    let agent_home_guard = EnvGuard::set(&lock, "AGENT_HOME", agent_home_str.as_str());
    let auth_file = stub.path().join("auth.json");
    std::fs::write(&auth_file, "{}").expect("write auth file");
    let auth_file_str = auth_file.to_string_lossy().to_string();
    let auth_file_guard = EnvGuard::set(&lock, "CODEX_AUTH_FILE", auth_file_str.as_str());
    let dangerous_guard = EnvGuard::set(&lock, "CODEX_ALLOW_DANGEROUS_ENABLED", "true");
    let env_guards = (
        path_guard,
        agent_home_guard,
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

#[test]
fn workflow_run_load_workflow_file_reports_read_parse_and_schema_errors() {
    let dir = StubBinDir::new();
    let missing = dir.path().join("missing.json");
    let missing_err = load_workflow_file(missing.as_path()).expect_err("missing should error");
    assert!(
        missing_err
            .to_string()
            .contains("failed to read workflow file"),
        "error={missing_err}"
    );

    let parse_path = dir.path().join("invalid.json");
    std::fs::write(&parse_path, "{ invalid json").expect("write invalid json");
    let parse_err = load_workflow_file(parse_path.as_path()).expect_err("parse should error");
    assert!(
        parse_err
            .to_string()
            .contains("failed to parse workflow file"),
        "error={parse_err}"
    );

    let schema_path = dir.path().join("schema.json");
    std::fs::write(
        &schema_path,
        r#"{"schema_version":"agentctl.workflow.v1","name":"bad","on_error":"fail-fast","steps":[]}"#,
    )
    .expect("write schema json");
    let schema_err = load_workflow_file(schema_path.as_path()).expect_err("schema should error");
    assert!(
        schema_err
            .to_string()
            .contains("workflow must define at least one step"),
        "error={schema_err}"
    );
}

#[test]
fn workflow_run_provider_step_surfaces_missing_codex_binary_error() {
    let lock = GlobalStateLock::new();
    let _path = EnvGuard::set(&lock, "PATH", "/usr/bin:/bin");
    let _dangerous = EnvGuard::set(&lock, "CODEX_ALLOW_DANGEROUS_ENABLED", "true");
    let _auth_file = EnvGuard::remove(&lock, "CODEX_AUTH_FILE");

    let workflow = WorkflowDocument {
        schema_version: WORKFLOW_SCHEMA_VERSION.to_string(),
        name: Some("provider-missing-binary".to_string()),
        on_error: WorkflowOnError::FailFast,
        steps: vec![WorkflowStep::Provider(ProviderStep {
            id: "provider-step".to_string(),
            provider: Some("codex".to_string()),
            task: "ping".to_string(),
            input: Some("missing binary path".to_string()),
            timeout_ms: Some(5000),
            retry: RetryPolicy::default(),
        })],
    };

    let report = execute_workflow_document(&workflow);
    assert_eq!(report.summary.failed_steps, 1);
    assert_eq!(report.ledger.len(), 1);
    let step = &report.ledger[0];
    assert_eq!(step.status, StepStatus::Failed);
    assert!(
        step.stderr
            .contains("codex binary is not available on PATH")
    );
}

#[test]
fn workflow_run_supports_claude_provider_step_success() {
    let lock = GlobalStateLock::new();
    let server = LoopbackServer::new().expect("loopback");
    server.add_route(
        "POST",
        "/v1/messages",
        HttpResponse::new(
            200,
            json!({
                "id": "msg_123",
                "type": "message",
                "role": "assistant",
                "content": [
                    { "type": "text", "text": "workflow claude success" }
                ],
                "model": "claude-sonnet-4-5-20250929"
            })
            .to_string(),
        ),
    );
    let _api_key = EnvGuard::set(&lock, "ANTHROPIC_API_KEY", "test-key");
    let _base_url = EnvGuard::set(&lock, "ANTHROPIC_BASE_URL", server.url().as_str());
    let _retry_max = EnvGuard::set(&lock, "CLAUDE_RETRY_MAX", "0");

    let workflow = load_workflow_file(fixture_path("claude-minimal.json").as_path())
        .expect("claude workflow fixture");
    let report = execute_workflow_document(&workflow);

    assert_eq!(report.summary.failed_steps, 0);
    assert_eq!(report.summary.succeeded_steps, 1);
    assert_eq!(report.ledger.len(), 1);
    let step = &report.ledger[0];
    assert_eq!(step.step_id, "claude-provider-step");
    assert_eq!(step.status, StepStatus::Succeeded);
    assert_eq!(step.provider.as_deref(), Some("claude"));
    assert!(step.stdout.contains("workflow claude success"));
}

#[test]
fn workflow_run_claude_provider_step_reports_missing_api_key() {
    let lock = GlobalStateLock::new();
    let _api_key = EnvGuard::remove(&lock, "ANTHROPIC_API_KEY");
    let _base_url = EnvGuard::remove(&lock, "ANTHROPIC_BASE_URL");

    let workflow = load_workflow_file(fixture_path("claude-minimal.json").as_path())
        .expect("claude workflow fixture");
    let report = execute_workflow_document(&workflow);

    assert_eq!(report.summary.failed_steps, 1);
    assert_eq!(report.ledger.len(), 1);
    let step = &report.ledger[0];
    assert_eq!(step.status, StepStatus::Failed);
    assert!(step.stderr.contains("ANTHROPIC_API_KEY"));
}

#[test]
fn workflow_run_automation_step_times_out_with_exit_code_124() {
    let lock = GlobalStateLock::new();
    let stub = StubBinDir::new();
    // Avoid external binaries (like `sleep`) because this test intentionally
    // constrains PATH to stubs only.
    stub.write_exe("fzf-cli", "#!/bin/sh\nwhile :; do\n  :\ndone\n");
    let path_only_stub = stub.path().to_string_lossy().to_string();
    let _path = EnvGuard::set(&lock, "PATH", &path_only_stub);
    let agent_home = stub.path().join("agent-home");
    std::fs::create_dir_all(&agent_home).expect("create agent home");
    let agent_home_str = agent_home.to_string_lossy().to_string();
    let _agent_home = EnvGuard::set(&lock, "AGENT_HOME", &agent_home_str);

    let workflow = WorkflowDocument {
        schema_version: WORKFLOW_SCHEMA_VERSION.to_string(),
        name: Some("timeout-check".to_string()),
        on_error: WorkflowOnError::FailFast,
        steps: vec![WorkflowStep::Automation(AutomationStep {
            id: "timeout-step".to_string(),
            tool: AutomationTool::FzfCli,
            args: vec!["help".to_string()],
            timeout_ms: Some(10),
            retry: RetryPolicy::default(),
        })],
    };

    let report = execute_workflow_document(&workflow);
    assert_eq!(report.summary.failed_steps, 1);
    assert_eq!(report.ledger.len(), 1);
    let step = &report.ledger[0];
    assert_eq!(step.status, StepStatus::Failed);
    assert_eq!(step.exit_code, 124);
    assert!(step.stderr.contains("step timed out"));
}

#[test]
fn workflow_run_appends_artifact_persist_errors_without_masking_success() {
    let lock = GlobalStateLock::new();
    let stub = StubBinDir::new();
    stub.write_exe("fzf-cli", "#!/bin/sh\necho ok\n");
    let path_only_stub = stub.path().to_string_lossy().to_string();
    let _path = EnvGuard::set(&lock, "PATH", &path_only_stub);
    let bad_agent_home = stub.path().join("agent-home-file");
    std::fs::write(&bad_agent_home, "not a dir").expect("write agent home file");
    let bad_agent_home_str = bad_agent_home.to_string_lossy().to_string();
    let _agent_home = EnvGuard::set(&lock, "AGENT_HOME", &bad_agent_home_str);

    let workflow = WorkflowDocument {
        schema_version: WORKFLOW_SCHEMA_VERSION.to_string(),
        name: Some("artifact-failure".to_string()),
        on_error: WorkflowOnError::FailFast,
        steps: vec![WorkflowStep::Automation(AutomationStep {
            id: "artifact-step".to_string(),
            tool: AutomationTool::FzfCli,
            args: vec!["help".to_string()],
            timeout_ms: Some(5000),
            retry: RetryPolicy::default(),
        })],
    };

    let report = execute_workflow_document(&workflow);
    assert_eq!(report.summary.succeeded_steps, 1);
    let step = &report.ledger[0];
    assert_eq!(step.status, StepStatus::Succeeded);
    assert!(step.artifact_paths.is_empty());
    assert!(step.stderr.contains("failed to persist step artifacts"));
}

#[test]
fn workflow_run_entrypoint_returns_expected_exit_codes_for_failure_and_success() {
    let lock = GlobalStateLock::new();
    let stub = StubBinDir::new();
    stub.write_exe("fzf-cli", "#!/bin/sh\necho run-fzf\nexit 7\n");
    stub.write_exe(
        "image-processing",
        "#!/bin/sh\necho image-processing-help\nexit 0\n",
    );
    let path_only_stub = stub.path().to_string_lossy().to_string();
    let _path = EnvGuard::set(&lock, "PATH", &path_only_stub);
    let agent_home = stub.path().join("agent-home");
    std::fs::create_dir_all(&agent_home).expect("create agent home");
    let agent_home_str = agent_home.to_string_lossy().to_string();
    let _agent_home = EnvGuard::set(&lock, "AGENT_HOME", &agent_home_str);

    let missing_exit = run(RunArgs {
        file: stub.path().join("missing.json"),
        format: RunOutputFormat::Json,
    });
    assert_eq!(missing_exit, 64);

    let failing_path = stub.path().join("failing-workflow.json");
    std::fs::write(
        &failing_path,
        r#"{
  "schema_version":"agentctl.workflow.v1",
  "name":"failing-run",
  "on_error":"fail-fast",
  "steps":[
    {"type":"automation","id":"fzf-fail","tool":"fzf-cli","args":["help"],"timeout_ms":5000}
  ]
}"#,
    )
    .expect("write failing workflow");
    let failing_exit = run(RunArgs {
        file: failing_path,
        format: RunOutputFormat::Text,
    });
    assert_eq!(failing_exit, 1);

    let success_path = stub.path().join("success-workflow.json");
    std::fs::write(
        &success_path,
        r#"{
  "schema_version":"agentctl.workflow.v1",
  "name":"success-run",
  "on_error":"fail-fast",
  "steps":[
    {"type":"automation","id":"image-ok","tool":"image-processing","args":["info","--help"],"timeout_ms":5000}
  ]
}"#,
    )
    .expect("write success workflow");
    let success_exit = run(RunArgs {
        file: success_path,
        format: RunOutputFormat::Text,
    });
    assert_eq!(success_exit, 0);
}
