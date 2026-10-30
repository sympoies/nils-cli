use nils_test_support::{
    StubBinDir,
    cmd::{CmdOptions, CmdOutput, run_with},
};
use pretty_assertions::assert_eq;
use serde_json::json;
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Command,
};

pub const CANARY: &str = "synthetic-replay-credential";
pub struct Fixture {
    pub root: tempfile::TempDir,
    pub repo: PathBuf,
    pub worktree: PathBuf,
    pub stubs: StubBinDir,
    pub fingerprint: String,
    pub upstream: String,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = Command::new("gpgconf")
            .env("GNUPGHOME", self.root.path().join("keys"))
            .args(["--kill", "gpg-agent"])
            .output();
    }
}
impl Fixture {
    pub fn options(&self, cwd: &Path) -> CmdOptions {
        let root = self.root.path();
        CmdOptions::new()
            .with_cwd(cwd)
            .with_path_prepend(self.stubs.path())
            .with_env("HOME", root.to_str().unwrap())
            .with_env("XDG_CONFIG_HOME", root.join("config").to_str().unwrap())
            .with_env("XDG_STATE_HOME", root.join("state").to_str().unwrap())
            .with_env("AGENT_HOME", root.join("agent").to_str().unwrap())
            .with_env("GNUPGHOME", root.join("keys").to_str().unwrap())
            .with_env_remove_prefix("GIT_")
            .with_env("GIT_CONFIG_NOSYSTEM", "1")
            .with_env(
                "GIT_CONFIG_GLOBAL",
                root.join(".gitconfig").to_str().unwrap(),
            )
            .with_env_remove_prefix("FORGE_IDENTITY_")
            .with_env("AGENT_SESSION_ID", "fixture-caller")
            .with_env("AGENT_SESSION_RUNTIME_ID", "fixture-runtime")
            .with_env("REPLAY_TEST_CREDENTIAL", CANARY)
    }
    pub fn git_at(&self, cwd: &Path, args: &[&str]) -> CmdOutput {
        run_with(Path::new("git"), args, &self.options(cwd))
    }
    pub fn git(&self, args: &[&str]) -> String {
        let out = self.git_at(&self.worktree, args);
        assert_eq!(out.code, 0, "{}", out.stderr_text());
        out.stdout_text().trim().to_owned()
    }
    pub fn replay(&self, args: &[&str]) -> CmdOutput {
        let mut all = vec!["rebase"];
        all.extend_from_slice(args);
        run_with(
            Path::new(env!("CARGO_BIN_EXE_semantic-commit")),
            &all,
            &self.options(&self.worktree),
        )
    }
    pub fn new(conflict: bool) -> Option<Self> {
        let git_cli = nils_test_support::bin::sibling_or_skip("git-cli", "nils-git-cli")?;
        let root = tempfile::tempdir().unwrap();
        let repo = root.path().join("repo");
        fs::create_dir(&repo).unwrap();
        let keys = root.path().join("keys");
        fs::create_dir(&keys).unwrap();
        fs::set_permissions(&keys, fs::Permissions::from_mode(0o700)).unwrap();
        let mut f = Self {
            root,
            repo: repo.clone(),
            worktree: repo,
            stubs: StubBinDir::new(),
            fingerprint: String::new(),
            upstream: String::new(),
        };
        let generated = run_with(
            Path::new("gpg"),
            &[
                "--batch",
                "--pinentry-mode",
                "loopback",
                "--passphrase",
                "",
                "--quick-generate-key",
                "Replay Agent <agent@example.invalid>",
                "ed25519",
                "sign",
                "0",
            ],
            &f.options(&f.repo),
        );
        assert_eq!(generated.code, 0, "{}", generated.stderr_text());
        let listed = run_with(
            Path::new("gpg"),
            &["--batch", "--with-colons", "--list-secret-keys"],
            &f.options(&f.repo),
        );
        assert_eq!(listed.code, 0);
        f.fingerprint = listed
            .stdout_text()
            .lines()
            .find_map(|line| {
                line.starts_with("fpr:")
                    .then(|| line.split(':').nth(9).unwrap().to_owned())
            })
            .unwrap();
        f.git(&["init", "-b", "main"]);
        f.git(&["config", "user.name", "Replay Agent"]);
        f.git(&["config", "user.email", "agent@example.invalid"]);
        fs::write(f.repo.join("base"), b"base\n").unwrap();
        f.git(&["add", "."]);
        f.git(&["commit", "-m", "test: base"]);
        f.git(&[
            "remote",
            "add",
            "origin",
            "https://github.com/sandbox/widget.git",
        ]);
        f.git(&["update-ref", "refs/remotes/origin/main", "HEAD"]);
        f.git(&[
            "symbolic-ref",
            "refs/remotes/origin/HEAD",
            "refs/remotes/origin/main",
        ]);
        let added = run_with(
            &git_cli,
            &[
                "worktree",
                "add",
                "replay-fixture",
                "--kind",
                "bug",
                "--from",
                "main",
                "--format",
                "json",
            ],
            &f.options(&f.repo),
        );
        assert_eq!(added.code, 0, "{}", added.stderr_text());
        let receipt: serde_json::Value = serde_json::from_slice(&added.stdout).unwrap();
        f.worktree = PathBuf::from(receipt["data"]["path"].as_str().unwrap());
        fs::write(
            f.worktree.join(if conflict { "base" } else { "topic" }),
            b"topic\n",
        )
        .unwrap();
        fs::write(f.worktree.join("binary"), b"topic\0bytes\xff\n").unwrap();
        f.git(&["add", "."]);
        f.git(&[
            "commit",
            "--author",
            "Original Author <author@example.invalid>",
            "-m",
            "test: topic",
        ]);
        fs::write(
            f.repo.join(if conflict { "base" } else { "upstream" }),
            b"upstream\n",
        )
        .unwrap();
        assert_eq!(f.git_at(&f.repo, &["add", "."]).code, 0);
        assert_eq!(
            f.git_at(&f.repo, &["commit", "-m", "test: upstream"]).code,
            0
        );
        f.upstream = f
            .git_at(&f.repo, &["rev-parse", "HEAD"])
            .stdout_text()
            .trim()
            .to_owned();
        fs::write(f.root.path().join(".gitconfig"), "[user]\n signingkey = 0000000000000000000000000000000000000000\n[commit]\n gpgsign = true\n[gpg]\n format = openpgp\n program = gpg\n").unwrap();
        let config = f.root.path().join("config/forge-cli");
        fs::create_dir_all(&config).unwrap();
        fs::write(
            config.join("identity.toml"),
            format!(
                r#"version=1
require_session_binding=true
[credentials.agent]
kind="env"
name="REPLAY_TEST_CREDENTIAL"
[profiles.agent]
expected_login="replay-agent"
credential="agent"
commit_name="Replay Agent"
commit_email="agent@example.invalid"
signing_fingerprint="{}"
operations=["commit"]
[principals.agent]
profiles=["agent"]
[[launch_rules]]
id="fixture-launch"
initiator="fixture"
principal="agent"
[[rules]]
id="fixture-repository"
principal="agent"
repo="github.com/sandbox/widget"
profile="agent"
"#,
                f.fingerprint
            ),
        )
        .unwrap();
        f.stubs.write_exe(
            "gh",
            "#!/bin/sh\nprintf '%s\\n' '{\"login\":\"replay-agent\"}'\n",
        );
        let binding = json!({"schema_version":"cli.agent-session.broker-identity.v1", "ok":true,
            "data":{"schema_version":"agent-session.forge-binding.v1", "session_id":"fixture-caller",
                "session_incarnation":"fixture-runtime", "session_created_at":"2026-01-01T00:00:00Z",
                "root":{"machine":"fixture", "session_id":"fixture-root", "session_created_at":"2026-01-01T00:00:00Z"},
                "parent":null, "initiator":"fixture", "role":null}});
        f.stubs.write_exe(
            "agent-session",
            &format!("#!/bin/sh\nprintf '%s\\n' '{binding}'\n"),
        );
        Some(f)
    }
    pub fn verify(&self, bytes: &[u8]) {
        self.git(&["verify-commit", "HEAD"]);
        assert_eq!(
            self.git(&["show", "-s", "--format=%G?|%GF|%cn|%ce|%an|%ae|%s"]),
            format!(
                "G|{}|Replay Agent|agent@example.invalid|Original Author|author@example.invalid|test: topic",
                self.fingerprint
            )
        );
        assert_eq!(self.git(&["rev-parse", "HEAD^"]), self.upstream);
        assert_eq!(
            fs::read(self.worktree.join("binary")).unwrap(),
            b"topic\0bytes\xff\n"
        );
        assert_eq!(
            fs::read(self.worktree.join(if self.worktree.join("topic").exists() {
                "topic"
            } else {
                "base"
            }))
            .unwrap(),
            bytes
        );
        let audit = fs::read_to_string(
            self.root
                .path()
                .join("state/forge-cli/identity-audit.jsonl"),
        )
        .unwrap();
        let head = self.git(&["rev-parse", "HEAD"]);
        assert!(audit.lines().any(|line| {
            let event: serde_json::Value = serde_json::from_str(line).unwrap();
            event["outcome"] == "execution_succeeded"
                && event["object"] == head
                && event["signer"] == self.fingerprint
        }));
        assert!(!audit.contains(CANARY));
    }
}
