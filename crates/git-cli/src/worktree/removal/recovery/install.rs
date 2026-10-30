//! Create-new, no-follow installation of recovered entries into a verified root.
//!
//! Every step is relative to an already-verified directory descriptor, so a
//! parent replaced by a symlink is refused and an entry that appeared after
//! the missing-entry scan is left alone instead of overwritten.
use std::{
    ffi::CString,
    fs::{self, File},
    io::{self, Error, ErrorKind},
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd, RawFd},
        unix::{
            ffi::OsStrExt,
            fs::{MetadataExt, PermissionsExt},
        },
    },
    path::Path,
};

const DIRECTORY: libc::c_int =
    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;

/// Open `target` as the root, requiring it to be the verified directory.
pub(super) fn open_root(target: &Path, identity: (u64, u64)) -> io::Result<OwnedFd> {
    let path = CString::new(target.as_os_str().as_bytes())?;
    let fd = unsafe { libc::open(path.as_ptr(), DIRECTORY) };
    if fd < 0 {
        return Err(Error::last_os_error());
    }
    let root = unsafe { OwnedFd::from_raw_fd(fd) };
    let metadata = File::from(root.try_clone()?).metadata()?;
    if (metadata.dev(), metadata.ino()) != identity {
        return Err(Error::new(
            ErrorKind::InvalidData,
            "target identity changed",
        ));
    }
    Ok(root)
}

fn name(bytes: &[u8]) -> io::Result<CString> {
    if bytes.is_empty() || bytes == b"." || bytes == b".." {
        return Err(Error::new(ErrorKind::InvalidInput, "unsafe path component"));
    }
    Ok(CString::new(bytes)?)
}

fn open_dir(parent: RawFd, component: &CString, create: bool) -> io::Result<OwnedFd> {
    for _ in 0..2 {
        let fd = unsafe { libc::openat(parent, component.as_ptr(), DIRECTORY) };
        if fd >= 0 {
            return Ok(unsafe { OwnedFd::from_raw_fd(fd) });
        }
        let error = Error::last_os_error();
        if !create || error.kind() != ErrorKind::NotFound {
            return Err(error);
        }
        if unsafe { libc::mkdirat(parent, component.as_ptr(), 0o755) } != 0 {
            let error = Error::last_os_error();
            if error.kind() != ErrorKind::AlreadyExists {
                return Err(error);
            }
        }
    }
    Err(Error::other("directory kept changing"))
}

/// Create `name` in `dir` exclusively. `None` means it already exists.
pub(super) fn create_file(dir: RawFd, file: &CString, mode: u32) -> io::Result<Option<File>> {
    let flags = libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC;
    let fd = unsafe { libc::openat(dir, file.as_ptr(), flags, mode as libc::c_uint) };
    if fd >= 0 {
        return Ok(Some(File::from(unsafe { OwnedFd::from_raw_fd(fd) })));
    }
    match Error::last_os_error() {
        error if error.kind() == ErrorKind::AlreadyExists => Ok(None),
        error => Err(error),
    }
}

pub(super) fn dot_git(root: &OwnedFd) -> io::Result<Option<File>> {
    create_file(root.as_raw_fd(), &name(b".git")?, 0o644)
}

/// Install the entry at `relative` from `scratch`. Returns whether it was
/// created; an entry that already exists is never touched.
pub(super) fn install(root: &OwnedFd, scratch: &Path, relative: &[u8]) -> io::Result<bool> {
    let mut components = relative.split(|byte| *byte == b'/').peekable();
    let mut parents = Vec::new();
    let mut current = root.try_clone()?;
    let leaf = loop {
        let component = name(components.next().unwrap_or_default())?;
        if components.peek().is_none() {
            break component;
        }
        let next = open_dir(current.as_raw_fd(), &component, true)?;
        parents.push(std::mem::replace(&mut current, next));
    };
    let source = scratch.join(std::ffi::OsStr::from_bytes(relative));
    let metadata = fs::symlink_metadata(&source)?;
    if metadata.file_type().is_symlink() {
        let link = CString::new(fs::read_link(&source)?.as_os_str().as_bytes())?;
        if unsafe { libc::symlinkat(link.as_ptr(), current.as_raw_fd(), leaf.as_ptr()) } == 0 {
            return Ok(true);
        }
        return match Error::last_os_error() {
            error if error.kind() == ErrorKind::AlreadyExists => Ok(false),
            error => Err(error),
        };
    }
    if !metadata.is_file() {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            "unsupported entry type",
        ));
    }
    let Some(mut created) = create_file(current.as_raw_fd(), &leaf, 0o600)? else {
        return Ok(false);
    };
    let copied = io::copy(&mut File::open(&source)?, &mut created)
        .and_then(|_| created.set_permissions(fs::Permissions::from_mode(metadata.mode() & 0o777)));
    if let Err(error) = copied {
        unsafe { libc::unlinkat(current.as_raw_fd(), leaf.as_ptr(), 0) };
        return Err(error);
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use std::os::unix::fs::symlink;

    fn fixture() -> (tempfile::TempDir, tempfile::TempDir, OwnedFd) {
        let root = tempfile::tempdir().unwrap();
        let scratch = tempfile::tempdir().unwrap();
        fs::create_dir_all(scratch.path().join("dir")).unwrap();
        fs::write(scratch.path().join("dir/file"), "snapshot").unwrap();
        let metadata = fs::metadata(root.path()).unwrap();
        let fd = open_root(root.path(), (metadata.dev(), metadata.ino())).unwrap();
        (root, scratch, fd)
    }

    #[test]
    fn existing_leaf_is_never_overwritten() {
        let (root, scratch, fd) = fixture();
        fs::create_dir(root.path().join("dir")).unwrap();
        fs::write(root.path().join("dir/file"), "late writer").unwrap();
        assert!(!install(&fd, scratch.path(), b"dir/file").unwrap());
        assert_eq!(
            fs::read(root.path().join("dir/file")).unwrap(),
            b"late writer"
        );
    }

    #[test]
    fn symlinked_parent_is_refused_without_writing_through_it() {
        let (root, scratch, fd) = fixture();
        let outside = tempfile::tempdir().unwrap();
        symlink(outside.path(), root.path().join("dir")).unwrap();
        assert!(install(&fd, scratch.path(), b"dir/file").is_err());
        assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
        assert!(
            fs::symlink_metadata(root.path().join("dir"))
                .unwrap()
                .is_symlink()
        );
    }

    #[test]
    fn replaced_root_is_refused() {
        let root = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let metadata = fs::metadata(other.path()).unwrap();
        assert!(open_root(root.path(), (metadata.dev(), metadata.ino())).is_err());
    }
}
