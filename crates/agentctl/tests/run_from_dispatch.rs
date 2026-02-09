use agentctl::debug::schema::BUNDLE_MANIFEST_FILE_NAME;
use nils_test_support::git::{self, InitRepoOptions};
use nils_test_support::{prepend_path, CwdGuard, EnvGuard, GlobalStateLock, StubBinDir};

#[test]
fn run_from_group_help_and_parse_errors_return_expected_exit_codes() {
    assert_eq!(agentctl::run_from(["agentctl"]), 0);
    assert_eq!(agentctl::run_from(["agentctl", "--help"]), 0);
    assert_eq!(agentctl::run_from(["agentctl", "provider"]), 0);
    assert_eq!(agentctl::run_from(["agentctl", "diag"]), 0);
    assert_eq!(agentctl::run_from(["agentctl", "debug"]), 0);
    assert_eq!(agentctl::run_from(["agentctl", "workflow"]), 0);
    assert_eq!(agentctl::run_from(["agentctl", "automation"]), 0);
    assert_eq!(agentctl::run_from(["agentctl", "not-a-real-command"]), 64);
}

#[test]
fn run_from_provider_and_diag_subcommands_dispatch_successfully() {
    let lock = GlobalStateLock::new();
    let codex_home = StubBinDir::new();
    let codex_home_dir = codex_home.path().join("codex-home");
    std::fs::create_dir_all(&codex_home_dir).expect("create codex home");
    let codex_home_value = codex_home_dir.to_string_lossy().to_string();
    let _codex_home = EnvGuard::set(&lock, "CODEX_HOME", codex_home_value.as_str());

    assert_eq!(
        agentctl::run_from(["agentctl", "provider", "list", "--format", "json"]),
        0
    );
    assert_eq!(
        agentctl::run_from([
            "agentctl",
            "diag",
            "capabilities",
            "--format",
            "json",
            "--probe-mode",
            "test",
        ]),
        0
    );
}

#[test]
fn run_from_debug_and_workflow_subcommands_dispatch_successfully() {
    let lock = GlobalStateLock::new();
    let stub = StubBinDir::new();
    stub.write_exe(
        "macos-agent",
        "#!/bin/sh\necho '{\"ok\":true,\"command\":\"preflight\"}'\n",
    );
    stub.write_exe("screen-record", "#!/bin/sh\necho 'preflight ok'\n");
    stub.write_exe(
        "image-processing",
        "#!/bin/sh\necho 'image-processing help'\n",
    );
    stub.write_exe("fzf-cli", "#!/bin/sh\necho 'workflow-automation ok'\n");

    let repo = git::init_repo_with(
        InitRepoOptions::new()
            .with_branch("main")
            .with_initial_commit(),
    );
    let _path = prepend_path(&lock, stub.path());
    let _cwd = CwdGuard::set(&lock, repo.path()).expect("set cwd");
    let codex_home_dir = repo.path().join("codex-home");
    std::fs::create_dir_all(&codex_home_dir).expect("create codex home");
    let codex_home_value = codex_home_dir.to_string_lossy().to_string();
    let _codex_home = EnvGuard::set(&lock, "CODEX_HOME", codex_home_value.as_str());

    let bundle_output_dir = repo.path().join("bundle-output");
    let bundle_output_arg = bundle_output_dir.to_string_lossy().to_string();
    assert_eq!(
        agentctl::run_from([
            "agentctl",
            "debug",
            "bundle",
            "--output-dir",
            bundle_output_arg.as_str(),
            "--format",
            "json",
        ]),
        0
    );
    assert!(bundle_output_dir.join(BUNDLE_MANIFEST_FILE_NAME).is_file());

    let workflow_file = repo.path().join("workflow.json");
    std::fs::write(
        &workflow_file,
        r#"{
  "schema_version":"agentctl.workflow.v1",
  "name":"dispatch-workflow",
  "on_error":"fail-fast",
  "steps":[
    {"type":"automation","id":"fzf-ok","tool":"fzf-cli","args":["help"],"timeout_ms":5000}
  ]
}"#,
    )
    .expect("write workflow fixture");
    let workflow_file_arg = workflow_file.to_string_lossy().to_string();
    assert_eq!(
        agentctl::run_from([
            "agentctl",
            "workflow",
            "run",
            "--file",
            workflow_file_arg.as_str(),
            "--format",
            "json",
        ]),
        0
    );
}
