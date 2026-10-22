use super::*;
use pretty_assertions::assert_eq;
use std::os::unix::process::ExitStatusExt;
use std::process::ExitStatus;

#[test]
fn selected_mapping_with_an_outside_hardlink_name_retains_target() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("target");
    std::fs::create_dir(&target).unwrap();
    let inside = target.join("shared");
    std::fs::write(&inside, "shared").unwrap();
    let outside = root.path().join("outside");
    std::fs::hard_link(&inside, &outside).unwrap();
    for descriptor in ["cwd", "3", "txt", "mem"] {
        for path in [&inside, &outside] {
            // Both names identify the same vnode selected by +D on Darwin.
            let bytes = format!("p42\0cworker\0\nf{descriptor}\0n{}\0\n", path.display());
            let error = check(bytes.as_bytes(), &[]).unwrap_err();
            assert_eq!(error.code, "removal-process-active");
            assert_eq!(error.details.unwrap()["pid"], 42);
            assert!(check(bytes.as_bytes(), &[42]).is_ok());
        }
    }
}

#[test]
fn incomplete_inventory_never_proves_idle() {
    for (code, stdout, stderr) in [
        (0, b"".as_slice(), b"".as_slice()),
        (1, b"".as_slice(), b"incomplete".as_slice()),
        (2, b"".as_slice(), b"".as_slice()),
        (1, b"malformed".as_slice(), b"".as_slice()),
        (0, b"pbad\0\nfmem\0".as_slice(), b"".as_slice()),
        (0, b"fmem\0".as_slice(), b"".as_slice()),
        (0, b"p42\0cworker\0\n".as_slice(), b"".as_slice()),
        (0, b"p42\0cworker\0\nfmem".as_slice(), b"".as_slice()),
    ] {
        let output = Output {
            status: ExitStatus::from_raw(code << 8),
            stdout: stdout.to_vec(),
            stderr: stderr.to_vec(),
        };
        assert_eq!(
            check_output(&output, &[]).unwrap_err().code,
            "removal-proof-unavailable"
        );
    }
    let output = Output {
        status: ExitStatus::from_raw(1 << 8),
        stdout: Vec::new(),
        stderr: Vec::new(),
    };
    assert!(check_output(&output, &[]).is_ok());
    let active = Output {
        stdout: b"p42\0cworker\0\nfmem\0noutside\0\n".to_vec(),
        stderr: b"incomplete".to_vec(),
        ..output
    };
    assert_eq!(
        check_output(&active, &[]).unwrap_err().code,
        "removal-process-active"
    );
}

#[test]
fn selected_probe_cannot_run_without_target_parent() {
    // Exercise probe argument validation on Linux too, where production uses
    // procfs. Invalid selection cannot fall back to an unfiltered inventory.
    assert_eq!(
        processes_idle(Path::new("/"), &[]).unwrap_err().code,
        "removal-proof-unavailable"
    );
}
