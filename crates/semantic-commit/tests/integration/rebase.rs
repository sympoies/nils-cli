#[path = "rebase_fixture.rs"]
mod fixture;
use fixture::Fixture;
use pretty_assertions::assert_eq;
use std::fs;

#[test]
fn selected_identity_signs_replay_with_unavailable_ambient_key() {
    let Some(f) = Fixture::new(false) else { return };
    let ambient = f.git_at(&f.worktree, &["rebase", "main"]);
    assert_ne!(ambient.code, 0);
    assert!(ambient.stderr_text().contains("failed to sign"));
    f.git(&["rebase", "--abort"]);
    let common = std::path::PathBuf::from(f.git(&[
        "rev-parse",
        "--path-format=absolute",
        "--git-common-dir",
    ]));
    let local_config = fs::read(common.join("config")).unwrap();
    let ambient_config = fs::read(f.root.path().join(".gitconfig")).unwrap();
    let replay = f.replay(&["main"]);
    assert_eq!(replay.code, 0, "{}", replay.stderr_text());
    f.verify(b"topic\n");
    assert_eq!(fs::read(common.join("config")).unwrap(), local_config);
    assert_eq!(
        fs::read(f.root.path().join(".gitconfig")).unwrap(),
        ambient_config
    );
}

#[test]
fn selected_identity_signs_conflict_continuation_and_preserves_resolution() {
    let Some(f) = Fixture::new(true) else { return };
    // Continue must retain the original branch's remote choice after HEAD
    // detaches, even when repository pushDefault points somewhere else.
    f.git(&[
        "remote",
        "add",
        "other",
        "https://github.com/other/widget.git",
    ]);
    f.git(&["config", "branch.fix/replay-fixture.pushRemote", "origin"]);
    f.git(&["config", "remote.pushDefault", "other"]);
    let replay = f.replay(&["main"]);
    assert_ne!(replay.code, 0);
    assert!(replay.stderr_text().contains("--continue"));
    assert!(!f.git(&["ls-files", "--unmerged"]).is_empty());
    fs::write(f.worktree.join("base"), b"resolved\n").unwrap();
    f.git(&["add", "base"]);
    let continued = f.replay(&["--continue"]);
    assert_eq!(continued.code, 0, "{}", continued.stderr_text());
    f.verify(b"resolved\n");
}

#[test]
fn continuation_refusals_preserve_staged_resolution_and_redact_diagnostics() {
    let Some(f) = Fixture::new(true) else { return };
    assert_ne!(f.replay(&["main"]).code, 0);
    assert!(!f.git(&["ls-files", "--unmerged"]).is_empty());
    fs::write(f.worktree.join("base"), b"resolved\n").unwrap();
    f.git(&["add", "base"]);
    let head = f.git(&["rev-parse", "HEAD"]);
    let index = std::path::PathBuf::from(f.git(&[
        "rev-parse",
        "--path-format=absolute",
        "--git-path",
        "index",
    ]));
    let state = std::path::PathBuf::from(f.git(&[
        "rev-parse",
        "--path-format=absolute",
        "--git-path",
        "rebase-merge",
    ]));
    let original_index = fs::read(&index).unwrap();
    let original_stages = f.git(&["ls-files", "--stage"]);
    for (name, unsupported, code) in [
        (
            "gpg_sign_opt",
            "-S0000000000000000000000000000000000000000\n",
            "identity_replay_signing_state_unsupported",
        ),
        (
            "git-rebase-todo",
            "exec false\n",
            "identity_replay_state_unsupported",
        ),
    ] {
        let path = state.join(name);
        let original = fs::read(&path).unwrap();
        fs::write(&path, unsupported).unwrap();
        let refused = f.replay(&["--continue"]);
        assert_ne!(refused.code, 0);
        assert!(
            refused.stderr_text().contains(code),
            "{}",
            refused.stderr_text()
        );
        assert_eq!(f.git(&["rev-parse", "HEAD"]), head);
        assert_eq!(fs::read(&index).unwrap(), original_index);
        fs::write(path, original).unwrap();
    }
    f.stubs.write_exe(
        "gh",
        "#!/bin/sh\nprintf '%s\\n' '{\"login\":\"wrong-actor\"}'\n",
    );
    let refused = f.replay(&["--continue"]);
    assert_ne!(refused.code, 0);
    assert!(refused.stderr_text().contains("identity_actor_mismatch"));
    assert!(
        refused
            .stderr_text()
            .contains("forge-cli identity doctor --operation commit")
    );
    assert!(!refused.stderr_text().contains(fixture::CANARY));
    assert_eq!(f.git(&["rev-parse", "HEAD"]), head);
    assert_eq!(fs::read(&index).unwrap(), original_index);
    assert!(state.exists());
    assert_eq!(fs::read(f.worktree.join("base")).unwrap(), b"resolved\n");
    f.stubs.write_exe(
        "gh",
        "#!/bin/sh\nprintf '%s\\n' '{\"login\":\"replay-agent\"}'\n",
    );
    let real_keys = nils_test_support::cmd::run_with(
        std::path::Path::new("gpg"),
        &["--batch", "--with-colons", "--list-secret-keys"],
        &f.options(&f.worktree),
    );
    assert_eq!(real_keys.code, 0);
    f.stubs.write_exe("gpg", &format!("#!/bin/sh\ncase \"$*\" in\n *--list-secret-keys*) printf '%s' '{}' ;;\n *) printf '%s\\n' 'signing failed: {}' >&2; exit 1 ;;\nesac\n", real_keys.stdout_text(), fixture::CANARY));
    let failed = f.replay(&["--continue"]);
    assert_ne!(failed.code, 0);
    assert!(!failed.stderr_text().contains(fixture::CANARY));
    assert!(failed.stderr_text().contains("--continue"));
    assert_eq!(f.git(&["rev-parse", "HEAD"]), head);
    // Git may refresh index extensions once continuation starts. The staged
    // objects and resolution must survive, rather than requiring identical
    // internal cache bytes after a signing attempt.
    assert_eq!(f.git(&["ls-files", "--stage"]), original_stages);
    assert!(state.exists());
    assert_eq!(fs::read(f.worktree.join("base")).unwrap(), b"resolved\n");
    let audit =
        fs::read_to_string(f.root.path().join("state/forge-cli/identity-audit.jsonl")).unwrap();
    for outcome in ["identity_actor_mismatch", "execution_failed"] {
        assert!(audit.lines().any(
            |line| serde_json::from_str::<serde_json::Value>(line).unwrap()["outcome"] == outcome
        ));
    }
    assert!(!audit.contains(fixture::CANARY));
    let last: serde_json::Value = serde_json::from_str(audit.lines().last().unwrap()).unwrap();
    assert_eq!(last["outcome"], "execution_failed");
    assert!(last["object"].is_null());
}

#[test]
fn replay_refuses_default_branch_and_unsafe_arguments_before_mutation() {
    let Some(f) = Fixture::new(false) else { return };
    let original = f.git(&["rev-parse", "HEAD"]);
    for args in [
        vec!["--exec", "false", "main"],
        vec!["--no-gpg-sign", "main"],
        vec!["main", "other-branch"],
        vec!["--continue", "main"],
    ] {
        assert_eq!(f.replay(&args).code, 64);
        assert_eq!(f.git(&["rev-parse", "HEAD"]), original);
    }
    f.git(&[
        "update-ref",
        "refs/remotes/origin/fix/replay-fixture",
        "HEAD",
    ]);
    f.git(&[
        "symbolic-ref",
        "refs/remotes/origin/HEAD",
        "refs/remotes/origin/fix/replay-fixture",
    ]);
    let refused = f.replay(&["main"]);
    assert_ne!(refused.code, 0);
    assert!(
        refused
            .stderr_text()
            .contains("identity_replay_default_branch_forbidden"),
        "{}",
        refused.stderr_text()
    );
    assert_eq!(f.git(&["rev-parse", "HEAD"]), original);
}
