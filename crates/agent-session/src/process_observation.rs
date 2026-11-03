use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Snapshot {
    kind: u8,
    state: u8,
    pid: libc::pid_t,
    start_time: u64,
    observed_at: u64,
    flags: u8,
}

fn kind_code(value: &str) -> u8 {
    match value {
        "identity" => 1,
        "process group" => 2,
        "session" => 3,
        "cgroup" => 4,
        "tmux verification" => 5,
        _ => unreachable!(),
    }
}
fn kind_name(value: u8) -> &'static str {
    match value {
        1 => "identity",
        2 => "process group",
        3 => "session",
        4 => "cgroup",
        5 => "tmux verification",
        _ => unreachable!(),
    }
}
fn state_code(value: &str) -> u8 {
    match value {
        "present" => 1,
        "unreadable" => 2,
        "absent" => 3,
        "boot identity unreadable" => 4,
        "path unavailable" => 5,
        "unknown (no verified mount provenance)" => 6,
        "metadata unreadable" => 7,
        "identity mismatch" => 8,
        "events unreadable" => 9,
        "events invalid" => 10,
        "stopped" => 11,
        _ => unreachable!(),
    }
}
fn state_name(value: u8) -> &'static str {
    match value {
        1 => "present",
        2 => "unreadable",
        3 => "absent",
        4 => "boot identity unreadable",
        5 => "path unavailable",
        6 => "unknown (no verified mount provenance)",
        7 => "metadata unreadable",
        8 => "identity mismatch",
        9 => "events unreadable",
        10 => "events invalid",
        11 => "stopped",
        _ => unreachable!(),
    }
}
impl Snapshot {
    fn describe(self) -> String {
        let identity = if self.flags & 1 == 0 {
            String::new()
        } else if self.flags & 2 != 0 {
            format!(", pid {}, start time {}", self.pid, self.start_time)
        } else {
            format!(", id {}", self.pid)
        };
        format!(
            "{}: {}{}",
            kind_name(self.kind),
            state_name(self.state),
            identity
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Observation {
    primary: Snapshot,
    boundary_id: Option<libc::pid_t>,
    later: [Option<Snapshot>; 2],
}

impl Observation {
    pub(super) fn new(
        kind: &'static str,
        state: &'static str,
        pid: Option<libc::pid_t>,
        start_time: Option<u64>,
    ) -> Self {
        Self {
            primary: Snapshot {
                kind: kind_code(kind),
                state: state_code(state),
                pid: pid.unwrap_or_default(),
                start_time: start_time.unwrap_or_default(),
                observed_at: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(),
                flags: u8::from(pid.is_some()) | (u8::from(start_time.is_some()) << 1),
            },
            boundary_id: None,
            later: [None; 2],
        }
    }
    pub(super) fn describe(self) -> String {
        let boundary = self
            .boundary_id
            .map(|id| format!(", session {id}"))
            .unwrap_or_default();
        let mut text = format!(
            "{}{} (observed during verification at Unix time {})",
            self.primary.describe(),
            boundary,
            self.primary.observed_at
        );
        if self.primary.state == state_code("present") && matches!(self.primary.kind, 2 | 4) {
            text.push_str("; individual survivor identity was not determined");
        }
        for later in self.later.into_iter().flatten() {
            text.push_str(&format!(
                "; later {} (observed at Unix time {})",
                later.describe(),
                later.observed_at
            ));
        }
        text
    }
    pub(super) fn in_session(mut self, id: libc::pid_t) -> Self {
        self.boundary_id = Some(id);
        self
    }
    pub(super) fn followed_by(mut self, later: Observation) -> Self {
        if let Some(slot) = self.later.iter_mut().find(|slot| slot.is_none()) {
            *slot = Some(later.primary);
        }
        self
    }
}

type Probe = (ProcessGroupStatus, Option<Observation>);

fn result(
    status: ProcessGroupStatus,
    kind: &'static str,
    state: &'static str,
    pid: Option<libc::pid_t>,
    start: Option<u64>,
) -> Probe {
    (status, Some(Observation::new(kind, state, pid, start)))
}

pub(super) fn runtime(identity: &TmuxRuntimeIdentity) -> Probe {
    if runtime_is_from_prior_boot(identity) {
        return (ProcessGroupStatus::Stopped, None);
    }
    #[cfg(target_os = "linux")]
    if let Some(group) = identity.control_group.as_ref() {
        return cgroup(identity, group, Path::new("/sys/fs/cgroup"));
    }
    #[cfg(target_os = "linux")]
    if let Some(session) = identity.process_session_id
        && !identity.process_session_members.is_empty()
    {
        return session_probe(identity, session);
    }
    match identity.process_group_id {
        Some(group) => {
            let status = process_group_status(group);
            result(
                status,
                "process group",
                if status == ProcessGroupStatus::Running {
                    "present"
                } else if status == ProcessGroupStatus::Unknown {
                    "unreadable"
                } else {
                    "absent"
                },
                Some(group),
                None,
            )
        }
        None => (ProcessGroupStatus::Stopped, None),
    }
}

#[cfg(target_os = "linux")]
pub(super) fn session_probe(identity: &TmuxRuntimeIdentity, session: libc::pid_t) -> Probe {
    for captured in &identity.process_session_members {
        match read_linux_process_identity(captured.pid) {
            Ok(Some(current)) if current.start_time == captured.start_time && !current.zombie => {
                return result(
                    ProcessGroupStatus::Running,
                    "identity",
                    "present",
                    Some(captured.pid),
                    Some(captured.start_time),
                );
            }
            Ok(_) => {}
            Err(_) => {
                return result(
                    ProcessGroupStatus::Unknown,
                    "identity",
                    "unreadable",
                    Some(captured.pid),
                    Some(captured.start_time),
                );
            }
        }
    }
    match linux_process_session_members(session) {
        Ok(members) if members.is_empty() => (ProcessGroupStatus::Stopped, None),
        Ok(members) => (
            ProcessGroupStatus::Running,
            Some(
                Observation::new(
                    "session",
                    "present",
                    Some(members[0].pid),
                    Some(members[0].start_time),
                )
                .in_session(session),
            ),
        ),
        Err(_) => result(
            ProcessGroupStatus::Unknown,
            "session",
            "unreadable",
            Some(session),
            None,
        ),
    }
}

#[cfg(target_os = "linux")]
pub(super) fn cgroup(
    identity: &TmuxRuntimeIdentity,
    group: &TmuxControlGroupIdentity,
    root: &Path,
) -> Probe {
    let unknown = |state| result(ProcessGroupStatus::Unknown, "cgroup", state, None, None);
    if let Some(boot_id) = group.boot_id.as_deref() {
        match linux_boot_id() {
            Ok(current) if current == boot_id => {}
            Ok(_) => return (ProcessGroupStatus::Stopped, None),
            Err(_) => return unknown("boot identity unreadable"),
        }
    }
    for captured in identity
        .process_session_members
        .iter()
        .chain(&identity.control_group_members)
    {
        match read_linux_process_identity(captured.pid) {
            Ok(Some(current)) if current.start_time == captured.start_time && !current.zombie => {
                return result(
                    ProcessGroupStatus::Running,
                    "identity",
                    "present",
                    Some(captured.pid),
                    Some(captured.start_time),
                );
            }
            Ok(_) => {}
            Err(_) => {
                return result(
                    ProcessGroupStatus::Unknown,
                    "identity",
                    "unreadable",
                    Some(captured.pid),
                    Some(captured.start_time),
                );
            }
        }
    }
    let path = match linux_control_group_full_path_at_root(Path::new(&group.path), root) {
        Ok(path) => path,
        Err(_) => return unknown("path unavailable"),
    };
    let metadata = match fs::metadata(&path) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            return if linux_cgroup_absence_is_trusted(identity, root) {
                (ProcessGroupStatus::Stopped, None)
            } else {
                unknown("unknown (no verified mount provenance)")
            };
        }
        Err(_) => return unknown("metadata unreadable"),
    };
    if !metadata.is_dir() || metadata.dev() != group.device || metadata.ino() != group.inode {
        return unknown("identity mismatch");
    }
    let events = match fs::read_to_string(path.join("cgroup.events")) {
        Ok(events) => events,
        Err(_) => return unknown("events unreadable"),
    };
    match events.lines().find_map(|line| {
        let (name, value) = line.split_once(' ')?;
        (name == "populated").then_some(value)
    }) {
        Some("0") => (ProcessGroupStatus::Stopped, None),
        Some("1") => result(ProcessGroupStatus::Running, "cgroup", "present", None, None),
        _ => unknown("events invalid"),
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn populated_cgroup_observation_reports_its_boundary() {
        let tmp = tempfile::TempDir::new().unwrap();
        let group_dir = tmp.path().join("fixture");
        fs::create_dir(&group_dir).unwrap();
        fs::write(group_dir.join("cgroup.events"), "populated 1\n").unwrap();
        let metadata = fs::metadata(&group_dir).unwrap();
        let group = TmuxControlGroupIdentity {
            path: "/fixture".to_owned(),
            device: metadata.dev(),
            inode: metadata.ino(),
            boot_id: None,
        };
        let identity: TmuxRuntimeIdentity = serde_json::from_value(json!({
            "session_id": "$91", "pane_id": "%91", "pane_pid": 2000000000
        }))
        .unwrap();
        let (status, evidence) = cgroup(&identity, &group, tmp.path());
        assert_eq!(status, ProcessGroupStatus::Running);
        assert!(evidence.unwrap().describe().contains("cgroup: present"));
    }
}
