//! Worktree build class: `git-cli worktree` records a deterministic build
//! directory under `AGENT_HOME/build/<repo-key>/<slug>` and reclaims it with the
//! worktree (agent write-location contract, `build` class).

use crate::common;
use common::{GitCliHarness, git, init_bare_remote, init_repo};
use nils_test_support::cmd::{CmdOptions, CmdOutput, run_with};
use pretty_assertions::assert_eq;
use serde_json::Value;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

struct Fixture {
    harness: GitCliHarness,
    repo: tempfile::TempDir,
    _remote: tempfile::TempDir,
    home: tempfile::TempDir,
    probes: nils_test_support::StubBinDir,
}

impl Fixture {
    fn new() -> Self {
        let harness = GitCliHarness::new();
        let repo = init_repo();
        let remote = init_bare_remote();
        git(
            repo.path(),
            &["remote", "add", "origin", remote.path().to_str().unwrap()],
        );
        git(repo.path(), &["push", "-u", "origin", "main"]);
        git(remote.path(), &["symbolic-ref", "HEAD", "refs/heads/main"]);
        git(repo.path(), &["remote", "set-head", "origin", "main"]);
        let home = tempfile::TempDir::new().unwrap();
        for directory in [
            home.path().join("lease-state"),
            home.path().join("sessions"),
            home.path().join("sessions/coordination"),
            home.path().join("lease-state/coordination"),
        ] {
            fs::create_dir_all(&directory).unwrap();
            fs::set_permissions(directory, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let probes = nils_test_support::StubBinDir::new();
        probes.write_exe("lsof", "#!/bin/sh\nexit 1\n");
        probes.write_exe(
            "agent-session",
            "#!/bin/sh\nprintf '%s\\n' '{\"ok\":true,\"data\":[]}'\n",
        );
        probes.write_exe(
            "forge-cli",
            "#!/bin/sh\nprintf '%s\\n' '{\"ok\":true,\"data\":{\"items\":[]}}'\n",
        );
        Self {
            harness,
            repo,
            _remote: remote,
            home,
            probes,
        }
    }

    fn options(&self) -> CmdOptions {
        let state = |name: &str| {
            fs::canonicalize(self.home.path().join(name))
                .unwrap()
                .to_str()
                .unwrap()
                .to_string()
        };
        self.harness
            .cmd_options(self.repo.path())
            .with_path_prepend(self.probes.path())
            .with_env("AGENT_HOME", self.home.path().to_str().unwrap())
            .with_env(
                "AGENT_RUNTIME_CHECKOUT_LEASE_STATE_HOME",
                &state("lease-state"),
            )
            .with_env("AGENT_SESSION_STATE_DIR", &state("sessions"))
            .with_env("AGENT_SESSION_COORDINATION_MODE", "advisory")
            .with_env("AGENT_SESSION_ID", "fixture-caller")
    }

    fn run(&self, args: &[&str]) -> CmdOutput {
        run_with(&self.harness.git_cli_bin(), args, &self.options())
    }

    fn add(&self, slug: &str) -> Value {
        let output = self.run(&[
            "worktree", "add", slug, "--from", "main", "--format", "json",
        ]);
        assert_eq!(output.code, 0, "stderr: {}", output.stderr_text());
        parse_json(&output)["data"].clone()
    }

    fn build_root(&self) -> PathBuf {
        fs::canonicalize(self.home.path()).unwrap().join("build")
    }
}

fn parse_json(output: &CmdOutput) -> Value {
    serde_json::from_str(output.stdout_text().trim()).expect("valid json output")
}

fn expected_build_dir(fixture: &Fixture, data: &Value, slug: &str) -> PathBuf {
    fixture
        .build_root()
        .join(data["repo_key"].as_str().expect("repo key"))
        .join(slug)
}

fn export_line(dir: &Path) -> String {
    format!("export CARGO_TARGET_DIR='{}'", dir.display())
}

#[test]
fn add_records_the_build_dir_and_export_command() {
    let fixture = Fixture::new();
    let data = fixture.add("topic-build");
    let dir = expected_build_dir(&fixture, &data, "topic-build");

    assert_eq!(data["build_dir"].as_str(), Some(&*dir.to_string_lossy()));
    assert_eq!(
        data["build_env"].as_str(),
        Some(export_line(&dir).as_str()),
        "add must record a ready-to-eval CARGO_TARGET_DIR export"
    );
    assert!(
        !dir.starts_with(data["path"].as_str().unwrap()),
        "build dir must live outside the worktree"
    );

    let text = fixture.run(&["worktree", "add", "topic-text", "--from", "main"]);
    assert_eq!(text.code, 0, "stderr: {}", text.stderr_text());
    assert!(
        text.stdout_text().contains("Build: "),
        "stdout: {}",
        text.stdout_text()
    );
}

#[test]
fn go_build_prints_the_build_dir_and_export_command() {
    let fixture = Fixture::new();
    let data = fixture.add("topic-go-build");
    let dir = expected_build_dir(&fixture, &data, "topic-go-build");

    let plain = fixture.run(&["worktree", "go", "topic-go-build", "--build"]);
    assert_eq!(plain.code, 0, "stderr: {}", plain.stderr_text());
    assert_eq!(plain.stdout_text().trim(), dir.to_string_lossy());

    let shell = fixture.run(&["worktree", "go", "topic-go-build", "--build", "--shell"]);
    assert_eq!(shell.code, 0, "stderr: {}", shell.stderr_text());
    assert_eq!(shell.stdout_text().trim(), export_line(&dir));

    let json = fixture.run(&["worktree", "go", "topic-go-build", "--format", "json"]);
    assert_eq!(json.code, 0, "stderr: {}", json.stderr_text());
    assert_eq!(
        parse_json(&json)["data"]["build_dir"].as_str(),
        Some(&*dir.to_string_lossy())
    );
}

#[test]
fn go_build_for_an_unmanaged_worktree_is_a_data_error() {
    let fixture = Fixture::new();
    let outside = tempfile::TempDir::new().unwrap();
    let external = outside.path().join("external");
    git(
        fixture.repo.path(),
        &[
            "worktree",
            "add",
            "-b",
            "external",
            external.to_str().unwrap(),
            "main",
        ],
    );

    let output = fixture.run(&["worktree", "go", external.to_str().unwrap(), "--build"]);
    assert_eq!(output.code, 65, "stdout: {}", output.stdout_text());
}

#[test]
fn remove_reclaims_the_build_dir_with_the_worktree() {
    let fixture = Fixture::new();
    let data = fixture.add("topic-reclaim");
    let sibling_data = fixture.add("topic-sibling");
    let dir = expected_build_dir(&fixture, &data, "topic-reclaim");
    let sibling = expected_build_dir(&fixture, &sibling_data, "topic-sibling");
    for build in [&dir, &sibling] {
        fs::create_dir_all(build.join("debug")).unwrap();
        fs::write(build.join("debug/artifact"), "bytes").unwrap();
    }

    let removed = fixture.run(&[
        "worktree",
        "remove",
        "topic-reclaim",
        "--safe",
        "--format",
        "json",
    ]);
    assert_eq!(removed.code, 0, "stdout: {}", removed.stdout_text());
    let receipt = parse_json(&removed);
    assert_eq!(
        receipt["data"]["build_dir"].as_str(),
        Some(&*dir.to_string_lossy())
    );
    assert_eq!(receipt["data"]["build_dir_removed"], true);
    assert!(
        !dir.exists(),
        "build dir must be reclaimed with the worktree"
    );
    assert!(sibling.join("debug/artifact").exists());
}

#[test]
fn remove_without_a_build_dir_reports_nothing_reclaimed() {
    let fixture = Fixture::new();
    fixture.add("topic-nobuild");

    let removed = fixture.run(&[
        "worktree",
        "remove",
        "topic-nobuild",
        "--safe",
        "--format",
        "json",
    ]);
    assert_eq!(removed.code, 0, "stdout: {}", removed.stdout_text());
    assert_eq!(parse_json(&removed)["data"]["build_dir_removed"], false);
}

#[test]
fn remove_unlinks_a_symlinked_build_dir_without_following_it() {
    let fixture = Fixture::new();
    let data = fixture.add("topic-link");
    let dir = expected_build_dir(&fixture, &data, "topic-link");
    let elsewhere = tempfile::TempDir::new().unwrap();
    fs::write(elsewhere.path().join("keep"), "keep").unwrap();
    fs::create_dir_all(dir.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(elsewhere.path(), &dir).unwrap();

    let removed = fixture.run(&[
        "worktree",
        "remove",
        "topic-link",
        "--safe",
        "--format",
        "json",
    ]);
    assert_eq!(removed.code, 0, "stdout: {}", removed.stdout_text());
    assert_eq!(parse_json(&removed)["data"]["build_dir_removed"], true);
    assert!(fs::symlink_metadata(&dir).is_err(), "link must be removed");
    assert!(
        elsewhere.path().join("keep").exists(),
        "link target must survive"
    );
}
