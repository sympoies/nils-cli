use std::ffi::OsString;
use std::path::{Path, PathBuf};

pub fn cmd_exists(program: &str) -> bool {
    find_in_path(program).is_some()
}

pub fn find_in_path(program: &str) -> Option<PathBuf> {
    if looks_like_path(program) {
        let p = PathBuf::from(program);
        return is_executable_file(&p).then_some(p);
    }

    let path_var: OsString = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join(program);
        if is_executable_file(&candidate) {
            return Some(candidate);
        }
    }
    None
}

fn looks_like_path(program: &str) -> bool {
    // Treat both separators as paths, even on unix. It is harmless and avoids surprises when a
    // caller passes a Windows-style path.
    program.contains('/') || program.contains('\\')
}

fn is_executable_file(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nils_test_support::{prepend_path, GlobalStateLock, StubBinDir};
    use std::fs;

    #[test]
    fn find_in_path_with_explicit_missing_path_returns_none() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let path = dir.path().join("missing");

        let found = find_in_path(path.to_string_lossy().as_ref());

        assert!(found.is_none());
    }

    #[cfg(unix)]
    #[test]
    fn find_in_path_with_non_executable_file_returns_none() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::TempDir::new().expect("tempdir");
        let path = dir.path().join("file");
        fs::write(&path, "data").expect("write file");

        let mut perms = fs::metadata(&path).expect("metadata").permissions();
        perms.set_mode(0o644);
        fs::set_permissions(&path, perms).expect("set permissions");

        let found = find_in_path(path.to_string_lossy().as_ref());

        assert!(found.is_none());
    }

    #[cfg(unix)]
    #[test]
    fn find_in_path_with_executable_file_returns_path() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::TempDir::new().expect("tempdir");
        let path = dir.path().join("exec");
        fs::write(&path, "data").expect("write file");

        let mut perms = fs::metadata(&path).expect("metadata").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&path, perms).expect("set permissions");

        let found = find_in_path(path.to_string_lossy().as_ref());

        assert_eq!(found, Some(path));
    }

    #[test]
    fn find_in_path_resolves_from_path_env() {
        let lock = GlobalStateLock::new();
        let stub = StubBinDir::new();
        stub.write_exe("hello-stub", "#!/bin/sh\necho hi\n");

        let _path_guard = prepend_path(&lock, stub.path());

        let found = find_in_path("hello-stub").expect("found");
        assert!(found.ends_with("hello-stub"));
    }
}
