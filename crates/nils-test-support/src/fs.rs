use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::Serialize;

/// Per-process sequence for unique sibling temp files.
static EXECUTABLE_TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

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

pub fn write_text_in_dir(dir: &Path, rel: &str, contents: &str) -> PathBuf {
    write_text(&dir.join(rel), contents)
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
    write_executable_with_mode(path, contents, 0o755)
}

/// Install an executable at `path` atomically, with `mode` applied before the
/// file becomes visible at `path`.
///
/// The contents are written to a sibling temp file, the file is synced, the
/// write descriptor is closed, and only then is the temp file renamed over
/// `path`. At the moment the rename lands, no process holds `path` open for
/// write, so an `execve` of it can never fail with ETXTBSY because of this
/// install. A direct write to `path` instead leaves it open for write for the
/// whole write: a sibling test thread that forks in that window inherits the
/// descriptor, the kernel keeps the inode text-busy until every such
/// descriptor is closed, and a pre-warm `exec` of the fixture fails with
/// "Text file busy" (sympoies/nils-cli#2170).
///
/// On non-unix platforms the file is written in place: `ETXTBSY` is a
/// unix-only failure, and std's `rename` does not replace an existing file
/// there.
pub fn write_executable_with_mode(path: &Path, contents: &str, mode: u32) -> PathBuf {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        ensure_parent_dir(path);
        let parent = path.parent().expect("parent directory");
        let name = path.file_name().expect("file name").to_string_lossy();
        let temp = parent.join(format!(
            ".{name}.tmp-{}-{}",
            std::process::id(),
            EXECUTABLE_TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));

        let mut file = std::fs::File::create(&temp).expect("create temp executable");
        file.set_permissions(std::fs::Permissions::from_mode(mode))
            .expect("mode temp executable");
        file.write_all(contents.as_bytes())
            .expect("write temp executable");
        file.sync_all().expect("sync temp executable");
        // Close the write descriptor before the rename: the final path must
        // never be observed open for write, or an exec of it fails with
        // ETXTBSY while a forked sibling still holds the inherited fd.
        drop(file);
        if let Err(error) = std::fs::rename(&temp, path) {
            let _ = std::fs::remove_file(&temp);
            panic!("rename temp executable into place: {error}");
        }
        if let Ok(directory) = std::fs::File::open(parent) {
            // Best effort: make the rename itself durable.
            let _ = directory.sync_all();
        }
        path.to_path_buf()
    }

    #[cfg(not(unix))]
    {
        let _ = mode;
        ensure_parent_dir(path);
        std::fs::write(path, contents).expect("write executable");
        path.to_path_buf()
    }
}

pub fn write_executable_in_dir(dir: &Path, rel: &str, contents: &str) -> PathBuf {
    write_executable(&dir.join(rel), contents)
}
