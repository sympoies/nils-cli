use std::path::{Path, PathBuf};
use std::process::{Command, Output};

struct Fixture {
    root: tempfile::TempDir,
    home: PathBuf,
    tmp: PathBuf,
    sysroot: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("home");
        let tmp = root.path().join("tmp");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&tmp).unwrap();
        let sysroot = Command::new("rustc")
            .args(["--print", "sysroot"])
            .output()
            .unwrap();
        assert!(sysroot.status.success());
        Self {
            root,
            home,
            tmp,
            sysroot: PathBuf::from(String::from_utf8(sysroot.stdout).unwrap().trim()),
        }
    }

    fn command(&self, program: impl AsRef<std::ffi::OsStr>, cwd: &Path) -> Command {
        let mut command = Command::new(program);
        command
            .env_clear()
            .current_dir(cwd)
            .env("PATH", std::env::var_os("PATH").unwrap())
            .env("HOME", &self.home)
            .env("TMPDIR", &self.tmp)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("XDG_CONFIG_HOME", self.home.join("config"))
            .env("XDG_STATE_HOME", self.home.join("state"))
            .env("CARGO_HOME", self.home.join("cargo"))
            .env("CARGO_TARGET_DIR", self.root.path().join("build"))
            .env("CARGO_BUILD_JOBS", "1")
            .env("RUSTC", self.sysroot.join("bin/rustc"));
        if self
            .sysroot
            .parent()
            .and_then(Path::file_name)
            .is_some_and(|name| name == "toolchains")
        {
            command.env(
                "RUSTUP_HOME",
                self.sysroot.parent().unwrap().parent().unwrap(),
            );
        }
        command
    }

    fn git(&self, cwd: &Path, args: &[&str]) {
        let output = self.command("git", cwd).args(args).output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn check(&self, cwd: &Path) -> String {
        let Output {
            status,
            stdout,
            stderr,
        } = self
            .command(env!("CARGO"), cwd)
            .args(["check", "--locked", "--offline", "-vv"])
            .output()
            .unwrap();
        assert!(status.success(), "{}", String::from_utf8_lossy(&stderr));
        format!(
            "{}{}",
            String::from_utf8_lossy(&stdout),
            String::from_utf8_lossy(&stderr)
        )
    }
}

#[test]
fn linked_worktree_reuses_artifacts_and_observes_head_and_packed_refs() {
    let fixture = Fixture::new();
    let repo = fixture.root.path().join("repo");
    let linked = fixture.root.path().join("linked");
    std::fs::create_dir_all(repo.join("src")).unwrap();
    std::fs::write(
        repo.join("Cargo.toml"),
        "[package]\nname='build-info-fixture'\nversion='0.0.0'\nedition='2024'\n",
    )
    .unwrap();
    std::fs::write(
        repo.join("Cargo.lock"),
        "version = 4\n[[package]]\nname = 'build-info-fixture'\nversion = '0.0.0'\n",
    )
    .unwrap();
    std::fs::write(repo.join("build.rs"), include_str!("../build.rs")).unwrap();
    std::fs::write(
        repo.join("src/lib.rs"),
        "pub const VERSION: &str = env!(\"NILS_GIT_DESCRIBE\");\n",
    )
    .unwrap();
    fixture.git(&repo, &["init", "-q"]);
    fixture.git(&repo, &["config", "user.name", "Fixture"]);
    fixture.git(&repo, &["config", "user.email", "fixture@example.invalid"]);
    fixture.git(&repo, &["config", "commit.gpgsign", "false"]);
    fixture.git(&repo, &["add", "."]);
    fixture.git(&repo, &["commit", "-qm", "initial"]);
    fixture.git(
        &repo,
        &[
            "worktree",
            "add",
            "-b",
            "fixture-branch",
            "-q",
            linked.to_str().unwrap(),
        ],
    );

    fixture.check(&linked);
    let fresh = fixture.check(&linked);
    assert!(
        !fresh.contains("Dirty build-info-fixture"),
        "no-op build rewrote artifacts: {fresh}"
    );
    assert!(fresh.contains("Fresh build-info-fixture"), "{fresh}");

    fixture.git(&repo, &["branch", "unrelated-fixture-branch"]);
    let unrelated = fixture.check(&linked);
    assert!(
        !unrelated.contains("Dirty build-info-fixture"),
        "another worktree's branch must not invalidate this metadata: {unrelated}"
    );

    fixture.git(&linked, &["commit", "--allow-empty", "-qm", "advance"]);
    let advanced = fixture.check(&linked);
    assert!(
        advanced.contains("Dirty build-info-fixture"),
        "HEAD changes must invalidate metadata: {advanced}"
    );
    fixture.git(&linked, &["tag", "fixture-tag"]);
    fixture.git(&linked, &["pack-refs", "--all"]);
    let packed = fixture.check(&linked);
    assert!(
        packed.contains("NILS_GIT_DESCRIBE=fixture-tag"),
        "packed tag must update metadata: {packed}"
    );
    assert!(!fixture.check(&linked).contains("Dirty build-info-fixture"));
}
