//! Native identity proof for provider-owned Codex processes on macOS.
use super::*;
use std::os::unix::ffi::OsStrExt;

pub(super) fn classify(pid: libc::pid_t) -> Option<&'static str> {
    let unique = unique_id(pid)?;
    let info = macos_orphans::process_info(pid).ok()??;
    let executable = executable_path(pid)?;
    let (home, records) = provider_records(&executable)?;
    let boot = capture_macos_boot_id()?;
    let command = records.into_iter().find_map(|(file, label)| {
        let record = read_pid_record(&home.join("app-server-daemon").join(file))?;
        let identity = &record["processIdentity"];
        (record["pid"].as_i64() == Some(i64::from(pid))
            && identity["uniqueId"].as_u64() == Some(unique)
            && identity["startSeconds"].as_u64() == Some(info.pbi_start_tvsec)
            && identity["startMicroseconds"].as_u64() == Some(info.pbi_start_tvusec)
            && identity["bootId"]
                .as_str()
                .is_some_and(|recorded| recorded.eq_ignore_ascii_case(&boot)))
        .then_some(label)
    })?;
    (unique_id(pid) == Some(unique) && macos_orphans::process_info(pid).ok().flatten().is_some())
        .then_some(command)
}

fn provider_records(executable: &Path) -> Option<(&Path, [(&'static str, &'static str); 2])> {
    // Codex 0.162.1 writes these PID records for its managed and standalone
    // packages. Derive the provider home from the live executable's native
    // path. A private Codex process can use that same executable, so it still
    // needs a matching provider PID record to be retained.
    let bin = executable.parent()?;
    let releases = bin.parent()?.parent()?;
    let package = releases.parent()?;
    let packages = package.parent()?;
    if executable.file_name()? != "codex"
        || bin.file_name()? != "bin"
        || releases.file_name()? != "releases"
        || packages.file_name()? != "packages"
    {
        return None;
    }
    let records = match package.file_name()?.to_str()? {
        "app-server-daemon" => [
            ("daemon.pid", "codex-app-server"),
            ("daemon-updater.pid", "codex-daemon-updater"),
        ],
        "standalone" => [
            ("app-server.pid", "codex-app-server"),
            ("app-server-updater.pid", "codex-daemon-updater"),
        ],
        _ => return None,
    };
    Some((packages.parent()?, records))
}

fn read_pid_record(path: &Path) -> Option<Value> {
    // A malformed record, symlink, or FIFO must not exempt the process or
    // block close. The provider writes a small, owner-writable JSON file.
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(path)
        .ok()?;
    let metadata = file.metadata().ok()?;
    if !metadata.is_file()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.nlink() != 1
        || metadata.mode() & 0o022 != 0
        || metadata.len() > 8192
    {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(8193).read_to_end(&mut bytes).ok()?;
    (bytes.len() <= 8192)
        .then(|| serde_json::from_slice(&bytes).ok())
        .flatten()
}

fn executable_path(pid: libc::pid_t) -> Option<PathBuf> {
    let mut buffer = [0_u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
    let read = unsafe { libc::proc_pidpath(pid, buffer.as_mut_ptr().cast(), buffer.len() as u32) };
    if read <= 0 {
        return None;
    }
    let path = std::ffi::CStr::from_bytes_until_nul(&buffer).ok()?;
    let path = Path::new(std::ffi::OsStr::from_bytes(path.to_bytes()));
    path.is_absolute().then(|| path.to_path_buf())
}

fn unique_id(pid: libc::pid_t) -> Option<u64> {
    // XNU proc_uniqidentifierinfo, as used in the Codex 0.162.1 PID record.
    const PROC_PIDUNIQIDENTIFIERINFO: libc::c_int = 17;
    #[repr(C)]
    struct UniqueInfo {
        executable_uuid: [u8; 16],
        unique_id: u64,
        parent_unique_id: u64,
        reserved: [u64; 3],
    }
    let mut info = std::mem::MaybeUninit::<UniqueInfo>::uninit();
    let size = std::mem::size_of::<UniqueInfo>() as libc::c_int;
    let read = unsafe {
        libc::proc_pidinfo(
            pid,
            PROC_PIDUNIQIDENTIFIERINFO,
            1,
            info.as_mut_ptr().cast(),
            size,
        )
    };
    (read == size).then(|| unsafe { info.assume_init() }.unique_id)
}
