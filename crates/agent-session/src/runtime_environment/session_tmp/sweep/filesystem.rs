//! Keep both the preflight and deletion walk on the tmp root's mount.
use super::*;
use std::ffi::{CStr, CString};
use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd};

#[cfg(target_os = "linux")]
type MountId = u64;
#[cfg(target_os = "macos")]
type MountId = Vec<libc::c_char>;
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
type MountId = ();

pub(super) struct Boundary {
    root: fs::File,
    mount: MountId,
}

fn untrusted() -> io::Error {
    io::Error::other("session tmp tree is untrusted")
}

impl Boundary {
    pub(super) fn open(path: &Path) -> io::Result<Self> {
        let root = fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(path)?;
        let mount = mount_id(&root).ok_or_else(untrusted)?;
        Ok(Self { root, mount })
    }

    pub(super) fn trusted_tree(&self, id: &str) -> bool {
        CString::new(id).is_ok_and(|name| visit(&self.root, &name, &self.mount, false).is_ok())
    }

    pub(super) fn remove_tree(&self, id: &str) -> io::Result<()> {
        let name = CString::new(id).map_err(|_| untrusted())?;
        visit(&self.root, &name, &self.mount, true)
    }
}

fn visit(parent: &fs::File, name: &CStr, mount: &MountId, remove: bool) -> io::Result<()> {
    // Pin the object before checking its mount. A concurrent overmount cannot
    // redirect this descriptor; unlinkat never recurses into a replacement.
    // SAFETY: parent is open and name is NUL-terminated, without a slash.
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: openat returned a new descriptor, now exclusively owned by File.
    let file = unsafe { fs::File::from_raw_fd(fd) };
    let metadata = file.metadata()?;
    // SAFETY: geteuid has no preconditions.
    if metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o022 != 0
        || mount_id(&file).as_ref() != Some(mount)
    {
        return Err(untrusted());
    }
    let flags = if metadata.is_dir() {
        for child in names(&file)? {
            visit(&file, &child, mount, remove)?;
        }
        libc::AT_REMOVEDIR
    } else if metadata.is_file() {
        0
    } else {
        return Err(untrusted());
    };
    if remove {
        // SAFETY: both arguments remain valid; this only unlinks the named
        // entry. An overmounted entry fails with EBUSY without traversing it.
        if unsafe { libc::unlinkat(parent.as_raw_fd(), name.as_ptr(), flags) } != 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn mount_id(file: &fs::File) -> Option<MountId> {
    let mut stat = std::mem::MaybeUninit::<libc::statx>::uninit();
    // Linux 5.8+ supplies mount IDs, including distinct IDs for bind mounts.
    // Missing evidence on an older kernel is a retention decision, not a
    // reason to fall back to st_dev (which cannot recognize same-device binds).
    // SAFETY: the descriptor and empty C string are valid; stat is writable.
    let result = unsafe {
        libc::statx(
            file.as_raw_fd(),
            c"".as_ptr(),
            libc::AT_EMPTY_PATH | libc::AT_SYMLINK_NOFOLLOW,
            libc::STATX_MNT_ID,
            stat.as_mut_ptr(),
        )
    };
    if result != 0 {
        return None;
    }
    // SAFETY: statx succeeded and initialized stat.
    let stat = unsafe { stat.assume_init() };
    (stat.stx_mask & libc::STATX_MNT_ID != 0).then_some(stat.stx_mnt_id)
}

#[cfg(target_os = "macos")]
fn mount_id(file: &fs::File) -> Option<MountId> {
    let mut stat = std::mem::MaybeUninit::<libc::statfs>::uninit();
    // SAFETY: the descriptor is live and stat points to writable storage.
    if unsafe { libc::fstatfs(file.as_raw_fd(), stat.as_mut_ptr()) } != 0 {
        return None;
    }
    // SAFETY: fstatfs succeeded and initialized stat. Mounted-on names identify
    // nested mount points, even when their backing filesystem is shared.
    let stat = unsafe { stat.assume_init() };
    let end = stat.f_mntonname.iter().position(|byte| *byte == 0)?;
    (end > 0).then(|| stat.f_mntonname[..end].to_vec())
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn mount_id(_file: &fs::File) -> Option<MountId> {
    None
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn names(file: &fs::File) -> io::Result<Vec<CString>> {
    let fd = file.try_clone()?.into_raw_fd();
    // SAFETY: fd is a new owned descriptor for a directory.
    let stream = unsafe { libc::fdopendir(fd) };
    if stream.is_null() {
        let error = io::Error::last_os_error();
        // SAFETY: failed fdopendir did not take ownership of fd.
        unsafe { libc::close(fd) };
        return Err(error);
    }
    struct Directory(*mut libc::DIR);
    impl Drop for Directory {
        fn drop(&mut self) {
            // SAFETY: this wrapper uniquely owns a live directory stream.
            unsafe { libc::closedir(self.0) };
        }
    }
    let stream = Directory(stream);
    let mut names = Vec::new();
    loop {
        // SAFETY: errno is thread-local; stream is live and used by this thread
        // only. Copy each entry before the next readdir invalidates its buffer.
        unsafe {
            #[cfg(target_os = "linux")]
            let errno = libc::__errno_location();
            #[cfg(target_os = "macos")]
            let errno = libc::__error();
            *errno = 0;
            let entry = libc::readdir(stream.0);
            if entry.is_null() {
                return if *errno == 0 {
                    Ok(names)
                } else {
                    Err(io::Error::from_raw_os_error(*errno))
                };
            }
            let name = CStr::from_ptr((*entry).d_name.as_ptr());
            if name != c"." && name != c".." {
                names.push(name.to_owned());
            }
        }
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn names(_file: &fs::File) -> io::Result<Vec<CString>> {
    Err(untrusted())
}
