use crate::support::*;
use nils_test_support::git as test_git;
use pretty_assertions::assert_eq;

#[cfg(unix)]
#[test]
fn agent_commit_uses_bounded_structured_output_and_semantic_commit_only() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let repo = tmp.path().join("repo");
    init_git_repo(&repo);
    std::fs::write(repo.join("change.txt"), "change\n").expect("change");
    test_git::git(&repo, &["add", "change.txt"]);
    let old_head = test_git::git(&repo, &["rev-parse", "HEAD"])
        .trim()
        .to_string();

    let claude_argv = tmp.path().join("claude-argv.log");
    let claude_stdin = tmp.path().join("claude-stdin.log");
    let semantic_log = tmp.path().join("semantic.log");
    let bin_dir = write_fake_claude(
        tmp.path(),
        r#"#!/bin/sh
set -eu
if [ "${1:-}" = "--help" ]; then
  printf '%s\n' '--print --output-format --json-schema --safe-mode --strict-mcp-config --no-session-persistence --permission-mode --disable-slash-commands --no-chrome --tools --append-system-prompt --model --effort'
  exit 0
fi
: > "$CLAUDE_TEST_ARGV_LOG"
printf 'CWD:%s\n' "$PWD" >> "$CLAUDE_TEST_ARGV_LOG"
for arg in "$@"; do printf 'ARG:%s\n' "$arg" >> "$CLAUDE_TEST_ARGV_LOG"; done
cat > "$CLAUDE_TEST_STDIN_LOG"
printf '%s\n' '[{"type":"result","subtype":"success","is_error":false,"structured_output":{"type":"fix","scope":"agent","subject":"add safe commit workflow","body_bullets":["Keep the index staged on failure"]}}]'
"#,
    );
    nils_test_support::write_exe(
        &bin_dir,
        "semantic-commit",
        r#"#!/bin/sh
set -eu
printf '%s\n' "$*" >> "$SEMANTIC_TEST_LOG"
if [ "${1:-}" = "staged-context" ]; then
  printf '%s\n' 'STAGED BUNDLE'
  exit 0
fi
repo=''
previous=''
for arg in "$@"; do
  if [ "$previous" = '--repo' ]; then repo="$arg"; fi
  previous="$arg"
done
"$REAL_GIT" -C "$repo" commit -m 'fix(agent): add safe commit workflow' >/dev/null
"#,
    );
    let real_git = nils_common::process::find_in_path("git").expect("git");
    let output = run(
        &[
            "agent", "commit", "--model", "sonnet", "--effort", "high", "prefer", "a", "small",
            "scope",
        ],
        &base_options(tmp.path())
            .with_cwd(&repo)
            .with_fake_claude(&bin_dir)
            .with_env("CLAUDE_TEST_ARGV_LOG", &path_str(&claude_argv))
            .with_env("CLAUDE_TEST_STDIN_LOG", &path_str(&claude_stdin))
            .with_env("SEMANTIC_TEST_LOG", &path_str(&semantic_log))
            .with_env("REAL_GIT", &path_str(&real_git)),
    );

    assert_exit(&output, 0);
    assert_ne!(
        test_git::git(&repo, &["rev-parse", "HEAD"]).trim(),
        old_head
    );
    let argv = std::fs::read_to_string(claude_argv).expect("claude argv");
    assert!(argv.contains("ARG:--json-schema\n"));
    assert!(argv.contains("ARG:--safe-mode\n"));
    assert!(argv.contains("ARG:--strict-mcp-config\n"));
    assert!(argv.contains("ARG:--no-session-persistence\n"));
    assert!(argv.contains("ARG:--tools\nARG:\n"));
    assert!(!argv.contains("Bash"));
    assert!(!argv.contains(&format!("CWD:{}", repo.display())));
    let prompt = std::fs::read_to_string(claude_stdin).expect("claude stdin");
    assert!(prompt.contains("STAGED BUNDLE"));
    assert!(prompt.contains("prefer a small scope"));
    let semantic = std::fs::read_to_string(semantic_log).expect("semantic log");
    assert!(semantic.contains("staged-context --format bundle --repo"));
    assert!(semantic.contains(
        "commit --type fix --scope agent --subject add safe commit workflow \
--body-bullet Keep the index staged on failure"
    ));
    assert!(semantic.contains(&format!("--expect-head {old_head}")));
    assert!(semantic.contains("--automation"));
}

#[cfg(unix)]
#[test]
fn agent_commit_rejects_index_drift_and_leaves_changes_staged() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let repo = tmp.path().join("repo");
    init_git_repo(&repo);
    std::fs::write(repo.join("change.txt"), "original\n").expect("change");
    test_git::git(&repo, &["add", "change.txt"]);
    let old_head = test_git::git(&repo, &["rev-parse", "HEAD"])
        .trim()
        .to_string();

    let semantic_log = tmp.path().join("semantic.log");
    let bin_dir = write_fake_claude(
        tmp.path(),
        r#"#!/bin/sh
set -eu
if [ "${1:-}" = "--help" ]; then
  printf '%s\n' '--print --output-format --json-schema --safe-mode --strict-mcp-config --no-session-persistence --permission-mode --disable-slash-commands --no-chrome --tools --append-system-prompt --model --effort'
  exit 0
fi
printf '%s\n' 'mutated during model call' > "$CLAUDE_TEST_REPO/change.txt"
"$REAL_GIT" -C "$CLAUDE_TEST_REPO" add change.txt
cat >/dev/null
printf '%s\n' '{"type":"result","subtype":"success","is_error":false,"structured_output":{"type":"fix","scope":null,"subject":"must not commit drift","body_bullets":[]}}'
"#,
    );
    nils_test_support::write_exe(
        &bin_dir,
        "semantic-commit",
        r#"#!/bin/sh
set -eu
printf '%s\n' "$*" >> "$SEMANTIC_TEST_LOG"
if [ "${1:-}" = "staged-context" ]; then
  printf '%s\n' 'STAGED BUNDLE'
  exit 0
fi
exit 97
"#,
    );
    let real_git = nils_common::process::find_in_path("git").expect("git");
    let output = run(
        &["agent", "commit"],
        &base_options(tmp.path())
            .with_cwd(&repo)
            .with_fake_claude(&bin_dir)
            .with_env("SEMANTIC_TEST_LOG", &path_str(&semantic_log))
            .with_env("CLAUDE_TEST_REPO", &path_str(&repo))
            .with_env("REAL_GIT", &path_str(&real_git)),
    );

    assert_exit(&output, 1);
    assert!(
        stderr(&output).contains("repository changed during message generation"),
        "stderr: {}",
        stderr(&output)
    );
    assert_eq!(
        test_git::git(&repo, &["rev-parse", "HEAD"]).trim(),
        old_head
    );
    assert!(
        test_git::git(&repo, &["diff", "--cached", "--name-only"])
            .lines()
            .any(|line| line == "change.txt")
    );
    let semantic = std::fs::read_to_string(semantic_log).expect("semantic log");
    assert_eq!(
        semantic
            .lines()
            .filter(|line| line.starts_with("commit "))
            .count(),
        0
    );
}

#[cfg(unix)]
#[test]
fn agent_commit_auto_stages_and_pushes_the_verified_commit_to_its_captured_upstream() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let repo = tmp.path().join("repo");
    let remote = tmp.path().join("remote.git");
    init_git_repo(&repo);
    test_git::git(&repo, &["branch", "-M", "agent-e2e"]);
    test_git::git(
        tmp.path(),
        &["init", "--bare", remote.to_str().expect("remote path")],
    );
    test_git::git(
        &repo,
        &[
            "remote",
            "add",
            "origin",
            remote.to_str().expect("remote path"),
        ],
    );
    test_git::git(&repo, &["push", "-u", "origin", "agent-e2e"]);
    std::fs::write(repo.join("auto-stage.txt"), "auto-stage and push\n").expect("change");
    let old_head = test_git::git(&repo, &["rev-parse", "HEAD"])
        .trim()
        .to_string();
    let bin_dir = write_agent_commit_success_tools(tmp.path());
    let real_git = nils_common::process::find_in_path("git").expect("git");

    let output = run(
        &["agent", "commit", "--auto-stage", "--push"],
        &base_options(tmp.path())
            .with_cwd(&repo)
            .with_fake_claude(&bin_dir)
            .with_env("REAL_GIT", &path_str(&real_git)),
    );

    assert_exit(&output, 0);
    let new_head = test_git::git(&repo, &["rev-parse", "HEAD"])
        .trim()
        .to_string();
    assert_ne!(new_head, old_head);
    assert_eq!(
        test_git::git(
            tmp.path(),
            &[
                "--git-dir",
                remote.to_str().expect("remote path"),
                "rev-parse",
                "refs/heads/agent-e2e",
            ],
        )
        .trim(),
        new_head
    );
    assert!(
        test_git::git(&repo, &["status", "--porcelain"])
            .trim()
            .is_empty()
    );
}

#[cfg(unix)]
#[test]
fn agent_commit_refuses_push_when_the_captured_endpoint_is_retargeted() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let repo = tmp.path().join("repo");
    let original = tmp.path().join("original.git");
    let alternate = tmp.path().join("alternate.git");
    init_git_repo(&repo);
    test_git::git(&repo, &["branch", "-M", "agent-e2e"]);
    for remote in [&original, &alternate] {
        test_git::git(
            tmp.path(),
            &["init", "--bare", remote.to_str().expect("remote path")],
        );
    }
    test_git::git(
        &repo,
        &[
            "remote",
            "add",
            "origin",
            original.to_str().expect("original path"),
        ],
    );
    test_git::git(&repo, &["push", "-u", "origin", "agent-e2e"]);
    std::fs::write(repo.join("retarget.txt"), "do not retarget push\n").expect("change");
    test_git::git(&repo, &["add", "retarget.txt"]);
    let old_head = test_git::git(&repo, &["rev-parse", "HEAD"])
        .trim()
        .to_string();
    let bin_dir = write_agent_commit_success_tools(tmp.path());
    let real_git = nils_common::process::find_in_path("git").expect("git");

    let output = run(
        &["agent", "commit", "--push"],
        &base_options(tmp.path())
            .with_cwd(&repo)
            .with_fake_claude(&bin_dir)
            .with_env("REAL_GIT", &path_str(&real_git))
            .with_env(
                "SEMANTIC_TEST_RETARGET_URL",
                alternate.to_str().expect("alternate path"),
            ),
    );

    assert_exit(&output, 1);
    assert!(
        stderr(&output).contains("push endpoint changed before push"),
        "stderr: {}",
        stderr(&output)
    );
    let new_head = test_git::git(&repo, &["rev-parse", "HEAD"])
        .trim()
        .to_string();
    assert_ne!(new_head, old_head);
    assert_eq!(
        test_git::git(
            tmp.path(),
            &[
                "--git-dir",
                original.to_str().expect("original path"),
                "rev-parse",
                "refs/heads/agent-e2e",
            ],
        )
        .trim(),
        old_head
    );
    let alternate_head = test_git::git_output(
        tmp.path(),
        &[
            "--git-dir",
            alternate.to_str().expect("alternate path"),
            "rev-parse",
            "--verify",
            "refs/heads/agent-e2e",
        ],
    );
    assert!(!alternate_head.status.success());
}

#[cfg(unix)]
#[test]
fn agent_commit_pins_the_captured_endpoint_against_chained_url_rewrites() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let repo = tmp.path().join("repo");
    let original = tmp.path().join("original.git");
    let alternate = tmp.path().join("alternate.git");
    init_git_repo(&repo);
    test_git::git(&repo, &["branch", "-M", "agent-e2e"]);
    for remote in [&original, &alternate] {
        test_git::git(
            tmp.path(),
            &["init", "--bare", remote.to_str().expect("remote path")],
        );
    }
    test_git::git(
        &repo,
        &[
            "push",
            original.to_str().expect("original path"),
            "agent-e2e:refs/heads/agent-e2e",
        ],
    );
    test_git::git(&repo, &["remote", "add", "origin", "seed:"]);
    test_git::git(&repo, &["config", "branch.agent-e2e.remote", "origin"]);
    test_git::git(
        &repo,
        &["config", "branch.agent-e2e.merge", "refs/heads/agent-e2e"],
    );
    test_git::git(
        &repo,
        &[
            "config",
            &format!(
                "url.{}.pushInsteadOf",
                original.to_str().expect("original path")
            ),
            "seed:",
        ],
    );
    test_git::git(
        &repo,
        &[
            "config",
            &format!(
                "url.{}.insteadOf",
                alternate.to_str().expect("alternate path")
            ),
            original.to_str().expect("original path"),
        ],
    );
    assert_eq!(
        test_git::git(&repo, &["remote", "get-url", "--push", "origin"]).trim(),
        original.to_str().expect("original path")
    );
    std::fs::write(repo.join("rewrite.txt"), "pin captured push endpoint\n").expect("change");
    test_git::git(&repo, &["add", "rewrite.txt"]);
    let bin_dir = write_agent_commit_success_tools(tmp.path());
    let real_git = nils_common::process::find_in_path("git").expect("git");

    let output = run(
        &["agent", "commit", "--push"],
        &base_options(tmp.path())
            .with_cwd(&repo)
            .with_fake_claude(&bin_dir)
            .with_env("REAL_GIT", &path_str(&real_git)),
    );

    assert_exit(&output, 0);
    let new_head = test_git::git(&repo, &["rev-parse", "HEAD"])
        .trim()
        .to_string();
    assert_eq!(
        test_git::git(
            tmp.path(),
            &[
                "--git-dir",
                original.to_str().expect("original path"),
                "rev-parse",
                "refs/heads/agent-e2e",
            ],
        )
        .trim(),
        new_head
    );
    let alternate_head = test_git::git_output(
        tmp.path(),
        &[
            "--git-dir",
            alternate.to_str().expect("alternate path"),
            "rev-parse",
            "--verify",
            "refs/heads/agent-e2e",
        ],
    );
    assert!(!alternate_head.status.success());
}

#[cfg(unix)]
#[test]
fn agent_commit_preserves_the_local_commit_when_push_fails() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let repo = tmp.path().join("repo");
    init_git_repo(&repo);
    test_git::git(&repo, &["branch", "-M", "agent-e2e"]);
    let missing_remote = tmp.path().join("missing-remote.git");
    test_git::git(
        &repo,
        &[
            "remote",
            "add",
            "origin",
            missing_remote.to_str().expect("remote path"),
        ],
    );
    test_git::git(&repo, &["config", "branch.agent-e2e.remote", "origin"]);
    test_git::git(
        &repo,
        &["config", "branch.agent-e2e.merge", "refs/heads/agent-e2e"],
    );
    std::fs::write(repo.join("push-failure.txt"), "preserve local commit\n").expect("change");
    test_git::git(&repo, &["add", "push-failure.txt"]);
    let old_head = test_git::git(&repo, &["rev-parse", "HEAD"])
        .trim()
        .to_string();
    let bin_dir = write_agent_commit_success_tools(tmp.path());
    let real_git = nils_common::process::find_in_path("git").expect("git");

    let output = run(
        &["agent", "commit", "--push"],
        &base_options(tmp.path())
            .with_cwd(&repo)
            .with_fake_claude(&bin_dir)
            .with_env("REAL_GIT", &path_str(&real_git)),
    );

    assert_ne!(output.code, 0);
    let new_head = test_git::git(&repo, &["rev-parse", "HEAD"])
        .trim()
        .to_string();
    assert_ne!(new_head, old_head);
    assert!(
        stderr(&output).contains("push failed; local commit was preserved"),
        "stderr: {}",
        stderr(&output)
    );
    assert!(
        test_git::git(&repo, &["show", "--pretty=", "--name-only", "HEAD"])
            .lines()
            .any(|line| line == "push-failure.txt")
    );
}

#[cfg(unix)]
#[test]
fn agent_commit_rejects_invalid_structured_message_and_preserves_index() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let repo = tmp.path().join("repo");
    init_git_repo(&repo);
    std::fs::write(repo.join("change.txt"), "change\n").expect("change");
    test_git::git(&repo, &["add", "change.txt"]);
    let old_head = test_git::git(&repo, &["rev-parse", "HEAD"])
        .trim()
        .to_string();
    let semantic_log = tmp.path().join("semantic.log");
    let bin_dir = write_fake_claude(
        tmp.path(),
        r#"#!/bin/sh
set -eu
if [ "${1:-}" = "--help" ]; then
  printf '%s\n' '--print --output-format --json-schema --safe-mode --strict-mcp-config --no-session-persistence --permission-mode --disable-slash-commands --no-chrome --tools --append-system-prompt'
  exit 0
fi
cat >/dev/null
printf '%s\n' '{"type":"result","subtype":"success","is_error":false,"structured_output":{"type":"feature","scope":null,"subject":"invalid type","body_bullets":[]}}'
"#,
    );
    nils_test_support::write_exe(
        &bin_dir,
        "semantic-commit",
        r#"#!/bin/sh
set -eu
printf '%s\n' "$*" >> "$SEMANTIC_TEST_LOG"
if [ "${1:-}" = "staged-context" ]; then
  printf '%s\n' 'STAGED BUNDLE'
  exit 0
fi
exit 97
"#,
    );
    let output = run(
        &["agent", "commit"],
        &base_options(tmp.path())
            .with_cwd(&repo)
            .with_fake_claude(&bin_dir)
            .with_env("SEMANTIC_TEST_LOG", &path_str(&semantic_log)),
    );

    assert_exit(&output, 65);
    assert!(stderr(&output).contains("invalid commit type"));
    assert!(!stdout(&output).contains("feature"));
    assert_eq!(
        test_git::git(&repo, &["rev-parse", "HEAD"]).trim(),
        old_head
    );
    assert!(
        test_git::git(&repo, &["diff", "--cached", "--name-only"])
            .lines()
            .any(|line| line == "change.txt")
    );
    let semantic = std::fs::read_to_string(semantic_log).expect("semantic log");
    assert_eq!(
        semantic
            .lines()
            .filter(|line| line.starts_with("commit "))
            .count(),
        0
    );
}

#[cfg(unix)]
#[test]
fn agent_commit_rejects_secret_like_staged_context_before_claude_launch() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let repo = tmp.path().join("repo");
    init_git_repo(&repo);
    std::fs::write(repo.join("change.txt"), "change\n").expect("change");
    test_git::git(&repo, &["add", "change.txt"]);
    let launched = tmp.path().join("claude-launched");
    let bin_dir = write_fake_claude(
        tmp.path(),
        r#"#!/bin/sh
set -eu
if [ "${1:-}" = "--help" ]; then
  printf '%s\n' '--print --output-format --json-schema --safe-mode --strict-mcp-config --no-session-persistence --permission-mode --disable-slash-commands --no-chrome --tools --append-system-prompt'
  exit 0
fi
: > "$CLAUDE_TEST_LAUNCHED"
exit 91
"#,
    );
    nils_test_support::write_exe(
        &bin_dir,
        "semantic-commit",
        r#"#!/bin/sh
set -eu
if [ "${1:-}" = "staged-context" ]; then
  printf '%s\n' 'api_key=supersecretvalue123'
  exit 0
fi
exit 97
"#,
    );
    let output = run(
        &["agent", "commit"],
        &base_options(tmp.path())
            .with_cwd(&repo)
            .with_fake_claude(&bin_dir)
            .with_env("CLAUDE_TEST_LAUNCHED", &path_str(&launched)),
    );

    assert_exit(&output, 65);
    assert!(stderr(&output).contains("secret-like content"));
    assert!(stderr(&output).contains("generic-secret-kv"));
    assert!(!stderr(&output).contains("supersecretvalue123"));
    assert!(!launched.exists());
    assert!(
        test_git::git(&repo, &["diff", "--cached", "--name-only"])
            .lines()
            .any(|line| line == "change.txt")
    );
}

#[cfg(unix)]
#[test]
fn agent_commit_probes_optional_model_and_effort_capabilities() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let repo = tmp.path().join("repo");
    init_git_repo(&repo);
    std::fs::write(repo.join("change.txt"), "change\n").expect("change");
    test_git::git(&repo, &["add", "change.txt"]);
    let launched = tmp.path().join("claude-launched");
    let bin_dir = write_fake_claude(
        tmp.path(),
        r#"#!/bin/sh
set -eu
if [ "${1:-}" = "--help" ]; then
  printf '%s\n' '--print --output-format --json-schema --safe-mode --strict-mcp-config --no-session-persistence --permission-mode --disable-slash-commands --no-chrome --tools --append-system-prompt --model'
  exit 0
fi
: > "$CLAUDE_TEST_LAUNCHED"
exit 91
"#,
    );
    nils_test_support::write_exe(
        &bin_dir,
        "semantic-commit",
        "#!/bin/sh\nprintf '%s\\n' 'STAGED BUNDLE'\n",
    );
    let output = run(
        &["agent", "commit", "--effort", "high"],
        &base_options(tmp.path())
            .with_cwd(&repo)
            .with_fake_claude(&bin_dir)
            .with_env("CLAUDE_TEST_LAUNCHED", &path_str(&launched)),
    );

    assert_exit(&output, 69);
    assert!(stderr(&output).contains("--effort"));
    assert!(!launched.exists());
}

#[cfg(unix)]
#[test]
fn agent_commit_rejects_created_commit_when_tree_does_not_match_snapshot() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let repo = tmp.path().join("repo");
    init_git_repo(&repo);
    std::fs::write(repo.join("change.txt"), "expected\n").expect("change");
    test_git::git(&repo, &["add", "change.txt"]);
    let bin_dir = write_fake_claude(
        tmp.path(),
        r#"#!/bin/sh
set -eu
if [ "${1:-}" = "--help" ]; then
  printf '%s\n' '--print --output-format --json-schema --safe-mode --strict-mcp-config --no-session-persistence --permission-mode --disable-slash-commands --no-chrome --tools --append-system-prompt'
  exit 0
fi
cat >/dev/null
printf '%s\n' '{"type":"result","subtype":"success","is_error":false,"structured_output":{"type":"test","scope":"agent","subject":"verify commit tree","body_bullets":[]}}'
"#,
    );
    nils_test_support::write_exe(
        &bin_dir,
        "semantic-commit",
        r#"#!/bin/sh
set -eu
if [ "${1:-}" = "staged-context" ]; then
  printf '%s\n' 'STAGED BUNDLE'
  exit 0
fi
repo=''
previous=''
for arg in "$@"; do
  if [ "$previous" = '--repo' ]; then repo="$arg"; fi
  previous="$arg"
done
printf '%s\n' 'unexpected' > "$repo/injected.txt"
"$REAL_GIT" -C "$repo" add injected.txt
"$REAL_GIT" -C "$repo" commit -m 'test(agent): verify commit tree' >/dev/null
"#,
    );
    let real_git = nils_common::process::find_in_path("git").expect("git");
    let output = run(
        &["agent", "commit"],
        &base_options(tmp.path())
            .with_cwd(&repo)
            .with_fake_claude(&bin_dir)
            .with_env("REAL_GIT", &path_str(&real_git)),
    );

    assert_exit(&output, 1);
    assert!(stderr(&output).contains("failed parent/tree integrity verification"));
    assert_eq!(
        test_git::git(&repo, &["show", "--pretty=", "--name-only", "HEAD"])
            .lines()
            .collect::<Vec<_>>(),
        vec!["change.txt", "injected.txt"]
    );
}
