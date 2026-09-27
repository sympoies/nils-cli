//! Kernel-derived client identity for Linux crash recovery. Request JSON names
//! only a live ancestor PID; it cannot supply liveness or identity evidence.
#[cfg(target_os = "linux")]
use std::fs;
use std::fs::File;
use std::io::{self, Read};
#[cfg(target_os = "linux")]
use std::os::fd::AsRawFd;
#[cfg(target_os = "linux")]
use std::os::unix::fs::MetadataExt;

use serde::{Deserialize, Serialize};

use crate::error::HookError;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct OwnerProcess {
    pid: u32,
    start_ticks: u64,
    boot_id: String,
    namespace_dev: u64,
    namespace_ino: u64,
    uid: u32,
}

fn unavailable() -> HookError {
    HookError::data(
        "finish-line-owner-invalid",
        "finish-line owner must be a verifiable live same-user caller ancestor",
    )
}

#[cfg(target_os = "linux")]
fn context() -> Result<(String, u64, u64, u32), HookError> {
    let proc = File::open("/proc").map_err(|_| unavailable())?;
    let mut stat: libc::statfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::fstatfs(proc.as_raw_fd(), &mut stat) } != 0
        || stat.f_type != libc::PROC_SUPER_MAGIC
    {
        return Err(unavailable());
    }
    let boot = bounded_read("/proc/sys/kernel/random/boot_id").map_err(|_| unavailable())?;
    let boot = boot.trim();
    let parsed = uuid::Uuid::parse_str(boot).map_err(|_| unavailable())?;
    if parsed.hyphenated().to_string() != boot {
        return Err(unavailable());
    }
    let namespace = fs::metadata("/proc/self/ns/pid").map_err(|_| unavailable())?;
    Ok(
        (boot.to_string(), namespace.dev(), namespace.ino(), unsafe {
            libc::geteuid()
        }),
    )
}

fn bounded_read(path: &str) -> io::Result<String> {
    let mut text = String::new();
    File::open(path)?.take(4097).read_to_string(&mut text)?;
    if text.len() > 4096 {
        return Err(io::Error::from(io::ErrorKind::InvalidData));
    }
    Ok(text)
}

fn process_stat(pid: u32) -> io::Result<(u32, u64, char)> {
    let text = bounded_read(&format!("/proc/{pid}/stat"))?;
    let (_, fields) = text.rsplit_once(") ").ok_or(io::ErrorKind::InvalidData)?;
    let fields: Vec<_> = fields.split_whitespace().collect();
    let state = fields
        .first()
        .and_then(|value| value.chars().next())
        .ok_or(io::ErrorKind::InvalidData)?;
    let parent = fields
        .get(1)
        .and_then(|value| value.parse().ok())
        .ok_or(io::ErrorKind::InvalidData)?;
    let start = fields
        .get(19)
        .and_then(|value| value.parse().ok())
        .filter(|value| *value > 0)
        .ok_or(io::ErrorKind::InvalidData)?;
    Ok((parent, start, state))
}

#[cfg(target_os = "linux")]
pub(super) fn authenticate(pid: u32) -> Result<OwnerProcess, HookError> {
    let (boot_id, namespace_dev, namespace_ino, uid) = context()?;
    if pid == 0 || pid == std::process::id() {
        return Err(unavailable());
    }
    let mut ancestor = std::process::id();
    let mut found = false;
    for _ in 0..64 {
        ancestor = process_stat(ancestor).map_err(|_| unavailable())?.0;
        if ancestor == pid {
            found = true;
            break;
        }
        if ancestor == 0 {
            break;
        }
    }
    if !found {
        return Err(unavailable());
    }
    let before = process_stat(pid).map_err(|_| unavailable())?;
    if matches!(before.2, 'Z' | 'X' | 'x') {
        return Err(unavailable());
    }
    let namespace = fs::metadata(format!("/proc/{pid}/ns/pid")).map_err(|_| unavailable())?;
    let status = bounded_read(&format!("/proc/{pid}/status")).map_err(|_| unavailable())?;
    let uids = status
        .lines()
        .find_map(|line| line.strip_prefix("Uid:"))
        .ok_or_else(unavailable)?;
    let uids = uids
        .split_whitespace()
        .map(str::parse::<u32>)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| unavailable())?;
    let after = process_stat(pid).map_err(|_| unavailable())?;
    if uids.len() != 4
        || uids.iter().any(|value| *value != uid)
        || namespace.dev() != namespace_dev
        || namespace.ino() != namespace_ino
        || (after.0, after.1) != (before.0, before.1)
        || matches!(after.2, 'Z' | 'X' | 'x')
    {
        return Err(unavailable());
    }
    Ok(OwnerProcess {
        pid,
        start_ticks: before.1,
        boot_id,
        namespace_dev,
        namespace_ino,
        uid,
    })
}

#[cfg(target_os = "linux")]
pub(super) fn proven_dead(owner: &OwnerProcess) -> bool {
    let Ok((boot, dev, ino, uid)) = context() else {
        return false;
    };
    if owner.boot_id != boot
        || owner.namespace_dev != dev
        || owner.namespace_ino != ino
        || owner.uid != uid
    {
        return false;
    }
    match process_stat(owner.pid) {
        Err(error) => error.kind() == io::ErrorKind::NotFound,
        Ok(before) if before.1 != owner.start_ticks => true,
        Ok(before) if matches!(before.2, 'Z' | 'X' | 'x') => {
            process_stat(owner.pid).is_ok_and(|after| after == before)
        }
        _ => false,
    }
}

pub(super) fn authenticate_bound(owner: &OwnerProcess) -> Result<(), HookError> {
    if authenticate(owner.pid)? != *owner {
        return Err(unavailable());
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
pub(super) fn authenticate(_pid: u32) -> Result<OwnerProcess, HookError> {
    Err(unavailable())
}
#[cfg(not(target_os = "linux"))]
pub(super) fn proven_dead(_owner: &OwnerProcess) -> bool {
    false
}

// The v1 main file has strict released readers. A separate private sidecar
// preserves rollback and binds ownership to the exact capability incarnation.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct OwnerBinding {
    session_key: String,
    capability_digest: String,
    incarnation: u64,
    owner: OwnerProcess,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct OwnerRegistry {
    schema_version: String,
    repo_digest: String,
    bindings: std::collections::BTreeMap<String, OwnerBinding>,
}

impl OwnerRegistry {
    fn new(repo: &str) -> Self {
        Self {
            schema_version: "agent-hook.finish-line.owners.v1".to_string(),
            repo_digest: repo.to_string(),
            bindings: Default::default(),
        }
    }
}

pub(super) fn load(
    path: &std::path::Path,
    state: &mut super::State,
) -> Result<OwnerRegistry, HookError> {
    use std::fs::OpenOptions;
    use std::os::unix::fs::OpenOptionsExt;
    let file = match OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(path.with_extension("owners"))
    {
        Ok(file) => Some(file),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(_) => return Err(unavailable()),
    };
    let registry = if let Some(file) = file {
        super::verify_private_regular(&file, "finish-line-owner-state-untrusted")?;
        let mut bytes = Vec::new();
        file.take(super::STATE_MAX_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| unavailable())?;
        if bytes.len() as u64 > super::STATE_MAX_BYTES {
            return Err(unavailable());
        }
        let registry: OwnerRegistry = serde_json::from_slice(&bytes).map_err(|_| unavailable())?;
        if registry.schema_version != "agent-hook.finish-line.owners.v1"
            || registry.repo_digest != state.repo_digest
            || registry.bindings.len() > super::MAX_SESSIONS * 2
        {
            return Err(unavailable());
        }
        if registry.bindings.values().any(|binding| {
            let owner = &binding.owner;
            owner.pid == 0
                || owner.pid > i32::MAX as u32
                || owner.start_ticks == 0
                || owner.namespace_ino == 0
                || binding.incarnation == 0
                || uuid::Uuid::parse_str(&owner.boot_id).is_err()
        }) {
            return Err(unavailable());
        }
        registry
    } else {
        OwnerRegistry::new(&state.repo_digest)
    };
    let mut active = OwnerRegistry::new(&state.repo_digest);
    for (key, session) in &mut state.sessions {
        let (Some(digest), Some(incarnation)) = (
            session.runner_capability_digest.as_ref(),
            session.runner_capability_incarnation,
        ) else {
            continue;
        };
        let binding_key = super::released_session_key(key, digest);
        if let Some(binding) = registry.bindings.get(&binding_key)
            && binding.session_key == *key
            && binding.capability_digest == *digest
            && binding.incarnation == incarnation
        {
            session.owner_process = Some(binding.owner.clone());
            active.bindings.insert(binding_key, binding.clone());
        }
    }
    Ok(active)
}

pub(super) fn save(
    path: &std::path::Path,
    state: &super::State,
    initial: &OwnerRegistry,
) -> Result<(), HookError> {
    // Retain current-on-disk AND prospective identities until the primary save.
    // Interrupted recovery can still authenticate the old main incarnation and
    // retry. The next Store::open compacts entries against the committed main file.
    let mut registry = initial.clone();
    for (key, session) in &state.sessions {
        if let (Some(owner), Some(digest), Some(incarnation)) = (
            &session.owner_process,
            &session.runner_capability_digest,
            session.runner_capability_incarnation,
        ) {
            registry.bindings.insert(
                super::released_session_key(key, digest),
                OwnerBinding {
                    session_key: key.clone(),
                    capability_digest: digest.clone(),
                    incarnation,
                    owner: owner.clone(),
                },
            );
        }
    }
    if registry.bindings.len() > super::MAX_SESSIONS * 2 {
        return Err(unavailable());
    }
    let bytes = serde_json::to_vec(&registry).map_err(|_| unavailable())?;
    if bytes.len() as u64 > super::STATE_MAX_BYTES {
        return Err(unavailable());
    }
    super::write_state_atomic(&path.with_extension("owners"), &bytes)
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn crash_recovery_interrupted_sidecar_save_binds_only_committed_incarnation() {
        let dir = tempfile::TempDir::new().expect("sidecar fixture");
        let path = dir.path().join("repository.json");
        let owner = authenticate(unsafe { libc::getppid() as u32 }).expect("parent owner");
        let mut old = super::super::State::new("sha256:repository");
        old.sessions.insert(
            "session".to_string(),
            super::super::SessionState {
                owner_process: Some(owner.clone()),
                runner_capability_digest: Some("old-digest".to_string()),
                runner_capability_incarnation: Some(1),
                ..Default::default()
            },
        );
        save(&path, &old, &OwnerRegistry::new(&old.repo_digest)).expect("original sidecar");
        let initial = load(&path, &mut old).expect("hydrate committed old owner");
        let mut next = super::super::State::new(&old.repo_digest);
        let mut new_owner = owner.clone();
        new_owner.start_ticks += 1;
        next.sessions.insert(
            "session".to_string(),
            super::super::SessionState {
                owner_process: Some(new_owner.clone()),
                runner_capability_digest: Some("new-digest".to_string()),
                runner_capability_incarnation: Some(2),
                ..Default::default()
            },
        );
        save(&path, &next, &initial).expect("sidecar before primary save");
        old.sessions.get_mut("session").expect("old").owner_process = None;
        load(&path, &mut old).expect("interrupted main save retains old binding");
        assert_eq!(old.sessions["session"].owner_process.as_ref(), Some(&owner));
        next.sessions
            .get_mut("session")
            .expect("next")
            .owner_process = None;
        let committed = load(&path, &mut next).expect("committed main selects new binding");
        assert_eq!(
            next.sessions["session"].owner_process.as_ref(),
            Some(&new_owner)
        );
        save(&path, &next, &committed).expect("compact retired binding");
        let registry: OwnerRegistry = serde_json::from_slice(
            &std::fs::read(path.with_extension("owners")).expect("registry"),
        )
        .expect("JSON");
        assert_eq!(registry.bindings.len(), 1);
        old.sessions.get_mut("session").expect("old").owner_process = None;
        load(&path, &mut old).expect("stale digest treated as unbound ownership");
        assert!(old.sessions["session"].owner_process.is_none());
    }

    #[test]
    fn crash_recovery_identity_distinguishes_live_reused_and_ambiguous_owners() {
        let live = authenticate(unsafe { libc::getppid() as u32 }).expect("live caller ancestor");
        assert!(!proven_dead(&live));
        let mut reused = live.clone();
        reused.start_ticks += 1;
        assert!(
            proven_dead(&reused),
            "different start ticks cannot keep an old incarnation alive"
        );
        let mut missing = live.clone();
        missing.pid = i32::MAX as u32;
        assert!(proven_dead(&missing), "absent same-namespace PID is dead");
        for field in ["boot", "namespace", "uid"] {
            let mut uncertain = missing.clone();
            match field {
                "boot" => uncertain.boot_id = uuid::Uuid::new_v4().to_string(),
                "namespace" => uncertain.namespace_ino += 1,
                _ => uncertain.uid = uncertain.uid.wrapping_add(1),
            }
            assert!(
                !proven_dead(&uncertain),
                "unknown host identity must fail closed: {field}"
            );
        }
        assert_eq!(
            authenticate(0).expect_err("PID zero").code,
            "finish-line-owner-invalid"
        );
        assert_eq!(
            authenticate(std::process::id())
                .expect_err("short-lived invoker")
                .code,
            "finish-line-owner-invalid"
        );
        assert_eq!(
            authenticate(i32::MAX as u32)
                .expect_err("non-ancestor")
                .code,
            "finish-line-owner-invalid"
        );
    }
}
