//! Host-wide contained-runner admission, independent of repository/worktree.
use super::*;

pub(super) struct RunnerPermit {
    file: File,
    pub(super) limit: usize,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SlotOwner {
    pid: u32,
    since: String,
    unit: Option<String>,
}

fn unavailable() -> HookError {
    finish_line_unavailable(
        "finish-line-resource-unavailable",
        "contained-runner resource admission is unavailable",
    )
}

fn setting(name: &str, default: u64, maximum: u64) -> Result<u64, HookError> {
    let value = match std::env::var(name) {
        Ok(value) => value.parse::<u64>().map_err(|_| unavailable())?,
        Err(std::env::VarError::NotPresent) => default,
        Err(_) => return Err(unavailable()),
    };
    if !(1..=maximum).contains(&value) {
        return Err(unavailable());
    }
    Ok(value)
}

fn default_limit(cpus: usize, available_bytes: u64) -> usize {
    (cpus / 4)
        .min((available_bytes / (4 * 1024 * 1024 * 1024)) as usize)
        .clamp(1, 2)
}

pub(super) fn slice_argument() -> Result<Option<String>, HookError> {
    match std::env::var("NILS_CLI_GATE_SLICE") {
        Err(std::env::VarError::NotPresent) => Ok(None),
        Ok(slice)
            if slice
                .strip_prefix("nilsgate")
                .and_then(|name| name.strip_suffix(".slice"))
                .is_some_and(|id| id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit())) =>
        {
            Ok(Some(format!("--slice={slice}")))
        }
        _ => Err(unavailable()),
    }
}

pub(super) fn memory_properties() -> Result<Vec<String>, HookError> {
    let maximum = setting("NILS_CLI_GATE_MEMORY_MAX_GIB", 16, 1048576)?;
    let high = setting(
        "NILS_CLI_GATE_MEMORY_HIGH_GIB",
        (maximum * 3 / 4).max(1),
        maximum,
    )?;
    Ok(vec![
        format!("--property=MemoryHigh={high}G"),
        format!("--property=MemoryMax={maximum}G"),
        "--property=MemorySwapMax=0".into(),
        "--property=OOMPolicy=kill".into(),
    ])
}

pub(super) fn bound_environment(
    environment: &mut BTreeMap<String, String>,
    limit: usize,
) -> Result<(), HookError> {
    environment.insert("NILS_CLI_RUNNER_MAX".into(), limit.to_string());
    environment.insert("NILS_CLI_CONTAINED_RUNNER_ACTIVE".into(), "1".into());
    for name in [
        "NEXTEST_TEST_THREADS",
        "RUST_TEST_THREADS",
        "CARGO_BUILD_JOBS",
    ] {
        let default = if name == "CARGO_BUILD_JOBS" {
            1
        } else {
            limit as u64
        };
        let value = environment.get(name).map_or(Ok(default), |value| {
            value.parse::<u64>().map_err(|_| unavailable())
        })?;
        if value == 0 {
            return Err(unavailable());
        }
        environment.insert(name.into(), value.min(limit as u64).to_string());
    }
    Ok(())
}

impl RunnerPermit {
    pub(super) fn acquire(unit: &str) -> Result<Self, HookError> {
        let available = fs::read_to_string("/proc/meminfo")
            .ok()
            .and_then(|text| {
                text.lines().find_map(|line| {
                    line.strip_prefix("MemAvailable:")
                        .and_then(|value| value.split_whitespace().next())
                        .and_then(|value| value.parse::<u64>().ok())
                })
            })
            .ok_or_else(unavailable)?
            * 1024;
        let cpus = thread::available_parallelism().map_or(1, usize::from);
        let limit = setting(
            "NILS_CLI_RUNNER_MAX",
            default_limit(cpus, available) as u64,
            64,
        )?;
        let timeout = setting("NILS_CLI_GATE_TIMEOUT_SECONDS", 3600, 86400)?;
        let gate_admitted = std::env::var("NILS_CLI_GATE_ACTIVE").is_ok_and(|value| value == "1")
            && slice_argument()?.is_some();
        let root = std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state"))
            })
            .ok_or_else(unavailable)?;
        let directory = std::env::var_os("NILS_CLI_RESOURCE_STATE_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| root.join("nils-cli/resources"));
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&directory)
            .map_err(|_| unavailable())?;
        Self::acquire_in(
            &directory,
            limit as usize,
            gate_admitted,
            Duration::from_secs(timeout),
            Some(unit),
            |previous| {
                Ok(contained_unit_is_quiescent(previous)?
                    && !contained_unit_has_pending_job(previous)?)
            },
            || VALIDATION_CANCEL_SIGNAL.load(Ordering::SeqCst) != 0,
        )
    }

    fn acquire_in(
        directory: &Path,
        limit: usize,
        gate_admitted: bool,
        timeout: Duration,
        unit: Option<&str>,
        quiescent: impl Fn(&str) -> Result<bool, HookError>,
        cancelled: impl Fn() -> bool,
    ) -> Result<Self, HookError> {
        let started = Instant::now();
        let mut notice = Instant::now();
        // Keep one slot available for an admitted gate's child. Queued outer
        // gates cannot consume the last slot and deadlock the active gate.
        let slots: Vec<_> = if gate_admitted {
            std::iter::once(limit - 1).chain(0..limit - 1).collect()
        } else {
            (0..if limit == 1 { 1 } else { limit - 1 }).collect()
        };
        loop {
            if cancelled() {
                return Err(finish_line_temporary(
                    "finish-line-runner-queue-cancelled",
                    "contained runner queue was cancelled before execution",
                ));
            }
            for index in &slots {
                // Stable lock inodes are never unlinked, including after a crash.
                let mut file = OpenOptions::new()
                    .read(true)
                    .write(true)
                    .create(true)
                    .truncate(false)
                    .mode(PRIVATE_MODE)
                    .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
                    .open(directory.join(format!("runner-{index}.lock")))
                    .map_err(|_| unavailable())?;
                verify_private_regular(&file, "finish-line-resource-untrusted")?;
                if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
                    let mut previous = String::new();
                    Read::by_ref(&mut file)
                        .take(1025)
                        .read_to_string(&mut previous)
                        .map_err(|_| unavailable())?;
                    // An interrupted metadata write precedes unit creation, so a
                    // malformed record with a free kernel lock is reclaimable.
                    if let Ok(owner) = serde_json::from_str::<SlotOwner>(&previous)
                        && let Some(previous_unit) = owner.unit
                    {
                        validate_contained_unit_name(&previous_unit)?;
                        if !quiescent(&previous_unit)? {
                            if notice.elapsed() >= Duration::from_secs(5) {
                                eprintln!(
                                    "contained runner queued: slot={index} pid={} since={} (unit draining)",
                                    owner.pid, owner.since
                                );
                            }
                            continue; // dead supervisor, but the unit is still alive/draining
                        }
                    }
                    file.set_len(0).map_err(|_| unavailable())?;
                    file.seek(SeekFrom::Start(0)).map_err(|_| unavailable())?;
                    let owner = SlotOwner {
                        pid: std::process::id(),
                        since: jiff::Timestamp::now().to_string(),
                        unit: unit.map(str::to_owned),
                    };
                    serde_json::to_writer(&mut file, &owner).map_err(|_| unavailable())?;
                    return Ok(Self { file, limit });
                }
                if io::Error::last_os_error().kind() != io::ErrorKind::WouldBlock {
                    return Err(unavailable());
                }
                if notice.elapsed() >= Duration::from_secs(5) {
                    let mut owner = String::new();
                    let _ = file.take(1025).read_to_string(&mut owner);
                    if let Ok(owner) = serde_json::from_str::<SlotOwner>(&owner)
                        && owner.since.parse::<jiff::Timestamp>().is_ok()
                    {
                        eprintln!(
                            "contained runner queued: slot={index} pid={} since={}",
                            owner.pid, owner.since
                        );
                    }
                }
            }
            if notice.elapsed() >= Duration::from_secs(5) {
                notice = Instant::now();
            }
            if started.elapsed() >= timeout {
                return Err(finish_line_temporary(
                    "finish-line-runner-queue-timeout",
                    "contained runner queue timed out; retry after an active runner finishes",
                ));
            }
            thread::sleep(Duration::from_millis(25));
        }
    }
}

impl Drop for RunnerPermit {
    fn drop(&mut self) {
        let _ = unsafe { libc::flock(self.file.as_raw_fd(), libc::LOCK_UN) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn queued_outer_commands_leave_a_reserved_gate_runner_slot() {
        let directory = tempfile::tempdir().unwrap();
        let _outer = RunnerPermit::acquire_in(
            directory.path(),
            2,
            false,
            Duration::from_millis(100),
            None,
            |_| Ok(true),
            || false,
        )
        .unwrap();
        assert!(
            RunnerPermit::acquire_in(
                directory.path(),
                2,
                false,
                Duration::from_millis(30),
                None,
                |_| Ok(true),
                || false,
            )
            .is_err(),
            "queued outer commands must not occupy the admitted gate's last runner slot"
        );
        let child = RunnerPermit::acquire_in(
            directory.path(),
            2,
            true,
            Duration::from_millis(100),
            None,
            |_| Ok(true),
            || false,
        )
        .unwrap();
        assert!(
            RunnerPermit::acquire_in(
                directory.path(),
                2,
                true,
                Duration::from_millis(30),
                None,
                |_| Ok(true),
                || false,
            )
            .is_err(),
            "the total runner cap includes the outer gate and child"
        );
        drop(child);
        assert!(
            RunnerPermit::acquire_in(
                directory.path(),
                2,
                true,
                Duration::from_millis(100),
                None,
                |_| Ok(true),
                || false,
            )
            .is_ok()
        );
    }

    #[test]
    fn resource_scheduler_never_exceeds_runner_limit() {
        let directory = tempfile::tempdir().unwrap();
        let active = AtomicUsize::new(0);
        let peak = AtomicUsize::new(0);
        thread::scope(|scope| {
            for _ in 0..8 {
                scope.spawn(|| {
                    let _permit = RunnerPermit::acquire_in(
                        directory.path(),
                        2,
                        true,
                        Duration::from_secs(5),
                        None,
                        |_| Ok(true),
                        || false,
                    )
                    .unwrap();
                    let count = active.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(count, Ordering::SeqCst);
                    thread::sleep(Duration::from_millis(30));
                    active.fetch_sub(1, Ordering::SeqCst);
                });
            }
        });
        assert_eq!(peak.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn stale_runner_metadata_does_not_hold_a_slot() {
        let directory = tempfile::tempdir().unwrap();
        fs::write(
            directory.path().join("runner-0.lock"),
            r#"{"pid":999999,"since":"1970-01-01T00:00:00Z","unit":null}"#,
        )
        .unwrap();
        fs::set_permissions(
            directory.path().join("runner-0.lock"),
            fs::Permissions::from_mode(PRIVATE_MODE),
        )
        .unwrap();
        let permit = RunnerPermit::acquire_in(
            directory.path(),
            1,
            true,
            Duration::from_millis(100),
            None,
            |_| Ok(true),
            || false,
        )
        .unwrap();
        assert!(
            RunnerPermit::acquire_in(
                directory.path(),
                1,
                true,
                Duration::from_millis(30),
                None,
                |_| Ok(true),
                || false
            )
            .is_err()
        );
        drop(permit);
        assert!(
            RunnerPermit::acquire_in(
                directory.path(),
                1,
                true,
                Duration::from_millis(100),
                None,
                |_| Ok(true),
                || false
            )
            .is_ok()
        );
    }

    #[test]
    fn interrupted_runner_metadata_write_is_recovered() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("runner-0.lock");
        fs::write(&path, r#"{"pid":"#).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(PRIVATE_MODE)).unwrap();
        let _permit = RunnerPermit::acquire_in(
            directory.path(),
            1,
            true,
            Duration::from_millis(100),
            None,
            |_| panic!("partial metadata cannot name a launched unit"),
            || false,
        )
        .unwrap();
        let owner: SlotOwner = serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
        assert_eq!(owner.pid, std::process::id());
    }

    #[test]
    fn crashed_supervisor_slot_waits_for_unit_quiescence() {
        let directory = tempfile::tempdir().unwrap();
        let unit = format!("nils-finish-line-{}", "a".repeat(32));
        let permit = RunnerPermit::acquire_in(
            directory.path(),
            1,
            true,
            Duration::from_millis(100),
            Some(&unit),
            |_| Ok(true),
            || false,
        )
        .unwrap();
        drop(permit); // supervisor disappeared; simulated service remains live
        assert!(
            RunnerPermit::acquire_in(
                directory.path(),
                1,
                true,
                Duration::from_millis(30),
                None,
                |_| Ok(false),
                || false
            )
            .is_err()
        );
        assert!(
            RunnerPermit::acquire_in(
                directory.path(),
                1,
                true,
                Duration::from_millis(100),
                None,
                |_| Ok(true),
                || false
            )
            .is_ok()
        );
    }

    #[test]
    fn cancelled_waiter_never_starts_a_runner() {
        let directory = tempfile::tempdir().unwrap();
        let error = RunnerPermit::acquire_in(
            directory.path(),
            1,
            true,
            Duration::from_secs(5),
            None,
            |_| Ok(true),
            || true,
        )
        .err()
        .unwrap();
        assert_eq!(error.code, "finish-line-runner-queue-cancelled");
    }

    #[test]
    fn environment_uses_the_resolved_limit_and_clamps_compilation() {
        let mut environment = BTreeMap::from([("CARGO_BUILD_JOBS".into(), "64".into())]);
        bound_environment(&mut environment, 1).unwrap();
        for name in [
            "CARGO_BUILD_JOBS",
            "NEXTEST_TEST_THREADS",
            "RUST_TEST_THREADS",
        ] {
            assert_eq!(environment[name], "1");
        }
        environment.insert("CARGO_BUILD_JOBS".into(), "64".into());
        environment.insert("NEXTEST_TEST_THREADS".into(), "32".into());
        bound_environment(&mut environment, 2).unwrap();
        assert_eq!(environment["CARGO_BUILD_JOBS"], "2");
        assert_eq!(environment["NEXTEST_TEST_THREADS"], "2");
        assert_eq!(environment["RUST_TEST_THREADS"], "1");
        assert_eq!(environment["NILS_CLI_RUNNER_MAX"], "2");
        assert_eq!(environment["NILS_CLI_CONTAINED_RUNNER_ACTIVE"], "1");
    }

    #[test]
    fn default_scheduler_limit_has_a_safe_floor_and_ceiling() {
        assert_eq!(default_limit(1, 0), 1);
        assert_eq!(default_limit(64, 64 * 1024 * 1024 * 1024), 2);
        assert_eq!(default_limit(8, 4 * 1024 * 1024 * 1024), 1);
    }
}
