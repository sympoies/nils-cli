use super::*;

/// Trusted per-user directory holding reusable private worker copies. Created
/// `0o700`; a pre-existing directory that is group/other writable or not owned
/// by the effective user is rejected as untrusted.
pub(super) fn private_test_worker_dir() -> Result<PathBuf> {
    let euid = unsafe { libc::geteuid() };
    let directory = env::temp_dir().join(format!("git-cli-test-worker.{euid}"));
    match fs::DirBuilder::new().mode(0o700).create(&directory) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => {
            return Err(error)
                .context("private dirty snapshot worker test root could not be created");
        }
    }
    let metadata = fs::symlink_metadata(&directory)
        .context("private dirty snapshot worker test root metadata is unavailable")?;
    if !metadata.file_type().is_dir() || metadata.uid() != euid || metadata.mode() & 0o022 != 0 {
        return Err(domain_error(
            DirtyCheckoutErrorKind::ResourceUnavailable,
            "private dirty snapshot worker test root is not trusted",
        ));
    }
    // TMPDIR may contain directory aliases. Resolve them before the unchanged
    // production checks inspect every ancestor of the materialized worker.
    fs::canonicalize(directory)
        .context("private dirty snapshot worker test root could not be resolved")
}
