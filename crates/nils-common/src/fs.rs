use std::io;
use std::path::Path;

/// Replace `to` by renaming `from` to `to`.
///
/// Notes:
/// - On Unix, `rename` overwrites atomically when `from` and `to` are on the same filesystem.
/// - On Windows, `rename` fails when `to` exists. We fall back to remove + rename, which is not
///   atomic but matches the expected overwrite behavior for temp-file workflows.
pub fn replace_file(from: &Path, to: &Path) -> io::Result<()> {
    replace_file_impl(from, to)
}

/// Alias for `replace_file` (kept for readability at call sites).
pub fn rename_overwrite(from: &Path, to: &Path) -> io::Result<()> {
    replace_file(from, to)
}

#[cfg(unix)]
fn replace_file_impl(from: &Path, to: &Path) -> io::Result<()> {
    std::fs::rename(from, to)
}

#[cfg(windows)]
fn replace_file_impl(from: &Path, to: &Path) -> io::Result<()> {
    match std::fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(err) => {
            // Be conservative: do not delete `to` unless we can confirm `from` exists.
            if !from.exists() {
                return Err(err);
            }

            if !to.exists() {
                return Err(err);
            }

            match std::fs::remove_file(to) {
                Ok(()) => {}
                Err(remove_err) if remove_err.kind() == io::ErrorKind::NotFound => {}
                Err(remove_err) => {
                    return Err(io::Error::new(
                        io::ErrorKind::Other,
                        format!("rename failed: {err} (remove failed: {remove_err})"),
                    ));
                }
            }

            std::fs::rename(from, to).map_err(|err2| {
                io::Error::new(
                    io::ErrorKind::Other,
                    format!("rename failed: {err} ({err2})"),
                )
            })
        }
    }
}

#[cfg(not(any(unix, windows)))]
fn replace_file_impl(from: &Path, to: &Path) -> io::Result<()> {
    std::fs::rename(from, to)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn replace_file_overwrites_existing_destination() {
        let dir = TempDir::new().expect("tempdir");
        let from = dir.path().join("from.tmp");
        let to = dir.path().join("to.txt");

        fs::write(&from, "new").expect("write from");
        fs::write(&to, "old").expect("write to");

        replace_file(&from, &to).expect("replace_file");

        assert!(!from.exists(), "from should be moved away");
        assert_eq!(fs::read_to_string(&to).expect("read to"), "new");
    }
}
