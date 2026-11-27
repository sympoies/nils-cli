#![cfg(unix)]

use std::env;
use std::fs;
use std::os::unix::net::UnixListener;
use std::process::Command;

use nils_test_support::fixture::Fixture;
use nils_test_support::tempdir::ScopedTempDir;
use pretty_assertions::{assert_eq, assert_ne};

const PROBE: &str = "NILS_FIXTURE_ISOLATION_PROBE";

#[test]
fn fixture_isolation_probe() {
    if env::var_os(PROBE).is_none() {
        return;
    }

    let fixture = Fixture::new();
    let temp_dir = fixture.temp_dir().to_owned();
    let runtime_dir = fixture.runtime_dir().to_owned();
    let host_temp = env::temp_dir().canonicalize().expect("resolve parent temp");
    assert!(temp_dir.starts_with(&host_temp));
    assert_ne!(temp_dir, host_temp);
    assert_ne!(temp_dir, runtime_dir);

    let socket = fixture.runtime_dir().join("probe.sock");
    let listener = UnixListener::bind(&socket).expect("bind fixture socket");
    let status = fixture
        .command("sh")
        .args([
            "-ec",
            r#"test "$TMPDIR" = "$1" && test "$XDG_RUNTIME_DIR" = "$2" && touch child"#,
            "sh",
        ])
        .arg(&temp_dir)
        .arg(&runtime_dir)
        .status()
        .expect("run isolated child");
    assert!(status.success());
    assert!(temp_dir.join("child").is_file());

    for key in ["commit.gpgSign", "tag.gpgSign"] {
        let output = fixture
            .git_command(&temp_dir)
            .args(["config", "--global", "--get", key])
            .output()
            .expect("read fixture Git config");
        assert!(output.status.success(), "{output:?}");
        assert_eq!(output.stdout, b"false\n");
        let output = fixture
            .git_command(&temp_dir)
            .args(["config", "--get", key])
            .output()
            .expect("read effective fixture Git config");
        assert!(output.status.success(), "{output:?}");
        assert_eq!(output.stdout, b"false\n");
    }

    drop(listener);
    drop(fixture);
    assert!(!temp_dir.exists());
    assert!(!runtime_dir.exists());
}

#[test]
fn fixture_isolates_children_from_host_roots_and_git_config() {
    let ambient = ScopedTempDir::new();
    let tmpdir = ambient.path().join("long".repeat(32));
    fs::create_dir(&tmpdir).expect("create long temp root");
    let git_config = ambient.path().join("gitconfig");
    fs::write(
        &git_config,
        "[commit]\n\tgpgSign = true\n[tag]\n\tgpgSign = true\n",
    )
    .expect("write synthetic ambient Git config");
    let output = Command::new(env::current_exe().expect("current test executable"))
        .args(["--exact", "fixture::fixture_isolation_probe"])
        .env(PROBE, "1")
        .env("TMPDIR", &tmpdir)
        .env("XDG_RUNTIME_DIR", &tmpdir)
        .env("GIT_CONFIG_GLOBAL", &git_config)
        .env("GIT_CONFIG_COUNT", "1")
        .env("GIT_CONFIG_KEY_0", "commit.gpgSign")
        .env("GIT_CONFIG_VALUE_0", "true")
        .env("GIT_CONFIG_PARAMETERS", "'tag.gpgSign'='true'")
        .current_dir(ambient.path())
        .output()
        .expect("spawn fixture isolation probe");
    assert!(
        output.status.success() && String::from_utf8_lossy(&output.stdout).contains("1 passed"),
        "{output:?}"
    );
}
