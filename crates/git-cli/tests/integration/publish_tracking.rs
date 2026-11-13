use crate::common::{GitCliHarness, git, init_bare_remote, init_repo};
use nils_test_support::cmd::{CmdOutput, run_with};
use nils_test_support::git::{commit_file, git_output};
use pretty_assertions::{assert_eq, assert_ne};
use serde_json::Value;
use std::path::PathBuf;
use tempfile::TempDir;

const BRANCH: &str = "fix/tracking-topic";

struct NarrowPublish {
    harness: GitCliHarness,
    clone: TempDir,
    home: TempDir,
    remote: TempDir,
    worktree: PathBuf,
    remote_name: &'static str,
    head: String,
}

impl NarrowPublish {
    fn new(remote_name: &'static str, empty_remote: bool) -> Self {
        let harness = GitCliHarness::new();
        let seed = init_repo();
        let seeded_remote = init_bare_remote();
        git(
            seed.path(),
            &[
                "remote",
                "add",
                "origin",
                seeded_remote.path().to_str().unwrap(),
            ],
        );
        git(seed.path(), &["push", "-q", "origin", "main"]);
        git(
            seeded_remote.path(),
            &["symbolic-ref", "HEAD", "refs/heads/main"],
        );
        let clone = TempDir::new().unwrap();
        git(
            seed.path(),
            &[
                "clone",
                "--single-branch",
                "--branch",
                "main",
                seeded_remote.path().to_str().unwrap(),
                clone.path().to_str().unwrap(),
            ],
        );
        git(clone.path(), &["config", "user.name", "Test"]);
        git(clone.path(), &["config", "user.email", "test@example.com"]);
        git(clone.path(), &["config", "commit.gpgsign", "false"]);
        if remote_name != "origin" {
            git(clone.path(), &["remote", "rename", "origin", remote_name]);
        }
        git(clone.path(), &["remote", "set-head", remote_name, "main"]);
        let home = TempDir::new().unwrap();
        let options = harness
            .cmd_options(clone.path())
            .with_env("AGENT_HOME", home.path().to_str().unwrap());
        let base = format!("{remote_name}/main");
        let add = run_with(
            &harness.git_cli_bin(),
            &[
                "worktree",
                "add",
                "tracking-topic",
                "--kind",
                "bug",
                "--from",
                &base,
                "--format",
                "json",
            ],
            &options,
        );
        assert_success(&add);
        let worktree = PathBuf::from(envelope(&add)["data"]["path"].as_str().unwrap());
        commit_file(&worktree, "topic.txt", "topic\n", "add topic");
        let head = git(&worktree, &["rev-parse", "HEAD"]).trim().to_owned();
        // An empty remote cannot seed a clone. Redirect the narrow repository
        // only after creating the branch whose first publication is under test.
        let remote = if empty_remote {
            let empty = init_bare_remote();
            git(
                &worktree,
                &[
                    "remote",
                    "set-url",
                    remote_name,
                    empty.path().to_str().unwrap(),
                ],
            );
            empty
        } else {
            seeded_remote
        };
        Self {
            harness,
            clone,
            home,
            remote,
            worktree,
            remote_name,
            head,
        }
    }

    fn push(&self, extra: &[&str]) -> CmdOutput {
        let mut args = vec!["push", "--remote", self.remote_name, "--format", "json"];
        args.extend_from_slice(extra);
        let options = self
            .harness
            .cmd_options(&self.worktree)
            .with_env("AGENT_HOME", self.home.path().to_str().unwrap());
        run_with(&self.harness.git_cli_bin(), &args, &options)
    }

    fn fetch_mappings(&self) -> String {
        git(
            &self.worktree,
            &[
                "config",
                "--get-all",
                &format!("remote.{}.fetch", self.remote_name),
            ],
        )
    }

    fn assert_published(&self) {
        assert_eq!(
            git(
                self.remote.path(),
                &["rev-parse", &format!("refs/heads/{BRANCH}")]
            )
            .trim(),
            self.head
        );
    }

    fn assert_tracked(&self) {
        self.assert_published();
        assert_eq!(
            git(&self.worktree, &["rev-parse", "@{upstream}"]).trim(),
            self.head
        );
        assert_eq!(
            git(
                &self.worktree,
                &[
                    "rev-parse",
                    &format!("refs/remotes/{}/{BRANCH}", self.remote_name)
                ]
            )
            .trim(),
            self.head
        );
        assert_eq!(
            self.fetch_mappings().lines().collect::<Vec<_>>(),
            vec![
                format!("+refs/heads/main:refs/remotes/{}/main", self.remote_name),
                format!(
                    "+refs/heads/{BRANCH}:refs/remotes/{}/{BRANCH}",
                    self.remote_name
                ),
            ]
        );
    }
}

fn envelope(output: &CmdOutput) -> Value {
    serde_json::from_str(output.stdout_text().trim()).unwrap()
}

fn assert_success(output: &CmdOutput) {
    assert_eq!(
        output.code,
        0,
        "{}{}",
        output.stdout_text(),
        output.stderr_text()
    );
}

fn assert_tracking_failed(output: &CmdOutput) {
    assert_ne!(output.code, 0, "{}", output.stdout_text());
    let json = envelope(output);
    assert_eq!(json["error"]["code"], "push-tracking-failed");
    assert!(
        json["error"]["message"]
            .as_str()
            .unwrap()
            .contains("pushed"),
        "{}",
        output.stdout_text()
    );
}

#[test]
fn bootstrap_materializes_tracking_in_a_narrow_repository() {
    let fixture = NarrowPublish::new("origin", true);
    assert_success(&fixture.push(&["--bootstrap"]));
    fixture.assert_tracked();
}

#[test]
fn narrow_publish_tracks_the_selected_non_origin_remote() {
    let fixture = NarrowPublish::new("fork", false);
    assert_success(&fixture.push(&[]));
    fixture.assert_tracked();
    assert!(
        !git_output(
            &fixture.worktree,
            &["config", "--get-all", "remote.origin.fetch"]
        )
        .status
        .success()
    );
    assert!(git(&fixture.worktree, &["for-each-ref", "refs/remotes/origin/"]).is_empty());
}

#[test]
fn fetch_failure_reports_publication_and_a_retry_reuses_the_mapping() {
    let fixture = NarrowPublish::new("origin", false);
    git(
        &fixture.worktree,
        &[
            "remote",
            "set-url",
            "--push",
            "origin",
            fixture.remote.path().to_str().unwrap(),
        ],
    );
    let broken = fixture.clone.path().join("missing-remote");
    git(
        &fixture.worktree,
        &["remote", "set-url", "origin", broken.to_str().unwrap()],
    );
    assert_tracking_failed(&fixture.push(&[]));
    fixture.assert_published();
    let mappings = fixture.fetch_mappings();
    git(
        &fixture.worktree,
        &[
            "remote",
            "set-url",
            "origin",
            fixture.remote.path().to_str().unwrap(),
        ],
    );
    assert_success(&fixture.push(&[]));
    fixture.assert_tracked();
    assert_eq!(
        fixture.fetch_mappings(),
        mappings,
        "recovery must not duplicate the mapping"
    );
}

#[test]
fn config_write_failure_reports_publication_and_can_be_retried() {
    let fixture = NarrowPublish::new("origin", false);
    // Preconfigure the intended upstream so push need not write branch config;
    // only the post-push set-branches operation encounters this fixture lock.
    git(
        &fixture.worktree,
        &["config", &format!("branch.{BRANCH}.remote"), "origin"],
    );
    git(
        &fixture.worktree,
        &[
            "config",
            &format!("branch.{BRANCH}.merge"),
            &format!("refs/heads/{BRANCH}"),
        ],
    );
    let config_lock = fixture.clone.path().join(".git/config.lock");
    std::fs::write(&config_lock, "fixture lock\n").unwrap();
    assert_tracking_failed(&fixture.push(&[]));
    fixture.assert_published();
    assert_eq!(fixture.fetch_mappings().lines().count(), 1);
    std::fs::remove_file(config_lock).unwrap();
    assert_success(&fixture.push(&[]));
    fixture.assert_tracked();
}
