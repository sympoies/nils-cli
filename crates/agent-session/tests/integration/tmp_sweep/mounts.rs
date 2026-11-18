use super::*;
use pretty_assertions::assert_eq;
use std::process::Command;

fn namespace() -> Command {
    let mut command = Command::new("unshare");
    command
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("LC_ALL", "C")
        .args([
            "--user",
            "--map-root-user",
            "--mount",
            "--propagation",
            "private",
            "--",
        ]);
    command
}

#[test]
fn sweep_preserves_a_same_device_bind_mount_and_its_external_contents() {
    let probe = namespace().arg("true").output();
    match probe {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("skipped: unshare is unavailable");
            return;
        }
        Ok(output)
            if !output.status.success()
                && String::from_utf8_lossy(&output.stderr).contains("Operation not permitted") =>
        {
            eprintln!("skipped: private user/mount namespaces are unavailable");
            return;
        }
        other => assert!(other.unwrap().status.success(), "namespace probe failed"),
    }

    let fixture = tempfile::TempDir::new().unwrap();
    let home = fixture.path().join("home");
    let state = fixture.path().join("state");
    let external = fixture.path().join("external-volume");
    fs::create_dir_all(&external).unwrap();
    fs::write(external.join("keep"), "external canary").unwrap();
    owned_dir(&home, &state, "expired");
    let candidate = home.join("tmp/expired");
    let mountpoint = candidate.join("mounted");
    fs::create_dir(&mountpoint).unwrap();
    // Same device on both sides: st_dev alone cannot recognize this boundary.
    use std::os::unix::fs::MetadataExt;
    assert_eq!(
        fs::metadata(&external).unwrap().dev(),
        fs::metadata(&mountpoint).unwrap().dev()
    );
    // The mapped namespace identity need not traverse the build user's home.
    let binary = fixture.path().join("agent-session");
    fs::copy(nils_test_support::bin::resolve("agent-session"), &binary).unwrap();
    let output = namespace()
        .env("AGENT_HOME", &home)
        .args([
            "sh",
            "-eu",
            "-c",
            "mount --bind \"$1\" \"$2\"; exec \"$3\" --state-dir \"$4\" tmp sweep --grace-secs 0 --format json",
            "fixture",
        ])
        .arg(&external)
        .arg(&mountpoint)
        .arg(binary)
        .arg(&state)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "fixture sweep failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        fs::read_to_string(external.join("keep")).ok().as_deref(),
        Some("external canary"),
        "sweep crossed the mount boundary"
    );
    assert!(candidate.join(OWNER_FILE).is_file());
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["data"]["removed"], 0);
    assert_eq!(report["data"]["retained"]["untrusted"], 1);
}
