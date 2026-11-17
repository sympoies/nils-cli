//! Size-bounded, concurrency-safe append of identity audit records.
//!
//! Bound: the active file never exceeds [`LIMITS`]`.max_active_bytes` (4 MiB) and at most
//! `kept_rotated` (4) rotated copies are kept, so the audit trail occupies at most
//! 5 files / 20 MiB, provided each record is smaller than the limit (records are well under 4 KiB).
//! Rotated copies are named `identity-audit.jsonl.1` (newest) to `.4` (oldest); the oldest is deleted.
//! Writers serialise on an advisory lock, so a rotation never drops or splits the record being written.
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::os::unix::io::AsRawFd;
use std::path::Path;
use std::time::Duration;

const LOCK_TIMEOUT: Duration = Duration::from_secs(1);

/// Active file name inside the audit directory.
pub(super) const ACTIVE_NAME: &str = "identity-audit.jsonl";
const LOCK_NAME: &str = "identity-audit.lock";

#[derive(Debug, Clone, Copy)]
pub(super) struct Limits {
    pub max_active_bytes: u64,
    pub kept_rotated: usize,
}

pub(super) const LIMITS: Limits = Limits {
    max_active_bytes: 4 * 1024 * 1024,
    kept_rotated: 4,
};

fn open_private(path: &Path, append: bool) -> io::Result<File> {
    OpenOptions::new()
        .create(true)
        .write(!append)
        .append(append)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
}

/// Exclusive advisory lock held for the rotate-and-append critical section.
struct Lock(#[allow(dead_code)] File);
impl Lock {
    fn acquire(dir: &Path, timeout: Duration) -> io::Result<Self> {
        let file = open_private(&dir.join(LOCK_NAME), false)?;
        let deadline = super::probe::now() + timeout;
        loop {
            // SAFETY: the descriptor remains open while the lock is acquired.
            if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
                return Ok(Self(file));
            }
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::WouldBlock
                && error.kind() != io::ErrorKind::Interrupted
            {
                return Err(error);
            }
            let remaining = deadline.saturating_duration_since(super::probe::now());
            if remaining.is_zero() {
                return Err(io::Error::from(io::ErrorKind::TimedOut));
            }
            std::thread::sleep(remaining.min(Duration::from_millis(10)));
        }
    }
}

fn rotated(dir: &Path, index: usize) -> std::path::PathBuf {
    dir.join(format!("{ACTIVE_NAME}.{index}"))
}
fn rotate(dir: &Path, kept: usize) -> io::Result<()> {
    let ignore_missing = |r: io::Result<()>| match r {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        other => other,
    };
    ignore_missing(fs::remove_file(rotated(dir, kept)))?;
    for index in (1..kept).rev() {
        ignore_missing(fs::rename(rotated(dir, index), rotated(dir, index + 1)))?;
    }
    fs::rename(dir.join(ACTIVE_NAME), rotated(dir, 1))
}

/// Append one complete record, rotating first when it would push the active file past the limit.
pub(super) fn append(dir: &Path, record: &[u8], limits: Limits) -> io::Result<()> {
    append_with_timeout(dir, record, limits, LOCK_TIMEOUT)
}

fn append_with_timeout(
    dir: &Path,
    record: &[u8],
    limits: Limits,
    timeout: Duration,
) -> io::Result<()> {
    let _lock = Lock::acquire(dir, timeout)?;
    let active = dir.join(ACTIVE_NAME);
    let mut file = open_private(&active, true)?;
    let metadata = file.metadata()?;
    if metadata.permissions().mode() & 0o077 != 0 {
        return Err(io::Error::from(io::ErrorKind::PermissionDenied));
    }
    if metadata.len() > 0 && metadata.len() + record.len() as u64 > limits.max_active_bytes {
        drop(file);
        rotate(dir, limits.kept_rotated)?;
        file = open_private(&active, true)?;
    }
    file.write_all(record)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use std::collections::BTreeSet;
    use std::fs;

    const SMALL: Limits = Limits {
        max_active_bytes: 512,
        kept_rotated: 3,
    };

    fn line(n: usize) -> Vec<u8> {
        format!("{{\"n\":{n},\"pad\":\"{}\"}}\n", "x".repeat(60)).into_bytes()
    }
    fn files(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .filter(|n| n.starts_with(ACTIVE_NAME))
            .collect();
        names.sort();
        names
    }

    #[test]
    fn contended_lock_returns_within_a_short_bound() {
        let dir = tempfile::tempdir().unwrap();
        use std::io::BufRead;
        use std::process::{Command, Stdio};
        let mut child = Command::new("python3")
            .args(["-c", "import fcntl, sys, time; f = open(sys.argv[1], 'w'); fcntl.flock(f, fcntl.LOCK_EX); print('locked', flush=True); time.sleep(0.3)"])
            .arg(dir.path().join(LOCK_NAME))
            .stdout(Stdio::piped()).spawn().unwrap();
        let mut ready = String::new();
        std::io::BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut ready)
            .unwrap();
        assert_eq!(ready.trim(), "locked");
        let start = super::super::probe::now();
        let result = append_with_timeout(dir.path(), b"{}\n", SMALL, Duration::from_millis(20))
            .map_err(super::super::audit_error);
        let required_code = super::super::unasserted::finish_audit(
            super::super::unasserted::Mode::Deny,
            result,
            &mut Vec::new(),
        )
        .unwrap_err()
        .code;
        let optional = append_with_timeout(dir.path(), b"{}\n", SMALL, Duration::from_millis(20))
            .map_err(super::super::audit_error);
        let mut warning = Vec::new();
        super::super::unasserted::finish_audit(
            super::super::unasserted::Mode::Warn,
            optional,
            &mut warning,
        )
        .unwrap();
        let elapsed = start.elapsed();
        child.wait().unwrap();
        assert!(
            elapsed < std::time::Duration::from_millis(200),
            "lock wait took {elapsed:?}"
        );
        assert_eq!(required_code, "identity_audit_lock_timeout");
        assert_eq!(
            String::from_utf8(warning).unwrap(),
            "warning: forge identity: identity_audit_lock_timeout\n"
        );
        assert!(!dir.path().join(ACTIVE_NAME).exists());
    }

    #[test]
    fn filling_past_the_limit_rotates_and_bounds_disk_use() {
        let dir = tempfile::tempdir().unwrap();
        for n in 0..200 {
            append(dir.path(), &line(n), SMALL).unwrap();
            let active = fs::metadata(dir.path().join(ACTIVE_NAME)).unwrap().len();
            assert!(active <= SMALL.max_active_bytes, "active grew to {active}");
        }
        assert_eq!(
            files(dir.path()),
            [
                "identity-audit.jsonl",
                "identity-audit.jsonl.1",
                "identity-audit.jsonl.2",
                "identity-audit.jsonl.3"
            ]
        );
        let total: u64 = files(dir.path())
            .iter()
            .map(|f| fs::metadata(dir.path().join(f)).unwrap().len())
            .sum();
        assert!(total <= SMALL.max_active_bytes * (SMALL.kept_rotated as u64 + 1));
        // The newest record is always in the active file; older ones moved to higher suffixes.
        let active = fs::read_to_string(dir.path().join(ACTIVE_NAME)).unwrap();
        assert!(active.ends_with(&String::from_utf8(line(199)).unwrap()));
        let first_rotated = fs::read_to_string(dir.path().join("identity-audit.jsonl.1")).unwrap();
        assert!(!first_rotated.contains("\"n\":199,"));
    }

    #[test]
    fn rotated_files_keep_owner_only_mode() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        for n in 0..20 {
            append(dir.path(), &line(n), SMALL).unwrap();
        }
        for name in files(dir.path()) {
            let mode = fs::metadata(dir.path().join(&name))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o077, 0, "{name} is accessible beyond the owner");
        }
    }

    #[test]
    fn concurrent_writers_lose_no_record_and_never_exceed_the_bound() {
        let limits = Limits {
            max_active_bytes: 2048,
            kept_rotated: 200,
        };
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_path_buf();
        let handles: Vec<_> = (0..8)
            .map(|t| {
                let path = path.clone();
                std::thread::spawn(move || {
                    for i in 0..100 {
                        append(&path, &line(t * 1000 + i), limits).unwrap();
                        let active = fs::metadata(path.join(ACTIVE_NAME)).unwrap().len();
                        assert!(active <= limits.max_active_bytes);
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        let mut seen = BTreeSet::new();
        let mut count = 0;
        for name in files(dir.path()) {
            let text = fs::read_to_string(dir.path().join(name)).unwrap();
            for l in text.lines() {
                let v: serde_json::Value = serde_json::from_str(l).expect("whole record per line");
                seen.insert(v["n"].as_u64().unwrap());
                count += 1;
            }
        }
        assert_eq!((count, seen.len()), (800, 800));
    }
}
