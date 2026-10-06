//! Observable offline plans must expose the root-comment read dependency.
use pretty_assertions::assert_eq;
use std::{fs, os::unix::fs::PermissionsExt, process::Command};

#[test]
fn thread_mutation_dry_runs_declare_the_lookup_dependency_without_network_calls() {
    let temp = tempfile::tempdir().unwrap();
    let backend = temp.path().join("backend");
    fs::write(
        &backend,
        "#!/bin/sh\necho backend-must-not-run >&2\nexit 99\n",
    )
    .unwrap();
    fs::set_permissions(&backend, fs::Permissions::from_mode(0o755)).unwrap();
    for (operation, body_flag) in [("reply", "--body"), ("resolve", "--note")] {
        let output = Command::new(nils_test_support::bin::resolve("forge-cli"))
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", temp.path())
            .env("XDG_CONFIG_HOME", temp.path())
            .env("FORGE_CLI_GH_BIN", &backend)
            .current_dir(temp.path())
            .args([
                "--provider",
                "github",
                "--repo",
                "acme/widgets",
                "--format",
                "json",
                "--dry-run",
                "pr",
                "review-threads",
                operation,
                "7",
                "--thread",
                "PRRT_fixture",
                body_flag,
                "Fixed.",
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{} {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let envelope: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(
            envelope["data"]["root_comment_id_source"],
            "/data/node/comments/nodes/0/fullDatabaseId"
        );
        let target = envelope["data"]["target_plan"].to_string();
        assert!(
            target.contains("fullDatabaseId") && target.contains("viewerCanResolve"),
            "{target}"
        );
        let plan = envelope["data"]["plan"].to_string();
        assert!(
            plan.contains("comments/${root_comment_id}/replies"),
            "{plan}"
        );
        assert!(!plan.contains("ROOT_COMMENT_ID_PENDING"), "{plan}");
        assert!(!String::from_utf8_lossy(&output.stderr).contains("backend-must-not-run"));
    }
}

#[test]
fn thread_reply_dry_runs_without_repository_fail_with_typed_error() {
    let temp = tempfile::tempdir().unwrap();
    for (operation, flag) in [("reply", "--body"), ("resolve", "--note")] {
        let output = Command::new(nils_test_support::bin::resolve("forge-cli"))
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", temp.path())
            .env("XDG_CONFIG_HOME", temp.path())
            .current_dir(temp.path())
            .args([
                "--provider",
                "github",
                "--format",
                "json",
                "--dry-run",
                "pr",
                "review-threads",
                operation,
                "7",
                "--thread",
                "PRRT_fixture",
                flag,
                "Fixed.",
            ])
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(65),
            "{} {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let envelope: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(envelope["error"]["code"], "repo_required");
    }
}
