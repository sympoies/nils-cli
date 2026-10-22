//! Scan reported lsof paths without inode-based directory selection.
use super::{CliError, refused};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

pub(super) fn check(bytes: &[u8], target: &Path, excluded: &[u32]) -> Result<(), CliError> {
    let mut pid = None;
    let mut command = String::new();
    for field in bytes.split(|b| *b == 0) {
        let field = field.strip_prefix(b"\n").unwrap_or(field);
        let Some((&kind, value)) = field.split_first() else {
            continue;
        };
        match kind {
            b'p' => {
                pid = std::str::from_utf8(value)
                    .ok()
                    .and_then(|s| s.parse::<u32>().ok());
                command.clear();
            }
            b'c' => command = String::from_utf8_lossy(value).into_owned(),
            b'n' if Path::new(std::ffi::OsStr::from_bytes(
                value.strip_suffix(b" (deleted)").unwrap_or(value),
            ))
            .starts_with(target) =>
            {
                if let Some(pid) = pid.filter(|p| !excluded.contains(p)) {
                    return Err(refused("removal-process-active", "a readable process cwd, file or mapping is inside the target").with_details(serde_json::json!({"pid":pid,"command":command,"backend":"lsof","evidence":"path"})));
                }
            }
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn shared_inode_requires_target_path() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("target");
        std::fs::create_dir(&target).unwrap();
        let outside = root.path().join("outside");
        std::fs::write(&outside, "shared").unwrap();
        let inside = target.join("shared");
        std::fs::hard_link(&outside, &inside).unwrap();
        for descriptor in ["cwd", "3r", "txt", "mem"] {
            for (path, blocks) in [(&outside, false), (&inside, true)] {
                let output = format!("p42\0cworker\0\nf{descriptor}\0n{}\0\n", path.display());
                assert_eq!(check(output.as_bytes(), &target, &[]).is_err(), blocks);
                assert!(check(output.as_bytes(), &target, &[42]).is_ok());
            }
        }
    }
}
