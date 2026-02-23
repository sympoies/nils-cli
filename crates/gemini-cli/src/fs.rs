use nils_common::fs as shared_fs;
use std::io;
use std::path::Path;

pub const SECRET_FILE_MODE: u32 = shared_fs::SECRET_FILE_MODE;

pub fn sha256_file(path: &Path) -> io::Result<String> {
    match shared_fs::sha256_file(path) {
        Ok(hash) => Ok(hash),
        Err(shared_fs::FileHashError::OpenFile { source, .. }) => Err(source),
        Err(shared_fs::FileHashError::ReadFile { source, .. }) => Err(source),
    }
}

pub fn write_atomic(path: &Path, contents: &[u8], mode: u32) -> io::Result<()> {
    match shared_fs::write_atomic(path, contents, mode) {
        Ok(()) => Ok(()),
        Err(shared_fs::AtomicWriteError::CreateParentDir { source, .. }) => Err(source),
        Err(shared_fs::AtomicWriteError::CreateTempFile { source, .. }) => Err(source),
        Err(shared_fs::AtomicWriteError::WriteTempFile { source, .. }) => Err(source),
        Err(shared_fs::AtomicWriteError::SetPermissions { source, .. }) => Err(source),
        Err(shared_fs::AtomicWriteError::ReplaceFile { source, .. }) => Err(source),
        Err(shared_fs::AtomicWriteError::TempPathExhausted { target, .. }) => Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("failed to create unique temp file for {}", target.display()),
        )),
    }
}

pub fn write_timestamp(path: &Path, iso: Option<&str>) -> io::Result<()> {
    match shared_fs::write_timestamp(path, iso) {
        Ok(()) => Ok(()),
        Err(shared_fs::TimestampError::CreateParentDir { source, .. }) => Err(source),
        Err(shared_fs::TimestampError::WriteFile { source, .. }) => Err(source),
        Err(shared_fs::TimestampError::RemoveFile { source, .. }) => Err(source),
    }
}
