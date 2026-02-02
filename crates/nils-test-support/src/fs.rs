use std::path::{Path, PathBuf};

use serde::Serialize;

fn ensure_parent_dir(path: &Path) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("mkdir");
    }
}

pub fn write_text(path: &Path, contents: &str) -> PathBuf {
    ensure_parent_dir(path);
    std::fs::write(path, contents).expect("write text");
    path.to_path_buf()
}

pub fn write_bytes(path: &Path, contents: &[u8]) -> PathBuf {
    ensure_parent_dir(path);
    std::fs::write(path, contents).expect("write bytes");
    path.to_path_buf()
}

pub fn write_json<T: Serialize>(path: &Path, value: &T) -> PathBuf {
    ensure_parent_dir(path);
    let data = serde_json::to_vec_pretty(value).expect("json");
    std::fs::write(path, data).expect("write json");
    path.to_path_buf()
}

pub fn write_executable(path: &Path, contents: &str) -> PathBuf {
    ensure_parent_dir(path);
    std::fs::write(path, contents).expect("write executable");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(path).expect("metadata").permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(path, perms).expect("set perms");
    }

    path.to_path_buf()
}
