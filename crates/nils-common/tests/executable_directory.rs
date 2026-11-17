//! Directory trust at the Rust and generated-shell selection boundaries.
use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::Path;
use std::process::Command;

use nils_common::{provider_entrypoint, session_binary};
use pretty_assertions::assert_eq;

fn executable(path: &Path) {
    fs::write(path, "#!/bin/sh\nprintf selected\n").unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

fn selected(root: &Path, sibling: bool) -> std::process::Output {
    let args = provider_entrypoint::argv(true, &[]);
    let mut command = Command::new(&args[0]);
    command
        .args(&args[1..])
        .env("AGENT_RUNTIME_TRUSTED_CLI_ROOT", root);
    if sibling {
        command.env("AGENT_SESSION_BIN", root.join(session_binary::NAME));
    } else {
        command.env_remove("AGENT_SESSION_BIN");
    }
    command.output().unwrap()
}

fn assert_selection(root: &Path, accepted: bool) {
    let mut observed = vec![
        ("trusted root", session_binary::trusted_root(root).is_some()),
        (
            "sibling preflight",
            session_binary::is_owner_controlled_executable(&root.join("agent-hook")),
        ),
    ];
    for sibling in [false, true] {
        let output = selected(root, sibling);
        observed.push((
            if sibling {
                "selector sibling"
            } else {
                "selector roots"
            },
            output.status.code() == Some(0),
        ));
        assert_eq!(output.status.code(), Some(if accepted { 0 } else { 127 }));
        assert_eq!(
            output.stdout,
            if accepted {
                b"selected".to_vec()
            } else {
                vec![]
            }
        );
    }
    let expected: Vec<_> = observed.iter().map(|(name, _)| (*name, accepted)).collect();
    assert_eq!(observed, expected);
}

fn installed(root: &Path) {
    executable(&root.join(session_binary::NAME));
    executable(&root.join("agent-hook"));
}

#[test]
fn writable_containing_directories_are_rejected_on_every_selection_path() {
    let temp = tempfile::tempdir().unwrap();
    installed(temp.path());
    for mode in [0o775, 0o757] {
        fs::set_permissions(temp.path(), fs::Permissions::from_mode(mode)).unwrap();
        assert_selection(temp.path(), false);
    }
}

#[test]
fn a_writable_symlink_directory_resolving_to_a_secure_release_is_accepted() {
    let temp = tempfile::tempdir().unwrap();
    let bin = temp.path().join("bin");
    let release = temp.path().join("Cellar/tool/1.0/bin");
    fs::create_dir(&bin).unwrap();
    fs::create_dir_all(&release).unwrap();
    fs::set_permissions(&bin, fs::Permissions::from_mode(0o775)).unwrap();
    fs::set_permissions(&release, fs::Permissions::from_mode(0o755)).unwrap();
    installed(&release);
    for name in [session_binary::NAME, "agent-hook"] {
        symlink(release.join(name), bin.join(name)).unwrap();
    }
    assert_selection(&bin, true);
    assert_selection(&release, true);
}

#[test]
fn rejected_directories_continue_to_the_next_trusted_root() {
    let temp = tempfile::tempdir().unwrap();
    let bad = temp.path().join("bad");
    let good = temp.path().join("good");
    for directory in [&bad, &good] {
        fs::create_dir(directory).unwrap();
        installed(directory);
    }
    fs::set_permissions(&bad, fs::Permissions::from_mode(0o775)).unwrap();
    assert_eq!(
        [&bad, &good]
            .into_iter()
            .find_map(|root| session_binary::trusted_root(root)),
        Some(fs::canonicalize(good.join(session_binary::NAME)).unwrap())
    );
    fs::write(bad.join("agent-hook"), "#!/bin/sh\nprintf rejected\n").unwrap();
    let args = provider_entrypoint::argv(true, &[]);
    let output = Command::new(&args[0])
        .args(&args[1..])
        .env_remove("AGENT_SESSION_BIN")
        .env(
            "AGENT_RUNTIME_TRUSTED_CLI_ROOT",
            std::env::join_paths([bad, good]).unwrap(),
        )
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stdout, b"selected");
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "requires Linux unprivileged user namespaces and subordinate UID mappings"]
fn foreign_owned_containing_directories_are_rejected_on_every_selection_path() {
    const CHILD: &str = "NILS_DIRECTORY_OWNER_TEST_CHILD";
    if std::env::var_os(CHILD).is_none() {
        // Linux user namespaces map the caller to root and a subordinate UID
        // to 1, allowing a real foreign-owner fixture without host privileges.
        let available = Command::new("unshare")
            .args(["--user", "--map-auto", "--map-root-user", "true"])
            .output();
        assert!(
            available.is_ok_and(|output| output.status.success()),
            "explicitly enabled foreign-owner fixture requires Linux subordinate UID mappings"
        );
        let output = Command::new("unshare")
            .args(["--user", "--map-auto", "--map-root-user"])
            .arg(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "foreign_owned_containing_directories_are_rejected_on_every_selection_path",
                "--nocapture",
                "--ignored",
            ])
            .env(CHILD, "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("foreign");
    fs::create_dir(&directory).unwrap();
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o755)).unwrap();
    installed(&directory);
    std::os::unix::fs::chown(&directory, Some(1), None).unwrap();
    assert_selection(&directory, false);
}
