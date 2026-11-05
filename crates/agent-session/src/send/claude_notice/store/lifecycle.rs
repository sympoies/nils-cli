//! One current live segment, immutable predecessors and permanent compact key indexes.
use super::*;
use std::collections::{BTreeMap, BTreeSet, HashSet};

mod archive;
pub(super) mod checkpoint;
mod sealed;

pub(super) const CHECKPOINT_INITIALIZED: &str = "notice checkpoint initialized after deadline";

const POINTER_SCHEMA: &str = "agent-session.claude-notice-store-current.v1";
const MANIFEST_SCHEMA: &str = "agent-session.claude-notice-store-manifest.v1";
const SEGMENT_SCHEMA: &str = "agent-session.claude-notice-segment.v1";
const LAYOUT_MARKER: &[u8] = b"{\"schema_version\":\"agent-session.claude-notice-layout.v1\"}\n";
const MAX_MANIFEST: usize = 256 * 1024;
const MAX_INDEX: usize = MAX_CLAIMS * 132;
pub(super) const MAX_SEGMENT_BYTES: usize = MAX_BYTES + MAX_RECORD;

#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct Reference {
    file: String,
    sha256: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Pointer {
    schema_version: String,
    manifest: Reference,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema_version: String,
    generation: u64,
    active: String,
    predecessor: Option<Reference>,
    archive: Reference,
    key_index: Reference,
    first_seen: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Header {
    schema_version: String,
    kind: String,
    generation: u64,
    previous_sha256: String,
    key_index_sha256: String,
}

impl Header {
    fn from_manifest(manifest: &Manifest) -> Self {
        Self {
            schema_version: SEGMENT_SCHEMA.into(),
            kind: "segment".into(),
            generation: manifest.generation,
            previous_sha256: manifest.archive.sha256.clone(),
            key_index_sha256: manifest.key_index.sha256.clone(),
        }
    }
}

struct Layout {
    active: String,
    tip: Option<Reference>,
    chain: Vec<(Reference, Manifest)>,
    keys: BTreeSet<String>,
    index: Option<pages::Index>,
    index_name: Option<String>,
}

pub(super) struct Storage {
    layout: Layout,
    _gate: File,
}

fn named(file: &str, prefix: &str, suffix: &str) -> bool {
    file.strip_prefix(prefix)
        .and_then(|s| s.strip_suffix(suffix))
        .is_some_and(|id| uuid::Uuid::parse_str(id).is_ok() && !id.contains(['/', '\\']))
}

pub(super) fn check_time(deadline: Instant) -> io::Result<()> {
    if Instant::now() >= deadline {
        Err(io::ErrorKind::TimedOut.into())
    } else {
        Ok(())
    }
}

fn read_private(parent: &Path, file: &str, cap: usize) -> io::Result<Vec<u8>> {
    runtime::private_read(&parent.join(file), cap).map_err(|_| invalid())
}

fn verified(parent: &Path, reference: &Reference, cap: usize) -> io::Result<Vec<u8>> {
    if !hash_valid(&reference.sha256) {
        return Err(invalid());
    }
    let bytes = read_private(parent, &reference.file, cap)?;
    if sha256(&bytes) != reference.sha256 {
        return Err(invalid());
    }
    Ok(bytes)
}

fn load_verified(
    parent: &Path,
    deadline: Instant,
    finish_pending_seal: bool,
) -> io::Result<Layout> {
    let initialized = match fs::symlink_metadata(parent.join("layout.json")) {
        Ok(_) => {
            let marker = read_private(parent, "layout.json", 4096)?;
            if marker != LAYOUT_MARKER && marker != checkpoint::MARKER {
                return Err(invalid());
            }
            true
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => false,
        Err(e) => return Err(e),
    };
    let pointer = match fs::symlink_metadata(parent.join("current.json")) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            if initialized && !checkpoint::empty(parent)? {
                return Err(invalid());
            }
            return Ok(Layout {
                active: "ledger.jsonl".into(),
                tip: None,
                chain: Vec::new(),
                keys: BTreeSet::new(),
                index: None,
                index_name: None,
            });
        }
        Err(e) => return Err(e),
        Ok(_) => serde_json::from_slice::<Pointer>(&read_private(parent, "current.json", 4096)?)?,
    };
    if pointer.schema_version != POINTER_SCHEMA {
        return Err(invalid());
    }
    let tip = pointer.manifest.clone();
    let mut next = Some(pointer.manifest);
    let mut chain: Vec<(Reference, Manifest)> = Vec::new();
    let mut visited = HashSet::new();
    let mut keys = BTreeSet::new();
    while let Some(reference) = next {
        check_time(deadline)?;
        if !named(&reference.file, "manifest-", ".json") || !visited.insert(reference.file.clone())
        {
            return Err(invalid());
        }
        let manifest: Manifest =
            serde_json::from_slice(&verified(parent, &reference, MAX_MANIFEST)?)?;
        if manifest.schema_version != MANIFEST_SCHEMA
            || manifest.generation == 0
            || !named(&manifest.active, "segment-", ".jsonl")
            || !(manifest.archive.file == "ledger.jsonl"
                || named(&manifest.archive.file, "segment-", ".jsonl"))
            || !hash_valid(&manifest.archive.sha256)
            || !named(&manifest.key_index.file, "keys-", ".txt")
            || manifest.first_seen.len() > MAX_CLAIMS
            || manifest.first_seen.iter().any(|(op, time)| {
                uuid::Uuid::parse_str(op).is_err() || time.parse::<jiff::Timestamp>().is_err()
            })
        {
            return Err(invalid());
        }
        // Check sealed bytes before trusting the index that represents them.
        sealed::verified(
            parent,
            &manifest.archive,
            if manifest.archive.file == "ledger.jsonl" {
                MAX_BYTES
            } else {
                MAX_SEGMENT_BYTES
            },
            finish_pending_seal && chain.is_empty(),
            deadline,
        )?;
        if let Some((_, child)) = chain.last()
            && (manifest.generation.checked_add(1) != Some(child.generation)
                || child.archive.file != manifest.active)
        {
            return Err(invalid());
        }
        let index = verified(parent, &manifest.key_index, MAX_INDEX)?;
        if index.last().is_some_and(|b| *b != b'\n') {
            return Err(invalid());
        }
        let mut local = BTreeSet::new();
        for line in index.split(|b| *b == b'\n').filter(|line| !line.is_empty()) {
            check_time(deadline)?;
            let entry = std::str::from_utf8(line).map_err(|_| invalid())?;
            if entry.len() != 65
                || !matches!(entry.as_bytes()[0], b'k' | b'o')
                || !hash_valid(&entry[1..])
                || !local.insert(entry.to_string())
            {
                return Err(invalid());
            }
        }
        keys.extend(local);
        next = manifest.predecessor.clone();
        if next.is_none() && (manifest.generation != 1 || manifest.archive.file != "ledger.jsonl") {
            return Err(invalid());
        }
        chain.push((reference, manifest));
    }
    let active = chain.first().ok_or_else(invalid)?.1.active.clone();
    Ok(Layout {
        active,
        tip: Some(tip),
        chain,
        keys,
        index: None,
        index_name: None,
    })
}

fn lock(file: &File, operation: libc::c_int, deadline: Instant) -> io::Result<()> {
    loop {
        check_time(deadline)?;
        // SAFETY: the owner retains the file descriptor throughout this operation.
        if unsafe { libc::flock(file.as_raw_fd(), operation | libc::LOCK_NB) } == 0 {
            return Ok(());
        }
        let e = io::Error::last_os_error();
        if e.kind() != io::ErrorKind::WouldBlock {
            return Err(e);
        }
        std::thread::sleep(
            Duration::from_millis(2).min(deadline.saturating_duration_since(Instant::now())),
        );
    }
}

fn private_file(
    parent: &Path,
    name: &str,
    write: bool,
    cap: usize,
    deadline: Instant,
) -> io::Result<File> {
    let path = parent.join(name);
    let mut options = OpenOptions::new();
    options.read(true).custom_flags(libc::O_NOFOLLOW);
    if write {
        options.append(true).mode(0o600);
        if name == "ledger.jsonl" {
            options.create(true);
        }
    }
    let file = options.open(&path)?;
    let meta = file.metadata()?;
    // SAFETY: geteuid has no preconditions.
    if !meta.is_file()
        || meta.uid() != unsafe { libc::geteuid() }
        || meta.mode() & 0o777 != 0o600
        || meta.len() > cap as u64
    {
        return Err(invalid());
    }
    lock(
        &file,
        if write { libc::LOCK_EX } else { libc::LOCK_SH },
        deadline,
    )?;
    let after = fs::symlink_metadata(path)?;
    if (meta.dev(), meta.ino()) != (after.dev(), after.ino()) {
        return Err(invalid());
    }
    Ok(file)
}

pub(super) fn open(state: &Path, write: bool, deadline: Instant) -> io::Result<Option<Ledger>> {
    let parent = state.join("history/claude-notices");
    if write {
        fs::create_dir_all(state.join("history"))?;
        runtime::private_dir(&parent).map_err(|_| invalid())?;
    } else if !parent.exists() {
        return Ok(None);
    }
    let dir = fs::symlink_metadata(&parent)?;
    // SAFETY: geteuid has no preconditions.
    if !dir.is_dir() || dir.uid() != unsafe { libc::geteuid() } || dir.mode() & 0o777 != 0o700 {
        return Err(invalid());
    }
    // Stable across segment publication; no lock is renamed with the data.
    let gate = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(parent.join("store.lock"))?;
    let meta = gate.metadata()?;
    // SAFETY: geteuid has no preconditions.
    if !meta.is_file() || meta.uid() != unsafe { libc::geteuid() } || meta.mode() & 0o777 != 0o600 {
        return Err(invalid());
    }
    lock(
        &gate,
        if write { libc::LOCK_EX } else { libc::LOCK_SH },
        deadline,
    )?;
    let after = fs::symlink_metadata(parent.join("store.lock"))?;
    if (meta.dev(), meta.ino()) != (after.dev(), after.ino()) {
        return Err(invalid());
    }
    let layout = checkpoint::load(&parent, deadline, write)?;
    let cap = if layout.tip.is_some() {
        MAX_SEGMENT_BYTES
    } else {
        MAX_BYTES
    };
    let file = match private_file(&parent, &layout.active, write, cap, deadline) {
        Ok(file) => file,
        Err(e) if !write && layout.tip.is_none() && e.kind() == io::ErrorKind::NotFound => {
            return Ok(None);
        }
        Err(e) => return Err(e),
    };
    Ok(Some(Ledger {
        file,
        parent,
        storage: Storage {
            layout,
            _gate: gate,
        },
    }))
}

pub(super) fn expected_header(ledger: &Ledger) -> Option<Header> {
    ledger
        .storage
        .layout
        .chain
        .first()
        .map(|(_, m)| Header::from_manifest(m))
}

pub(super) fn segment_limit(ledger: &Ledger) -> usize {
    if ledger.storage.layout.tip.is_some() {
        MAX_SEGMENT_BYTES
    } else {
        MAX_BYTES
    }
}

fn key_id(claim: &Claim) -> Option<String> {
    Some(format!(
        "k{}",
        sha256(
            format!(
                "agent-session.claude-notice-key.v1\0{}\0{}",
                claim.binding.launch.session_id,
                claim.caller_key_sha256.as_deref()?
            )
            .as_bytes()
        )
    ))
}

pub(super) fn key_used(
    ledger: &Ledger,
    scan: &Scan,
    claim: &Claim,
    deadline: Instant,
) -> io::Result<bool> {
    let Some(key) = key_id(claim) else {
        return Ok(false);
    };
    Ok(checkpoint::get(ledger, &key, deadline)?.is_some()
        || scan
            .claims
            .values()
            .any(|(other, _)| key_id(other).as_ref() == Some(&key)))
}

pub(super) fn operation_used(
    ledger: &Ledger,
    scan: &Scan,
    claim: &Claim,
    deadline: Instant,
) -> io::Result<bool> {
    Ok(scan.claims.contains_key(&claim.op)
        || checkpoint::get(
            ledger,
            &format!("o{}", sha256(claim.op.as_bytes())),
            deadline,
        )?
        .is_some())
}

pub(super) fn compact(ledger: &mut Ledger, scan: &Scan, deadline: Instant) -> io::Result<()> {
    archive::compact(ledger, scan, deadline)
}

#[cfg(test)]
pub(super) fn merge_archived(
    ledger: &Ledger,
    target: &mut Scan,
    deadline: Instant,
) -> io::Result<()> {
    let layout = load_verified(&ledger.parent, deadline, false)?;
    for (index, (_, manifest)) in layout.chain.iter().enumerate() {
        check_time(deadline)?;
        let cap = if manifest.archive.file == "ledger.jsonl" {
            MAX_BYTES
        } else {
            MAX_SEGMENT_BYTES
        };
        let bytes = sealed::verified(&ledger.parent, &manifest.archive, cap, false, deadline)?;
        let header = layout
            .chain
            .get(index + 1)
            .map(|(_, m)| Header::from_manifest(m));
        let scan = super::records::scan(&bytes, deadline, header.as_ref())?;
        super::records::merge(target, scan)?;
    }
    Ok(())
}
