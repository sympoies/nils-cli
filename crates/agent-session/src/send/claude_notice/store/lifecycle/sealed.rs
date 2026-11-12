//! Retired data rejects new legacy append opens; quiescence still fences open FDs.
use super::*;
use std::os::unix::fs::PermissionsExt;

pub(super) fn seal(file: &File, parent: &Path, deadline: Instant) -> io::Result<()> {
    check_time(deadline)?;
    if file.metadata()?.mode() & 0o777 != 0o400 {
        file.set_permissions(fs::Permissions::from_mode(0o400))?;
    }
    file.sync_all()?;
    File::open(parent)?.sync_all()?;
    check_time(deadline)
}

pub(super) fn verified(
    parent: &Path,
    reference: &Reference,
    cap: usize,
    finish_pending_seal: bool,
    deadline: Instant,
) -> io::Result<Vec<u8>> {
    check_time(deadline)?;
    if !hash_valid(&reference.sha256) {
        return Err(invalid());
    }
    let path = parent.join(&reference.file);
    let before = fs::symlink_metadata(&path)?;
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    let meta = file.metadata()?;
    let mode = meta.mode() & 0o777;
    // Only the latest published archive can await its post-publication seal.
    // Readers never accept writable archives; writers finalize that exact tip.
    // SAFETY: geteuid has no preconditions.
    if !meta.is_file()
        || meta.uid() != unsafe { libc::geteuid() }
        || (mode != 0o400 && !(finish_pending_seal && mode == 0o600))
        || meta.len() > cap as u64
        || (meta.dev(), meta.ino()) != (before.dev(), before.ino())
    {
        return Err(invalid());
    }
    let mut bytes = Vec::new();
    (&file).take(cap as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > cap || sha256(&bytes) != reference.sha256 {
        return Err(invalid());
    }
    if finish_pending_seal {
        seal(&file, parent, deadline)?;
    }
    check_time(deadline)?;
    Ok(bytes)
}
