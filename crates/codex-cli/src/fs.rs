use anyhow::Result;
use nils_common::fs as shared_fs;
use std::io;
use std::path::Path;

pub const SECRET_FILE_MODE: u32 = shared_fs::SECRET_FILE_MODE;

pub fn sha256_file(path: &Path) -> Result<String> {
    match shared_fs::sha256_file(path) {
        Ok(hash) => Ok(hash),
        Err(shared_fs::FileHashError::OpenFile { source, .. }) => Err(anyhow::Error::new(source)
            .context(format!("failed to open for sha256: {}", path.display()))),
        Err(shared_fs::FileHashError::ReadFile { source, .. }) => Err(source.into()),
    }
}

pub fn write_atomic(path: &Path, contents: &[u8], mode: u32) -> Result<()> {
    shared_fs::write_atomic(path, contents, mode).map_err(map_atomic_write_error)
}

pub fn write_timestamp(path: &Path, iso: Option<&str>) -> Result<()> {
    match shared_fs::write_timestamp(path, iso) {
        Ok(()) => Ok(()),
        Err(shared_fs::TimestampError::CreateParentDir { path, source }) => {
            Err(anyhow::Error::new(source)
                .context(format!("failed to create dir: {}", path.display())))
        }
        Err(shared_fs::TimestampError::WriteFile { path, source }) => {
            Err(anyhow::Error::new(source)
                .context(format!("failed to write timestamp: {}", path.display())))
        }
        Err(shared_fs::TimestampError::RemoveFile { .. }) => Ok(()),
    }
}

fn map_atomic_write_error(err: shared_fs::AtomicWriteError) -> anyhow::Error {
    match err {
        shared_fs::AtomicWriteError::CreateParentDir { path, source } => {
            anyhow::Error::new(source).context(format!("failed to create dir: {}", path.display()))
        }
        shared_fs::AtomicWriteError::CreateTempFile { source, .. } => {
            anyhow::Error::new(source).context("failed to create temp file")
        }
        shared_fs::AtomicWriteError::TempPathExhausted { .. } => anyhow::Error::new(
            io::Error::new(io::ErrorKind::AlreadyExists, "temp file already exists"),
        )
        .context("failed to create unique temp file"),
        shared_fs::AtomicWriteError::WriteTempFile { path, source } => anyhow::Error::new(source)
            .context(format!("failed to write temp file: {}", path.display())),
        shared_fs::AtomicWriteError::SetPermissions { path, source } => anyhow::Error::new(source)
            .context(format!("failed to set permissions: {}", path.display())),
        shared_fs::AtomicWriteError::ReplaceFile { from, to, source } => anyhow::Error::new(source)
            .context(format!(
                "failed to rename {} -> {}",
                from.display(),
                to.display()
            )),
    }
}
