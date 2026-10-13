//! Small private liveness index. The registry remains the transaction owner;
//! heartbeat readers never load retained mailbox bodies or take its flock.
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use nils_common::fs::{SECRET_FILE_MODE, write_atomic};
use serde::{Deserialize, Serialize};

use super::{
    CliContext, CliError, Registry, read_private_file, store_corrupt, store_unavailable,
    store_untrusted,
};

pub(super) fn observational_root(context: &CliContext) -> Result<PathBuf, CliError> {
    let root = context.state_dir.join("coordination");
    let metadata = fs::symlink_metadata(&root).map_err(|_| store_unavailable())?;
    if metadata.file_type().is_symlink()
        || !metadata.is_dir()
        || metadata.uid() != unsafe { libc::geteuid() }
    {
        return Err(store_untrusted());
    }
    let state = fs::canonicalize(&context.state_dir).map_err(|_| store_unavailable())?;
    if !fs::canonicalize(&root)
        .map_err(|_| store_unavailable())?
        .starts_with(state)
    {
        return Err(store_untrusted());
    }
    Ok(root)
}

const FILE: &str = "broker-state.json";
const VERSION: &str = "agent-session.broker-state.v1";

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub(super) struct Owner {
    pub incarnation: String,
    pub generation: u64,
    pub capability_digest: String,
    pub state: String,
    pub runtime_identity_digest: String,
}

impl Owner {
    fn same_authority(&self, other: &Self) -> bool {
        self.incarnation == other.incarnation
            && self.generation == other.generation
            && self.capability_digest == other.capability_digest
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub(super) struct State {
    schema_version: String,
    pub owners: BTreeMap<String, Owner>,
    pub completion_pending: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    completion_registry: Option<RegistryStamp>,
}

/// Atomic registry replacement changes this stamp, including for an older
/// writer that does not know about the index. It is observation, not authority.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
struct RegistryStamp {
    device: u64,
    inode: u64,
    bytes: u64,
    modified_seconds: i64,
    modified_nanoseconds: i64,
}

fn registry_stamp(path: &Path) -> Result<Option<RegistryStamp>, CliError> {
    let file = match fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(store_unavailable()),
    };
    let metadata = file.metadata().map_err(|_| store_unavailable())?;
    if !metadata.is_file()
        || metadata.len() > super::MAX_REGISTRY_BYTES
        || metadata.mode() & 0o077 != 0
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.nlink() != 1
    {
        return Err(store_untrusted());
    }
    Ok(Some(RegistryStamp {
        device: metadata.dev(),
        inode: metadata.ino(),
        bytes: metadata.len(),
        modified_seconds: metadata.mtime(),
        modified_nanoseconds: metadata.mtime_nsec(),
    }))
}

impl State {
    pub fn from_registry(registry: &Registry) -> Self {
        Self {
            schema_version: VERSION.into(),
            owners: registry
                .brokers
                .iter()
                .filter(|(_, broker)| {
                    matches!(broker.state.as_str(), "starting" | "recovering" | "ready")
                        && !broker.capability_digest.is_empty()
                })
                .map(|(id, broker)| {
                    (
                        id.clone(),
                        Owner {
                            incarnation: broker.incarnation.clone(),
                            generation: broker.generation,
                            capability_digest: broker.capability_digest.clone(),
                            state: broker.state.clone(),
                            runtime_identity_digest: broker.runtime_identity_digest.clone(),
                        },
                    )
                })
                .collect(),
            completion_pending: !registry.completion_events.is_empty(),
            completion_registry: None,
        }
    }
}

fn read(path: &Path) -> Result<Option<State>, CliError> {
    let bytes = match read_private_file(path, super::MAX_REGISTRY_BYTES) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(store_unavailable()),
    };
    let state: State = serde_json::from_slice(&bytes).map_err(|_| store_corrupt())?;
    if state.schema_version != VERSION {
        return Err(store_corrupt());
    }
    Ok(Some(state))
}

pub(super) fn load(context: &CliContext) -> Result<State, CliError> {
    if let Some(state) = read(&observational_root(context)?.join(FILE))? {
        return Ok(state);
    }
    // Existing pre-index owners remain readable across upgrade. Newly provisioned
    // owners always publish the index before the held-launch gate opens.
    Ok(State::from_registry(&super::read_registry_snapshot(
        context,
    )?))
}

/// Called only after publishing the heartbeat. A false bit is trusted only for
/// the committed registry file observed by its writer or a locked queue drain.
pub(super) fn completion_drain_needed(context: &CliContext) -> Result<bool, CliError> {
    let state = load(context)?;
    Ok(state.completion_pending
        || state.completion_registry.is_none()
        || state.completion_registry
            != registry_stamp(&observational_root(context)?.join(super::REGISTRY_FILE))?)
}

/// Caller holds the registry flock and has observed the committed queue. Keep
/// existing heartbeat authority (including failed-publication fences) intact.
pub(super) fn refresh_completion(
    context: &CliContext,
    registry: &Registry,
) -> Result<(), CliError> {
    let root = observational_root(context)?;
    let path = root.join(FILE);
    let previous = read(&path)?;
    let mut state = previous
        .clone()
        .unwrap_or_else(|| State::from_registry(registry));
    state.completion_pending = !registry.completion_events.is_empty();
    state.completion_registry = registry_stamp(&root.join(super::REGISTRY_FILE))?;
    publish(&path, &state, previous.as_ref())
}

fn publish(path: &Path, state: &State, previous: Option<&State>) -> Result<(), CliError> {
    if previous == Some(state) {
        return Ok(());
    }
    let bytes = serde_json::to_vec(state).map_err(|_| store_corrupt())?;
    if bytes.len() as u64 > super::MAX_REGISTRY_BYTES {
        return Err(store_unavailable());
    }
    write_atomic(path, &bytes, SECRET_FILE_MODE).map_err(|_| store_unavailable())
}

/// Global registry flock is held by the caller. Reductions become visible before
/// commit; additions only after commit. A failed write can deny liveness, never
/// retain revoked authority or authorize an uncommitted owner. Pending completion
/// is similarly set before commit and cleared afterwards.
pub(super) fn persist(
    registry_path: &Path,
    registry: &Registry,
    bytes: &[u8],
) -> Result<(), CliError> {
    persist_with_commit(registry_path, registry, bytes, |bytes| {
        write_atomic(registry_path, bytes, SECRET_FILE_MODE).map_err(|_| store_unavailable())
    })
}

fn persist_with_commit(
    registry_path: &Path,
    registry: &Registry,
    bytes: &[u8],
    commit: impl FnOnce(&[u8]) -> Result<(), CliError>,
) -> Result<(), CliError> {
    let path = registry_path.with_file_name(FILE);
    let mut previous = read(&path)?;
    if previous.is_none() {
        // An observational writer can be the first index publisher after an
        // upgrade. Seed from committed state, not from its pending mutation:
        // an empty fence would temporarily revoke every unchanged old owner.
        match read_private_file(registry_path, super::MAX_REGISTRY_BYTES) {
            Ok(bytes) => {
                let committed: Registry =
                    serde_json::from_slice(&bytes).map_err(|_| store_corrupt())?;
                let mut state = State::from_registry(&committed);
                state.completion_registry = registry_stamp(registry_path)?;
                publish(&path, &state, None)?;
                previous = Some(state);
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(_) => return Err(store_unavailable()),
        }
    }
    let mut next = State::from_registry(registry);
    let mut fenced = previous.clone().unwrap_or_else(|| State {
        schema_version: VERSION.into(),
        owners: BTreeMap::new(),
        completion_pending: false,
        completion_registry: None,
    });
    fenced.owners.retain(|id, owner| {
        next.owners
            .get(id)
            .is_some_and(|next| owner.same_authority(next))
    });
    fenced.completion_pending |= next.completion_pending;
    publish(&path, &fenced, previous.as_ref())?;
    commit(bytes)?;
    next.completion_registry = registry_stamp(registry_path)?;
    publish(&path, &next, Some(&fenced))
}

pub(super) fn initialize(registry_path: &Path, registry: &Registry) -> Result<(), CliError> {
    let path = registry_path.with_file_name(FILE);
    // Only a mutating registry owner installs the pre-index projection. Read-only
    // observation never migrates or rewrites either file.
    if read(&path)?.is_none() {
        let mut state = State::from_registry(registry);
        state.completion_registry = registry_stamp(registry_path)?;
        publish(&path, &state, None)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::{assert_eq, assert_ne};
    use serde_json::json;

    fn seed_registry() -> Registry {
        serde_json::from_value(json!({"brokers": {"fixture": {
            "session_id": "fixture", "incarnation": "inc", "generation": 1,
            "capability_digest": "digest", "state": "ready",
            "heartbeat_at": "", "heartbeat_epoch": 0
        }}}))
        .unwrap()
    }

    #[test]
    fn failed_registry_commit_removes_revoked_authority_and_never_adds_new_authority() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("registry.json");
        let mut registry = seed_registry();
        persist(&path, &registry, &serde_json::to_vec(&registry).unwrap()).unwrap();
        assert!(
            read(&path.with_file_name(FILE))
                .unwrap()
                .unwrap()
                .owners
                .contains_key("fixture")
        );
        // Only disposable fixture state is changed. A directory blocks atomic
        // registry publication after the revocation fence has been persisted.
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        registry.brokers.get_mut("fixture").unwrap().state = "stopped".into();
        let mut replacement = registry.brokers["fixture"].clone();
        replacement.incarnation = "replacement".into();
        replacement.state = "ready".into();
        registry.brokers.insert("new-owner".into(), replacement);
        assert!(persist(&path, &registry, &serde_json::to_vec(&registry).unwrap()).is_err());
        assert!(
            read(&path.with_file_name(FILE))
                .unwrap()
                .unwrap()
                .owners
                .is_empty()
        );
    }

    #[test]
    fn first_index_publication_preserves_committed_owner_before_a_failed_addition() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("registry.json");
        let mut registry = seed_registry();
        write_atomic(
            &path,
            &serde_json::to_vec(&registry).unwrap(),
            SECRET_FILE_MODE,
        )
        .unwrap();
        let replacement = registry.brokers["fixture"].clone();
        registry.brokers.insert("new-owner".into(), replacement);
        let result = persist_with_commit(&path, &registry, b"unused", |_| {
            let staged = read(&path.with_file_name(FILE)).unwrap().unwrap();
            assert!(
                staged.owners.contains_key("fixture"),
                "unchanged owner was revoked during migration"
            );
            assert!(
                !staged.owners.contains_key("new-owner"),
                "uncommitted addition was authorized"
            );
            Err(store_unavailable())
        });
        assert!(result.is_err());
        let staged = read(&path.with_file_name(FILE)).unwrap().unwrap();
        assert_eq!(staged.owners.len(), 1);
        assert!(staged.owners.contains_key("fixture"));
    }

    #[test]
    fn mailbox_commit_refreshes_completion_stamp_without_changing_heartbeat_authority() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("registry.json");
        let mut registry = seed_registry();
        persist(&path, &registry, &serde_json::to_vec(&registry).unwrap()).unwrap();
        let state_path = path.with_file_name(FILE);
        let before = read(&state_path).unwrap().unwrap();
        registry.fingerprint_epoch = 2;
        persist(&path, &registry, &serde_json::to_vec(&registry).unwrap()).unwrap();
        let after = read(&state_path).unwrap().unwrap();
        assert_eq!(after.owners, before.owners);
        assert_ne!(after.completion_registry, before.completion_registry);
        assert_eq!(after.completion_registry, registry_stamp(&path).unwrap());
    }

    #[test]
    fn unstamped_legacy_index_observes_empty_queue_once_and_preserves_authority() {
        let tmp = tempfile::tempdir().unwrap();
        let context = CliContext {
            state_dir: tmp.path().into(),
            host: None,
        };
        let root = tmp.path().join("coordination");
        fs::create_dir(&root).unwrap();
        let registry = seed_registry();
        let path = root.join(super::super::REGISTRY_FILE);
        persist(&path, &registry, &serde_json::to_vec(&registry).unwrap()).unwrap();
        let before = read(&root.join(FILE)).unwrap().unwrap();
        // An existing index from before the stamp field is additive migration
        // state, not corruption. Its false bit needs one committed observation.
        let mut unstamped = serde_json::to_value(&before).unwrap();
        unstamped
            .as_object_mut()
            .unwrap()
            .remove("completion_registry");
        write_atomic(
            &root.join(FILE),
            &serde_json::to_vec(&unstamped).unwrap(),
            SECRET_FILE_MODE,
        )
        .unwrap();
        assert!(completion_drain_needed(&context).unwrap());
        refresh_completion(&context, &registry).unwrap();
        let held = super::super::lock_private_store(&context, "registry.lock").unwrap();
        assert!(!completion_drain_needed(&context).unwrap());
        assert_eq!(load(&context).unwrap().owners, before.owners);
        drop(held);
    }

    #[test]
    fn empty_completion_refresh_keeps_failed_commit_authority_fences() {
        let tmp = tempfile::tempdir().unwrap();
        let context = CliContext {
            state_dir: tmp.path().into(),
            host: None,
        };
        let root = tmp.path().join("coordination");
        fs::create_dir(&root).unwrap();
        let path = root.join(super::super::REGISTRY_FILE);
        let committed = seed_registry();
        persist(&path, &committed, &serde_json::to_vec(&committed).unwrap()).unwrap();
        let mut revoked = committed.clone();
        revoked.brokers.clear();
        assert!(
            persist_with_commit(&path, &revoked, b"unused", |_| Err(store_unavailable())).is_err()
        );
        refresh_completion(&context, &committed).unwrap();
        assert!(load(&context).unwrap().owners.is_empty());
        assert!(!completion_drain_needed(&context).unwrap());
    }

    #[test]
    fn read_only_paths_preserve_the_coordination_directory_symlink_guard() {
        let tmp = tempfile::tempdir().unwrap();
        let context = CliContext {
            state_dir: tmp.path().join("state"),
            host: None,
        };
        std::fs::create_dir(&context.state_dir).unwrap();
        let outside = tmp.path().join("outside");
        std::fs::create_dir(&outside).unwrap();
        let mut registry = seed_registry();
        registry.schema_version = super::super::REGISTRY_VERSION.into();
        persist(
            &outside.join("registry.json"),
            &registry,
            &serde_json::to_vec(&registry).unwrap(),
        )
        .unwrap();
        std::os::unix::fs::symlink(&outside, context.state_dir.join("coordination")).unwrap();
        assert!(load(&context).is_err());
        assert!(super::super::public_summary_registry_snapshot(&context).is_none());
    }

    #[test]
    fn legacy_fallback_is_read_only_and_corrupt_index_never_falls_back() {
        let tmp = tempfile::tempdir().unwrap();
        let context = CliContext {
            state_dir: tmp.path().to_path_buf(),
            host: None,
        };
        let root = tmp.path().join("coordination");
        std::fs::create_dir(&root).unwrap();
        let mut registry = seed_registry();
        registry.schema_version = super::super::REGISTRY_VERSION.into();
        write_atomic(
            &root.join("registry.json"),
            &serde_json::to_vec(&registry).unwrap(),
            SECRET_FILE_MODE,
        )
        .unwrap();
        assert!(load(&context).unwrap().owners.contains_key("fixture"));
        assert!(!root.join(FILE).exists());
        write_atomic(&root.join(FILE), b"{}", SECRET_FILE_MODE).unwrap();
        assert!(load(&context).is_err());
    }
}
