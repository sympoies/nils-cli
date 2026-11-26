use super::{RemovalFixture, git, parse_json};
use nils_test_support::cmd::{CmdOutput, run_with};
use pretty_assertions::assert_eq;
use std::{fs, path::Path};

fn head(target: &Path) -> String {
    git(target, &["rev-parse", "HEAD"]).trim().to_owned()
}

fn remove(fixture: &RemovalFixture, target: &str, predicates: &[&str]) -> CmdOutput {
    let mut args = vec!["worktree", "remove", target, "--format", "json"];
    args.extend_from_slice(predicates);
    run_with(
        &fixture.harness.git_cli_bin(),
        &args,
        &fixture.options(fixture.repo.path()),
    )
}

#[test]
fn conditional_removal_accepts_clean_expected_ancestor() {
    let fixture = RemovalFixture::new();
    let target = Path::new(&fixture.target);
    let lease = seed_caller_lease(&fixture);
    let expected = head(target);
    git(
        fixture.repo.path(),
        &["commit", "--allow-empty", "-m", "Advance main"],
    );
    let result = remove(
        &fixture,
        "safe",
        &[
            "--expect-head",
            &expected,
            "--require-clean",
            "--ancestor-of",
            "refs/heads/main",
        ],
    );
    assert_eq!(result.code, 0, "{}", result.stdout_text());
    let receipt = parse_json(&result);
    assert_eq!(receipt["data"]["removed_head"], expected);
    assert!(receipt["data"]["backup_ref"].is_null());
    assert!(!target.exists());
    assert!(!lease.exists());
    assert_eq!(
        git(fixture.repo.path(), &["rev-parse", "refs/heads/feat/safe"]).trim(),
        expected
    );
}

fn simulate_holder(fixture: &RemovalFixture) {
    fixture.probes.write_exe(
        "agent-session",
        &format!(
            "#!/bin/sh\nprintf '%s\\n' '{}'\n",
            serde_json::json!({"ok": true, "data": [
                {"status": "running", "session_id": "fixture-holder", "cwd": fixture.target}
            ]})
        ),
    );
}

fn seed_caller_lease(fixture: &RemovalFixture) -> std::path::PathBuf {
    use sha2::{Digest, Sha256};
    use std::os::unix::fs::PermissionsExt;

    // Let a harmless holder refusal initialize the checkout instance and locks.
    simulate_holder(fixture);
    fixture.refused("removal-session-active");
    fixture.probes.write_exe(
        "agent-session",
        "#!/bin/sh\nprintf '%s\\n' '{\"ok\":true,\"data\":[]}'\n",
    );

    let target = Path::new(&fixture.target).canonicalize().unwrap();
    let common = fixture.repo.path().join(".git").canonicalize().unwrap();
    let git_dir = git(&target, &["rev-parse", "--absolute-git-dir"]);
    let git_dir = Path::new(git_dir.trim());
    let instance = fs::read_to_string(git_dir.join(".agent-runtime-checkout-instance")).unwrap();
    let key = |bytes: &[u8]| {
        Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    };
    let path = fixture
        .home
        .path()
        .join("lease-state")
        .join(key(common.as_os_str().as_encoded_bytes()))
        .join(key(target.as_os_str().as_encoded_bytes()))
        .join("lease.json");
    let lease = serde_json::json!({
        "schema": "agent-runtime.checkout-lease.v1",
        "session_key": key(b"fixture-caller"), "checkout_instance": instance.trim(),
        "checkout_root": target, "checkout_git_dir": git_dir,
        "acquired_at": 1, "refreshed_at": 1, "expires_at": 4102444800u64
    });
    fs::write(&path, serde_json::to_vec(&lease).unwrap()).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    path
}

#[derive(Debug, PartialEq, Eq)]
struct Checkout {
    head: String,
    index: String,
    status: String,
    files: Vec<(String, Vec<u8>)>,
}

fn checkout(target: &Path) -> Checkout {
    let mut files: Vec<_> = fs::read_dir(target)
        .unwrap()
        .map(|entry| entry.unwrap())
        .filter(|entry| entry.file_name() != ".git")
        .map(|entry| {
            (
                entry.file_name().into_string().unwrap(),
                fs::read(entry.path()).unwrap(),
            )
        })
        .collect();
    files.sort();
    Checkout {
        head: head(target),
        index: git(target, &["ls-files", "--stage"]),
        status: git(
            target,
            &[
                "--no-optional-locks",
                "status",
                "--porcelain",
                "--untracked-files=all",
            ],
        ),
        files,
    }
}

fn assert_retained(
    fixture: &RemovalFixture,
    output: &CmdOutput,
    expected_code: &str,
    before: &Checkout,
    lease: &Path,
    lease_bytes: &[u8],
) {
    assert_ne!(output.code, 0, "{}", output.stdout_text());
    let receipt = parse_json(output);
    assert_eq!(
        receipt["error"]["code"],
        expected_code,
        "{}",
        output.stdout_text()
    );
    assert_eq!(receipt["error"]["details"]["phase"], "fence");
    assert_eq!(receipt["error"]["details"]["worktree_mutation"], "none");
    assert_eq!(&checkout(Path::new(&fixture.target)), before);
    assert_eq!(fs::read(lease).unwrap(), lease_bytes);
    assert!(
        git(
            fixture.repo.path(),
            &["for-each-ref", "refs/worktree-backup"]
        )
        .is_empty()
    );
    assert!(
        git(fixture.repo.path(), &["worktree", "list", "--porcelain"]).contains(&fixture.target)
    );
}

fn inject_git(fixture: &RemovalFixture, action: &str) {
    let git = std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .map(|path| path.join("git"))
        .find(|path| path.is_file())
        .expect("git executable");
    fixture.probes.write_exe(
        "git",
        &format!(
            "#!/bin/sh\ncase \"$*\" in\n{action}\nesac\nexec \"{}\" \"$@\"\n",
            git.display(),
        ),
    );
}

#[test]
fn conditional_removal_rechecks_after_sampling_and_retains_changes() {
    use std::{
        thread,
        time::{Duration, Instant},
    };

    for scenario in ["tracked", "staged", "untracked", "changed-head", "unmerged"] {
        let fixture = RemovalFixture::new();
        let target = Path::new(&fixture.target);
        let lease = seed_caller_lease(&fixture);
        let lease_bytes = fs::read(&lease).unwrap();
        let sampled_head = head(target);
        // The ancestry-only case pauses before the transaction reads its first
        // HEAD. The other cases pause later, after both fences are held and
        // removal has already sampled HEAD.
        let barrier = if scenario == "unmerged" {
            "rev-parse --verify HEAD"
        } else {
            "for-each-ref --contains "
        };
        inject_git(
            &fixture,
            &format!(
                "\"{barrier}\"*) if [ ! -f \"$AGENT_HOME/resume\" ]; then\n\
             : > \"$AGENT_HOME/ready\"\n\
             while [ ! -f \"$AGENT_HOME/resume\" ]; do sleep 0.01; done\n\
             fi ;;"
            ),
        );
        let predicates = if scenario == "unmerged" {
            vec!["--require-clean", "--ancestor-of", "refs/heads/main"]
        } else {
            vec![
                "--expect-head",
                &sampled_head,
                "--require-clean",
                "--ancestor-of",
                "refs/heads/main",
            ]
        };
        thread::scope(|scope| {
            let removing = scope.spawn(|| remove(&fixture, "safe", &predicates));
            let deadline = Instant::now() + Duration::from_secs(5);
            while !fixture.home.path().join("ready").is_file() {
                assert!(
                    !removing.is_finished(),
                    "removal exited before the barrier: {scenario}"
                );
                assert!(
                    Instant::now() < deadline,
                    "barrier was not reached: {scenario}"
                );
                thread::sleep(Duration::from_millis(10));
            }
            match scenario {
                "tracked" | "staged" => {
                    fs::write(target.join("README.md"), "changed tracked bytes").unwrap();
                    if scenario == "staged" {
                        git(target, &["add", "README.md"]);
                    }
                }
                "untracked" => fs::write(target.join("untracked"), b"untracked\0bytes").unwrap(),
                "changed-head" | "unmerged" => fixture.commit(),
                _ => unreachable!(),
            }
            let before = checkout(target);
            fs::write(fixture.home.path().join("resume"), "").unwrap();
            let result = removing.join().unwrap();
            let code = match scenario {
                "changed-head" => "removal-head-mismatch",
                "unmerged" => "removal-not-ancestor",
                _ => "removal-dirty",
            };
            assert_retained(&fixture, &result, code, &before, &lease, &lease_bytes);
        });
    }
}

#[test]
fn conditional_removal_retains_without_local_proof() {
    for scenario in [
        "wrong-head",
        "missing-ref",
        "broken-ref",
        "ancestry-error",
        "status-error",
        "hidden-index",
        "missing-fence",
        "holder",
        "git-lock",
    ] {
        let fixture = RemovalFixture::new();
        let target = Path::new(&fixture.target);
        let lease = seed_caller_lease(&fixture);
        let lease_bytes = fs::read(&lease).unwrap();
        let zero = "0".repeat(head(target).len());
        let mut options = fixture.options(fixture.repo.path());
        let (predicates, code) = match scenario {
            "wrong-head" => (vec!["--expect-head", &zero], "removal-head-mismatch"),
            "missing-ref" => (
                vec!["--ancestor-of", "refs/heads/missing"],
                "removal-ref-unavailable",
            ),
            "broken-ref" => {
                fs::write(
                    fixture.repo.path().join(".git/refs/heads/broken"),
                    "invalid\n",
                )
                .unwrap();
                (
                    vec!["--ancestor-of", "refs/heads/broken"],
                    "removal-ref-unavailable",
                )
            }
            "ancestry-error" => {
                inject_git(
                    &fixture,
                    "*\"merge-base --is-ancestor\"*) case \"$*\" in\n\
                     *core.commitGraph=false*)\n\
                     test \"${GIT_NO_LAZY_FETCH:-}\" = 1 && test \"${GIT_ALLOW_PROTOCOL-unset}\" = '' || exit 1\n\
                     exit 128 ;; esac ;;",
                );
                (
                    vec!["--ancestor-of", "refs/heads/main"],
                    "removal-proof-unavailable",
                )
            }
            "status-error" => {
                inject_git(
                    &fixture,
                    "*\"status --porcelain=v1 -z\"*) printf 'warning: unreadable path\\n' >&2; exit 0 ;;",
                );
                (vec!["--require-clean"], "removal-proof-unavailable")
            }
            "hidden-index" => {
                git(target, &["update-index", "--assume-unchanged", "README.md"]);
                fs::write(target.join("README.md"), "hidden edit").unwrap();
                (vec!["--require-clean"], "removal-proof-unavailable")
            }
            "missing-fence" => {
                // A file at the lifecycle directory makes its lock unavailable.
                let state = fixture.home.path().join("unavailable-state");
                fs::write(&state, "").unwrap();
                options = options.with_env(
                    "AGENT_RUNTIME_CHECKOUT_LEASE_STATE_HOME",
                    state.to_str().unwrap(),
                );
                (vec!["--require-clean"], "removal-proof-unavailable")
            }
            "holder" => {
                simulate_holder(&fixture);
                (vec!["--require-clean"], "removal-session-active")
            }
            "git-lock" => {
                let git_dir = git(target, &["rev-parse", "--absolute-git-dir"]);
                fs::write(Path::new(git_dir.trim()).join("index.lock"), "").unwrap();
                (vec!["--require-clean"], "removal-git-busy")
            }
            _ => unreachable!(),
        };
        let before = checkout(target);
        let mut args = vec!["worktree", "remove", "safe", "--format", "json"];
        args.extend(predicates);
        let result = run_with(&fixture.harness.git_cli_bin(), &args, &options);
        assert_retained(&fixture, &result, code, &before, &lease, &lease_bytes);
    }
}

#[test]
fn conditional_removal_batch_keeps_per_target_results_and_branches() {
    let fixture = RemovalFixture::new();
    let options = fixture.options(fixture.repo.path());
    let second = run_with(
        &fixture.harness.git_cli_bin(),
        &[
            "worktree", "add", "second", "--from", "main", "--format", "json",
        ],
        &options,
    );
    assert_eq!(second.code, 0, "{}", second.stderr_text());
    let second = parse_json(&second);
    let second = second["data"]["path"].as_str().unwrap();
    let expected = head(Path::new(second));
    fs::write(Path::new(&fixture.target).join("untracked"), "keep").unwrap();
    let predicates = [
        "--expect-head",
        &expected,
        "--require-clean",
        "--ancestor-of",
        "refs/heads/main",
    ];
    let receipts: Vec<_> = [&fixture.target, second]
        .into_iter()
        .map(|target| remove(&fixture, target, &predicates))
        .collect();
    assert_ne!(receipts[0].code, 0);
    assert_eq!(parse_json(&receipts[0])["error"]["code"], "removal-dirty");
    assert_eq!(receipts[1].code, 0, "{}", receipts[1].stdout_text());
    assert!(Path::new(&fixture.target).join("untracked").is_file());
    assert!(!Path::new(second).exists());
    for branch in ["refs/heads/feat/safe", "refs/heads/feat/second"] {
        assert_eq!(
            git(fixture.repo.path(), &["rev-parse", branch]).trim(),
            expected
        );
    }
}
