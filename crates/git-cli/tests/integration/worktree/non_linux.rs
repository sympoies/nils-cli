use super::*;
use pretty_assertions::assert_eq;

#[test]
fn incomplete_holder_probes_retain_target() {
    for script in [
        "#!/bin/sh\nexit 2\n",
        "#!/bin/sh\necho 'incomplete inventory' >&2\nexit 1\n",
        "#!/bin/sh\nprintf malformed\n",
        "#!/bin/sh\nexit 0\n",
        "#!/bin/sh\nexec python3 -c 'import sys; sys.stdout.write(\"x\" * (9 * 1024 * 1024))'\n",
    ] {
        let fixture = RemovalFixture::new();
        fixture.probes.write_exe("lsof", script);
        fixture.refused("removal-proof-unavailable");
    }
}

#[test]
fn unfiltered_process_inventory_is_not_requested() {
    let fixture = RemovalFixture::new();
    // Enforce target selection and prove a reported outside name cannot undo
    // lsof's positive inode selection. No real OS probe is used in this test.
    fixture.probes.write_exe("lsof", "#!/bin/sh\n[ \"$1\" = -nP ] && [ \"$2\" = -F0pcfn ] && [ \"$3\" = +D ] && [ -d \"$4\" ] && [ \"$5\" = -x ] && [ \"$6\" = f ] || exit 2\nprintf 'p2147483647\\0cworker\\0\\nfmem\\0noutside-alias\\0\\n'\n");
    let result = fixture.remove("safe");
    assert_eq!(
        parse_json(&result)["error"]["code"],
        "removal-process-active",
        "{}",
        result.stdout_text()
    );
    assert!(Path::new(&fixture.target).exists());
}

#[test]
fn real_inventory_retains_mapping_without_a_descriptor() {
    use std::io::{BufRead, BufReader};
    use std::process::{Command, Stdio};
    let fixture = RemovalFixture::new();
    let path = Path::new(&fixture.target).join("README.md");
    let mut child = Command::new("python3")
        .args([
            "-c",
            include_str!("mapped_holder.py"),
            path.to_str().unwrap(),
        ])
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut ready = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut ready)
        .unwrap();
    assert_eq!(ready.trim(), "ready");
    // Exercise the system-wide production selection, with no PID restriction.
    fixture.probes.write_exe(
        "lsof",
        "#!/bin/sh\nexec /usr/bin/env PATH=/usr/bin:/usr/sbin:/bin:/sbin lsof \"$@\"\n",
    );
    let result = fixture.remove("safe");
    child.kill().unwrap();
    child.wait().unwrap();
    assert_eq!(
        parse_json(&result)["error"]["code"],
        "removal-process-active",
        "{}",
        result.stdout_text()
    );
    assert_eq!(parse_json(&result)["error"]["details"]["pid"], child.id());
    assert!(path.exists());
}
