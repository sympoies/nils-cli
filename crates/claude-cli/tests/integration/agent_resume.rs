use crate::support::*;
use nils_test_support::cmd::CmdOptions;
use pretty_assertions::assert_eq;
use std::path::Path;

fn write_claude_session(config_dir: &Path, project: &str, id: &str, cwd: &Path) {
    let dir = config_dir.join("projects").join(project);
    std::fs::create_dir_all(&dir).expect("project dir");
    let line = format!(
        "{{\"sessionId\":\"{id}\",\"cwd\":\"{}\"}}\n",
        cwd.to_string_lossy()
    );
    std::fs::write(dir.join(format!("{id}.jsonl")), line).expect("transcript");
}

#[test]
fn agent_resume_control_char_id_is_usage_error() {
    let tmp = tempfile::TempDir::new().expect("tmp");
    let options = CmdOptions::default().with_cwd(tmp.path());
    let output = run(&["agent", "resume", "bad\tid"], &options);
    assert_exit(&output, 64);
}

#[test]
fn agent_resume_unknown_id_returns_data_error() {
    let tmp = tempfile::TempDir::new().expect("tmp");
    let config = tmp.path().join("claude-config");
    std::fs::create_dir_all(config.join("projects")).expect("projects");
    let elsewhere = tmp.path().join("elsewhere");
    std::fs::create_dir_all(&elsewhere).expect("elsewhere");

    let options = CmdOptions::default()
        .with_cwd(&elsewhere)
        .with_env("CLAUDE_CONFIG_DIR", &path_str(&config));
    let output = run(&["agent", "resume", "absent-id"], &options);

    assert_exit(&output, 65);
    assert!(stderr(&output).contains("no Claude session history"));
}

#[test]
fn agent_resume_launches_claude_in_recorded_cwd_from_unrelated_dir() {
    let tmp = tempfile::TempDir::new().expect("tmp");
    // A recorded cwd with a space exercises exact working-directory handling.
    let repo = tmp.path().join("recorded repo");
    std::fs::create_dir_all(&repo).expect("repo");
    let config = tmp.path().join("claude-config");
    write_claude_session(&config, "-recorded-repo", "cl-x", &repo);
    let elsewhere = tmp.path().join("elsewhere");
    std::fs::create_dir_all(&elsewhere).expect("elsewhere");

    let out_log = tmp.path().join("claude-out.txt");
    let stub_dir = tmp.path().join("stub");
    std::fs::create_dir_all(&stub_dir).expect("stub dir");
    // The stub drops a marker in its own working directory (proving it launched
    // in the recorded cwd) and records its argv to an absolute log path.
    nils_test_support::write_exe(
        &stub_dir,
        "claude",
        &format!(
            "#!/bin/sh\n: > launched-here\nprintf 'ARG:%s\\n' \"$@\" > '{}'\nexit 5\n",
            out_log.display()
        ),
    );

    let options = CmdOptions::default()
        .with_cwd(&elsewhere)
        .with_env("CLAUDE_CONFIG_DIR", &path_str(&config))
        .with_env(
            "CLAUDE_CLI_BIN",
            &path_str(&tmp.path().join("ambient-claude-must-not-run")),
        )
        .with_fake_claude(&stub_dir);
    let output = run(&["agent", "resume", "cl-x"], &options);

    assert_exit(&output, 5);
    assert!(
        repo.join("launched-here").exists(),
        "expected claude to be launched in the recorded cwd"
    );
    let logged = std::fs::read_to_string(&out_log).expect("stub log");
    assert_eq!(
        logged.lines().collect::<Vec<_>>(),
        vec!["ARG:--resume", "ARG:cl-x"],
        "claude must be launched with exactly `--resume <id>`"
    );
}

#[test]
fn agent_resume_cd_override_to_missing_directory_is_runtime_error() {
    let tmp = tempfile::TempDir::new().expect("tmp");
    let missing = tmp.path().join("does-not-exist");
    let options = CmdOptions::default().with_cwd(tmp.path());
    let output = run(
        &["agent", "resume", "any-id", "--cd", &path_str(&missing)],
        &options,
    );

    assert_exit(&output, 1);
    assert!(stderr(&output).contains("not an existing directory"));
}

#[test]
fn agent_resume_truncated_scan_returns_runtime_error() {
    let tmp = tempfile::TempDir::new().expect("tmp");
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(&repo).expect("repo");
    let config = tmp.path().join("claude-config");
    // Two matching projects plus a one-entry budget forces the bounded scan to
    // truncate before it can decide, without ever launching claude.
    write_claude_session(&config, "-proj-a", "trunc-id", &repo);
    write_claude_session(&config, "-proj-b", "trunc-id", &repo);

    let options = CmdOptions::default()
        .with_cwd(tmp.path())
        .with_env("CLAUDE_CONFIG_DIR", &path_str(&config))
        .with_env("AGENT_SESSION_CLAUDE_RESUME_SCAN_MAX_ENTRIES", "1");
    let output = run(&["agent", "resume", "trunc-id"], &options);

    assert_exit(&output, 1);
    assert!(stderr(&output).contains("truncated"));
}

#[test]
fn agent_resume_cd_override_bypasses_resolution() {
    let tmp = tempfile::TempDir::new().expect("tmp");
    let override_dir = tmp.path().join("override target");
    std::fs::create_dir_all(&override_dir).expect("override dir");
    // Empty project history: automatic resolution would fail with NotFound, so a
    // successful launch proves `--cd` bypassed resolution entirely.
    let config = tmp.path().join("claude-config");
    std::fs::create_dir_all(config.join("projects")).expect("projects");

    let stub_dir = tmp.path().join("stub");
    std::fs::create_dir_all(&stub_dir).expect("stub dir");
    nils_test_support::write_exe(
        &stub_dir,
        "claude",
        "#!/bin/sh\n: > launched-here\nexit 5\n",
    );

    let options = CmdOptions::default()
        .with_cwd(tmp.path())
        .with_env("CLAUDE_CONFIG_DIR", &path_str(&config))
        .with_fake_claude(&stub_dir);
    let output = run(
        &[
            "agent",
            "resume",
            "unresolved-id",
            "--cd",
            &path_str(&override_dir),
        ],
        &options,
    );

    assert_exit(&output, 5);
    assert!(
        override_dir.join("launched-here").exists(),
        "expected claude to be launched in the --cd override directory"
    );
}
