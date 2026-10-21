use super::{
    LaunchBasis, invocation_path, launch_executable_from, resolve_agent_session_executable_from,
};
use pretty_assertions::assert_eq;
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::PathBuf;

#[test]
fn agent_session_executable_resolves_facade_to_exact_release_sibling() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let sibling_helper = tmp
        .path()
        .join(format!("fixture-helper{}", std::env::consts::EXE_SUFFIX));
    let agent_session = tmp
        .path()
        .join(format!("agent-session{}", std::env::consts::EXE_SUFFIX));
    fs::write(&sibling_helper, "fixture-helper").expect("fixture-helper fixture");
    fs::write(&agent_session, "agent-session").expect("agent-session fixture");
    fs::set_permissions(&sibling_helper, fs::Permissions::from_mode(0o700))
        .expect("fixture-helper mode");
    fs::set_permissions(&agent_session, fs::Permissions::from_mode(0o700))
        .expect("agent-session mode");

    // The sibling keeps the invocation's directory spelling; it is not resolved
    // (macOS tempdirs sit behind the `/var` link).
    assert_eq!(
        resolve_agent_session_executable_from(&sibling_helper).expect("sibling executable"),
        agent_session
    );
}

#[test]
fn agent_session_executable_preserves_direct_agent_session_recursion() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let agent_session = tmp
        .path()
        .join(format!("agent-session{}", std::env::consts::EXE_SUFFIX));
    fs::write(&agent_session, "agent-session").expect("agent-session fixture");
    fs::set_permissions(&agent_session, fs::Permissions::from_mode(0o700))
        .expect("agent-session mode");

    assert_eq!(
        resolve_agent_session_executable_from(&agent_session).expect("current executable"),
        agent_session
    );
}

#[test]
fn agent_session_executable_keeps_a_linked_directory_in_the_invocation_path() {
    // A directory reached through a link (macOS `/var` -> `/private/var`) is the
    // stable spelling: resolving it would pin the path the link stands in for.
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let real = tmp.path().join("real");
    fs::create_dir(&real).expect("real dir");
    let linked = tmp.path().join("linked");
    symlink(&real, &linked).expect("directory link");
    for stem in ["agent-session", "fixture-helper"] {
        let binary = real.join(format!("{stem}{}", std::env::consts::EXE_SUFFIX));
        fs::write(&binary, stem).expect("fixture");
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).expect("mode");
    }
    let name = |stem: &str| linked.join(format!("{stem}{}", std::env::consts::EXE_SUFFIX));

    assert_eq!(
        resolve_agent_session_executable_from(&name("agent-session")).expect("direct"),
        name("agent-session")
    );
    assert_eq!(
        resolve_agent_session_executable_from(&name("fixture-helper")).expect("facade"),
        name("agent-session")
    );
}

#[test]
fn agent_session_executable_resolves_a_linked_invocation_to_its_release() {
    // macOS reports the unresolved invocation path from current_exe, so a
    // Homebrew `bin/` link must resolve to the linked release directory.
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let release = tmp.path().join("Cellar/nils-cli/1.0.0/bin");
    let bin = tmp.path().join("bin");
    fs::create_dir_all(&release).expect("release dir");
    fs::create_dir_all(&bin).expect("bin dir");
    let name = |stem: &str| format!("{stem}{}", std::env::consts::EXE_SUFFIX);
    for stem in ["agent-session", "fixture-helper"] {
        let binary = release.join(name(stem));
        fs::write(&binary, stem).expect("release fixture");
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).expect("release mode");
        symlink(&binary, bin.join(name(stem))).expect("bin link");
    }
    let released = fs::canonicalize(release.join(name("agent-session"))).expect("release");

    // The invocation link is outside the versioned keg, so it is the stable
    // path to embed; the keg path would not survive an upgrade.
    assert_eq!(
        resolve_agent_session_executable_from(&bin.join(name("agent-session")))
            .expect("linked agent-session"),
        bin.join(name("agent-session"))
    );
    assert_eq!(
        resolve_agent_session_executable_from(&bin.join(name("fixture-helper")))
            .expect("linked facade"),
        bin.join(name("agent-session"))
    );

    // Resolving the invocation must not relax the sibling check: a linked
    // agent-session beside the real facade is still refused.
    let linked_sibling = tmp.path().join("linked-sibling");
    fs::create_dir_all(&linked_sibling).expect("linked sibling dir");
    let facade = linked_sibling.join(name("fixture-helper"));
    fs::write(&facade, "fixture-helper").expect("facade fixture");
    fs::set_permissions(&facade, fs::Permissions::from_mode(0o700)).expect("facade mode");
    symlink(&released, linked_sibling.join(name("agent-session"))).expect("sibling link");
    let refused =
        resolve_agent_session_executable_from(&facade).expect_err("linked sibling must be refused");
    assert_eq!(refused.kind(), io::ErrorKind::PermissionDenied);
}

/// A package-manager prefix: `Cellar/<pkg>/<version>/bin/agent-session` plus
/// whichever stable links the test asks for.
fn package_prefix(opt_link: bool, bin_link: bool) -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let prefix = fs::canonicalize(tmp.path()).expect("canonical prefix");
    let keg = prefix.join("Cellar/nils-cli/1.0.0");
    fs::create_dir_all(keg.join("bin")).expect("keg bin");
    let binary = keg.join("bin/agent-session");
    fs::write(&binary, "agent-session").expect("keg binary");
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o755)).expect("keg mode");
    if opt_link {
        fs::create_dir_all(prefix.join("opt")).expect("opt dir");
        symlink("../Cellar/nils-cli/1.0.0", prefix.join("opt/nils-cli")).expect("opt link");
    }
    if bin_link {
        fs::create_dir_all(prefix.join("bin")).expect("bin dir");
        symlink(
            "../Cellar/nils-cli/1.0.0/bin/agent-session",
            prefix.join("bin/agent-session"),
        )
        .expect("bin link");
    }
    (tmp, prefix)
}

#[test]
fn agent_session_executable_prefers_the_package_opt_link_over_the_versioned_keg() {
    // Linux reports the resolved keg path from current_exe, so the stable
    // link must come from the package layout, not from the invocation.
    let (_tmp, prefix) = package_prefix(true, true);
    let keg_binary = prefix.join("Cellar/nils-cli/1.0.0/bin/agent-session");

    assert_eq!(
        resolve_agent_session_executable_from(&keg_binary).expect("stable opt link"),
        prefix.join("opt/nils-cli/bin/agent-session")
    );
}

#[test]
fn agent_session_executable_falls_back_to_the_package_bin_link() {
    let (_tmp, prefix) = package_prefix(false, true);
    let keg_binary = prefix.join("Cellar/nils-cli/1.0.0/bin/agent-session");

    assert_eq!(
        resolve_agent_session_executable_from(&keg_binary).expect("stable bin link"),
        prefix.join("bin/agent-session")
    );
}

#[test]
fn agent_session_executable_keeps_the_versioned_keg_when_no_stable_link_exists() {
    let (_tmp, prefix) = package_prefix(false, false);
    let keg_binary = prefix.join("Cellar/nils-cli/1.0.0/bin/agent-session");

    assert_eq!(
        resolve_agent_session_executable_from(&keg_binary).expect("keg binary"),
        keg_binary
    );
}

#[test]
fn agent_session_executable_ignores_a_stable_link_to_another_release() {
    // A link left pointing at a different version is not this binary: the
    // launch must not silently adopt it.
    let (_tmp, prefix) = package_prefix(true, false);
    let other = prefix.join("Cellar/nils-cli/2.0.0/bin");
    fs::create_dir_all(&other).expect("other keg");
    fs::write(other.join("agent-session"), "agent-session").expect("other binary");
    fs::set_permissions(
        other.join("agent-session"),
        fs::Permissions::from_mode(0o755),
    )
    .expect("other mode");
    fs::remove_file(prefix.join("opt/nils-cli")).expect("drop opt link");
    symlink("../Cellar/nils-cli/2.0.0", prefix.join("opt/nils-cli")).expect("repoint opt link");
    let keg_binary = prefix.join("Cellar/nils-cli/1.0.0/bin/agent-session");

    assert_eq!(
        resolve_agent_session_executable_from(&keg_binary).expect("keg binary"),
        keg_binary
    );
}

#[test]
fn agent_session_executable_fails_closed_for_missing_or_non_executable_sibling() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let sibling_helper = tmp
        .path()
        .join(format!("fixture-helper{}", std::env::consts::EXE_SUFFIX));
    let agent_session = tmp
        .path()
        .join(format!("agent-session{}", std::env::consts::EXE_SUFFIX));
    fs::write(&sibling_helper, "fixture-helper").expect("fixture-helper fixture");
    fs::set_permissions(&sibling_helper, fs::Permissions::from_mode(0o700))
        .expect("fixture-helper mode");

    let missing = resolve_agent_session_executable_from(&sibling_helper)
        .expect_err("missing sibling must fail");
    assert_eq!(missing.kind(), io::ErrorKind::NotFound);

    fs::write(&agent_session, "agent-session").expect("agent-session fixture");
    fs::set_permissions(&agent_session, fs::Permissions::from_mode(0o600))
        .expect("agent-session mode");
    let non_executable = resolve_agent_session_executable_from(&sibling_helper)
        .expect_err("non-executable sibling must fail");
    assert_eq!(non_executable.kind(), io::ErrorKind::PermissionDenied);
}

#[test]
fn launch_basis_names_the_rule_that_chose_the_path() {
    let (_tmp, prefix) = package_prefix(true, true);
    let keg_binary = prefix.join("Cellar/nils-cli/1.0.0/bin/agent-session");
    let launch = launch_executable_from(&keg_binary, &[]).expect("opt launch");
    assert_eq!(launch.basis, LaunchBasis::PackageOpt);

    let (_tmp, prefix) = package_prefix(false, true);
    let keg_binary = prefix.join("Cellar/nils-cli/1.0.0/bin/agent-session");
    let launch = launch_executable_from(&keg_binary, &[]).expect("bin launch");
    assert_eq!(launch.basis, LaunchBasis::PackageBin);

    let (_tmp, prefix) = package_prefix(false, false);
    let keg_binary = prefix.join("Cellar/nils-cli/1.0.0/bin/agent-session");
    let launch = launch_executable_from(&keg_binary, &[]).expect("keg launch");
    assert_eq!(launch.basis, LaunchBasis::VersionedKeg);
}

#[test]
fn an_invocation_hint_outside_the_keg_wins_on_a_resolved_current_exe() {
    // Linux reports the keg path from current_exe; argv[0] resolved through
    // PATH is the only place the stable invocation survives.
    let (_tmp, prefix) = package_prefix(true, true);
    let keg_binary = prefix.join("Cellar/nils-cli/1.0.0/bin/agent-session");
    let hint = prefix.join("bin/agent-session");

    let launch = launch_executable_from(&keg_binary, std::slice::from_ref(&hint)).expect("hint");

    assert_eq!(launch.path, hint);
    assert_eq!(launch.basis, LaunchBasis::Invocation);
}

#[test]
fn an_invocation_hint_for_another_binary_is_not_adopted() {
    let (_tmp, prefix) = package_prefix(false, false);
    let keg_binary = prefix.join("Cellar/nils-cli/1.0.0/bin/agent-session");
    let other = tempfile::TempDir::new().expect("other dir");
    let stranger = other.path().join("agent-session");
    fs::write(&stranger, "agent-session").expect("stranger binary");
    fs::set_permissions(&stranger, fs::Permissions::from_mode(0o755)).expect("stranger mode");

    let launch = launch_executable_from(&keg_binary, &[stranger]).expect("keg launch");

    assert_eq!(launch.path, keg_binary);
    assert_eq!(launch.basis, LaunchBasis::VersionedKeg);
}

#[test]
fn argv0_resolves_through_path_or_the_current_directory() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let dir = fs::canonicalize(tmp.path()).expect("canonical dir");
    let binary = dir.join("agent-session");
    fs::write(&binary, "agent-session").expect("binary");
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o755)).expect("mode");
    let path_env = std::env::join_paths([PathBuf::from("/nonexistent-agent-session"), dir.clone()])
        .expect("path env");

    assert_eq!(
        invocation_path(OsStr::new("agent-session"), Some(&path_env)),
        Some(binary.clone())
    );
    assert_eq!(
        invocation_path(binary.as_os_str(), None),
        Some(binary.clone())
    );
    assert_eq!(invocation_path(OsStr::new("agent-session"), None), None);
    assert_eq!(
        invocation_path(OsStr::new("missing-binary"), Some(&path_env)),
        None
    );
}

#[test]
fn the_held_launch_wrapper_stops_the_broker_with_the_upgraded_binary() {
    // Launch from release 1.0.0, upgrade while the provider runs (the old keg
    // is deleted and the stable link repointed), then let the wrapper reach
    // its runtime-exit `broker stop`.
    let (tmp, prefix) = package_prefix(true, false);
    let write_release = |version: &str| {
        let bin = prefix.join(format!("Cellar/nils-cli/{version}/bin"));
        fs::create_dir_all(&bin).expect("release bin");
        let binary = bin.join("agent-session");
        fs::write(
            &binary,
            format!(
                "#!/bin/sh\ncase \"$*\" in *'broker stop'*) echo {version} >> '{}';; esac\n",
                prefix.join("stops.log").display()
            ),
        )
        .expect("release binary");
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o755)).expect("release mode");
        binary
    };
    let old = write_release("1.0.0");
    write_release("2.0.0");
    let launch = launch_executable_from(&old, &[]).expect("launch executable");
    assert_eq!(launch.basis, LaunchBasis::PackageOpt);

    let state = tmp.path().join("state");
    fs::create_dir_all(&state).expect("state dir");
    let (gate, broker_gate) = (state.join("gate"), state.join("broker-gate"));
    fs::write(&gate, "").expect("gate");
    fs::write(&broker_gate, "").expect("broker gate");
    let upgrade = format!(
        "rm -rf '{keg}' && ln -sfn ../Cellar/nils-cli/2.0.0 '{opt}'",
        keg = prefix.join("Cellar/nils-cli/1.0.0").display(),
        opt = prefix.join("opt/nils-cli").display()
    );
    let status = std::process::Command::new("sh")
        .arg("-c")
        .arg(crate::HELD_LAUNCH_SCRIPT)
        .arg("agent-session-held-launch")
        .arg(&gate)
        .arg(&broker_gate)
        .arg(state.join("heartbeat"))
        .arg(state.join("capability"))
        .arg("incarnation")
        .arg("1")
        .arg(&launch.path)
        .args(["sh", "-c", &upgrade])
        .env("AGENT_SESSION_STATE_DIR", &state)
        .env("AGENT_SESSION_ID", "fixture")
        .status()
        .expect("held launch");

    assert!(status.success());
    assert_eq!(
        fs::read_to_string(prefix.join("stops.log")).expect("stop log"),
        "2.0.0\n"
    );
}
