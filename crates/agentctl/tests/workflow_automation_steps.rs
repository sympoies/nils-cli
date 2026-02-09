use agentctl::workflow::run::{execute_workflow_document, load_workflow_file, StepStatus};
use agentctl::workflow::schema::{
    AutomationStep, AutomationTool, RetryPolicy, WorkflowDocument, WorkflowOnError, WorkflowStep,
    WORKFLOW_SCHEMA_VERSION,
};
use nils_test_support::{prepend_path, EnvGuard, GlobalStateLock, StubBinDir};
use pretty_assertions::assert_eq;
use std::path::{Path, PathBuf};

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("workflow")
        .join(name)
}

fn install_automation_stubs(stub: &StubBinDir) {
    stub.write_exe(
        "macos-agent",
        "#!/bin/sh\necho macos-agent-stdout\necho macos-agent-stderr 1>&2\nexit 0\n",
    );
    stub.write_exe(
        "screen-record",
        "#!/bin/sh\necho screen-record-stdout\necho screen-record-stderr 1>&2\nexit 0\n",
    );
    stub.write_exe(
        "image-processing",
        "#!/bin/sh\necho image-processing-stdout\necho image-processing-stderr 1>&2\nexit 0\n",
    );
    stub.write_exe(
        "fzf-cli",
        "#!/bin/sh\necho fzf-cli-stdout\necho fzf-cli-stderr 1>&2\nexit 0\n",
    );
}

fn configure_test_env(
    lock: &GlobalStateLock,
    stub: &StubBinDir,
) -> (EnvGuard, EnvGuard, EnvGuard, EnvGuard) {
    let path_guard = prepend_path(lock, stub.path());
    let codex_home = stub.path().join("codex-home");
    std::fs::create_dir_all(&codex_home).expect("create codex home");
    let codex_home_str = codex_home.to_string_lossy().to_string();
    let codex_home_guard = EnvGuard::set(lock, "CODEX_HOME", codex_home_str.as_str());
    let macos_guard = EnvGuard::set(lock, "CODEX_MACOS_AGENT_TEST_MODE", "1");
    let screen_guard = EnvGuard::set(lock, "CODEX_SCREEN_RECORD_TEST_MODE", "1");
    (path_guard, codex_home_guard, macos_guard, screen_guard)
}

fn find_step<'a>(
    report: &'a agentctl::workflow::run::WorkflowRunReport,
    step_id: &str,
) -> &'a agentctl::workflow::run::StepLedgerEntry {
    report
        .ledger
        .iter()
        .find(|entry| entry.step_id == step_id)
        .expect("step should exist in ledger")
}

#[test]
fn workflow_automation_steps_emit_provenance_and_artifact_pointers_for_all_tools() {
    let lock = GlobalStateLock::new();
    let stub = StubBinDir::new();
    install_automation_stubs(&stub);
    let env_guards = configure_test_env(&lock, &stub);

    let workflow =
        load_workflow_file(fixture_path("automation-mixed.json").as_path()).expect("workflow");
    let report = execute_workflow_document(&workflow);

    assert_eq!(report.summary.total_steps, 4);
    assert_eq!(report.summary.executed_steps, 4);
    assert_eq!(report.summary.failed_steps, 0);
    assert!(report.summary.success);

    let expected = vec![
        (
            "macos-agent-step",
            "macos-agent",
            vec![
                "--format".to_string(),
                "json".to_string(),
                "preflight".to_string(),
            ],
            "macos-agent-stdout",
            "macos-agent-stderr",
        ),
        (
            "screen-record-step",
            "screen-record",
            vec!["--preflight".to_string()],
            "screen-record-stdout",
            "screen-record-stderr",
        ),
        (
            "image-processing-step",
            "image-processing",
            vec!["info".to_string(), "--help".to_string()],
            "image-processing-stdout",
            "image-processing-stderr",
        ),
        (
            "fzf-cli-step",
            "fzf-cli",
            vec!["help".to_string()],
            "fzf-cli-stdout",
            "fzf-cli-stderr",
        ),
    ];

    for (step_id, tool, args, stdout_marker, stderr_marker) in expected {
        let step = find_step(&report, step_id);
        assert_eq!(step.status, StepStatus::Succeeded);
        assert_eq!(step.step_type, "automation");
        assert_eq!(step.automation_tool.as_deref(), Some(tool));

        let command = step.command.as_ref().expect("command provenance");
        assert_eq!(command.tool, tool);
        assert_eq!(command.command, tool);
        assert_eq!(command.args, args);

        assert_eq!(step.artifact_paths.len(), 2);
        let mut stdout_log = None;
        let mut stderr_log = None;
        for path in &step.artifact_paths {
            let artifact_path = Path::new(path);
            assert!(artifact_path.is_file(), "artifact should exist: {path}");
            if path.ends_with("stdout.log") {
                stdout_log = Some(path.clone());
            } else if path.ends_with("stderr.log") {
                stderr_log = Some(path.clone());
            }
        }

        let stdout_path = stdout_log.expect("stdout artifact path");
        let stderr_path = stderr_log.expect("stderr artifact path");
        let stdout = std::fs::read_to_string(&stdout_path).expect("read stdout artifact");
        let stderr = std::fs::read_to_string(&stderr_path).expect("read stderr artifact");
        assert!(
            stdout.contains(stdout_marker),
            "stdout artifact missing marker for {step_id}: {stdout}"
        );
        assert!(
            stderr.contains(stderr_marker),
            "stderr artifact missing marker for {step_id}: {stderr}"
        );
    }
    std::hint::black_box(&env_guards);
}

#[test]
fn workflow_automation_steps_apply_typed_default_arguments() {
    let lock = GlobalStateLock::new();
    let stub = StubBinDir::new();
    install_automation_stubs(&stub);
    let env_guards = configure_test_env(&lock, &stub);

    let workflow = WorkflowDocument {
        schema_version: WORKFLOW_SCHEMA_VERSION.to_string(),
        name: Some("automation-default-args".to_string()),
        on_error: WorkflowOnError::ContinueOnError,
        steps: vec![
            WorkflowStep::Automation(AutomationStep {
                id: "macos-default".to_string(),
                tool: AutomationTool::MacosAgent,
                args: Vec::new(),
                timeout_ms: Some(5000),
                retry: RetryPolicy::default(),
            }),
            WorkflowStep::Automation(AutomationStep {
                id: "screen-default".to_string(),
                tool: AutomationTool::ScreenRecord,
                args: Vec::new(),
                timeout_ms: Some(5000),
                retry: RetryPolicy::default(),
            }),
            WorkflowStep::Automation(AutomationStep {
                id: "image-default".to_string(),
                tool: AutomationTool::ImageProcessing,
                args: Vec::new(),
                timeout_ms: Some(5000),
                retry: RetryPolicy::default(),
            }),
            WorkflowStep::Automation(AutomationStep {
                id: "fzf-default".to_string(),
                tool: AutomationTool::FzfCli,
                args: Vec::new(),
                timeout_ms: Some(5000),
                retry: RetryPolicy::default(),
            }),
        ],
    };

    let report = execute_workflow_document(&workflow);
    assert!(report.summary.success);

    let expected_args = vec![
        (
            "macos-default",
            vec![
                "--format".to_string(),
                "json".to_string(),
                "preflight".to_string(),
            ],
        ),
        ("screen-default", vec!["--preflight".to_string()]),
        (
            "image-default",
            vec!["info".to_string(), "--help".to_string()],
        ),
        ("fzf-default", vec!["help".to_string()]),
    ];
    for (step_id, args) in expected_args {
        let step = find_step(&report, step_id);
        let command = step.command.as_ref().expect("command provenance");
        assert_eq!(command.args, args);
    }
    std::hint::black_box(&env_guards);
}
