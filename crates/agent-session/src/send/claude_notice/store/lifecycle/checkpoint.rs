//! Verified archive checkpoint, permanent reservations and physical-row index.
//! Historical originals remain audit inputs. Normal calls authenticate the root,
//! current manifest and accessed pages, rather than re-reading every original.
use super::*;

pub(super) const MARKER: &[u8] =
    b"{\"schema_version\":\"agent-session.claude-notice-indexed-layout.v1\"}\n";
const SCHEMA: &str = "agent-session.claude-notice-checkpoint.v1";
const MIGRATION_BUDGET: Duration = Duration::from_secs(3600);

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Checkpoint {
    schema_version: String,
    manifest: Option<Reference>,
    generation: u64,
    index: String,
    bytes: u64,
    root: Option<pages::Page>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Operation {
    claim: String,
    terminal: Option<String>,
}

fn read(parent: &Path) -> io::Result<Checkpoint> {
    let checkpoint: Checkpoint =
        serde_json::from_slice(&read_private(parent, "checkpoint.json", MAX_MANIFEST)?)?;
    if checkpoint.schema_version != SCHEMA
        || !named(&checkpoint.index, "index-", ".bin")
        || (checkpoint.generation == 0) != checkpoint.manifest.is_none()
        || checkpoint.bytes == 0 && checkpoint.root.is_some()
        || checkpoint.bytes > 16 * 1024 * 1024 * 1024 * 1024
    {
        return Err(invalid());
    }
    Ok(checkpoint)
}

pub(super) fn empty(parent: &Path) -> io::Result<bool> {
    Ok(parent.join("checkpoint.json").try_exists()? && read(parent)?.manifest.is_none())
}

fn publish(
    parent: &Path,
    tip: Option<Reference>,
    generation: u64,
    name: &str,
    index: &pages::Index,
    deadline: Instant,
) -> io::Result<()> {
    let checkpoint = Checkpoint {
        schema_version: SCHEMA.into(),
        manifest: tip,
        generation,
        index: name.into(),
        bytes: index.committed,
        root: index.root.clone(),
    };
    let temporary = format!("checkpoint-{}.tmp", uuid::Uuid::new_v4());
    let temporary_file = archive::prepare_immutable(
        parent,
        &temporary,
        &serde_json::to_vec(&checkpoint)?,
        deadline,
    )?;
    archive::sync_files(&[index.file(), &temporary_file], deadline)?;
    fs::rename(parent.join(temporary), parent.join("checkpoint.json"))?;
    File::open(parent)?.sync_all()?;
    marker(parent, deadline)
}

pub(super) fn marker(parent: &Path, deadline: Instant) -> io::Result<()> {
    match fs::symlink_metadata(parent.join("layout.json")) {
        Ok(_) => {
            let bytes = read_private(parent, "layout.json", 4096)?;
            if bytes == MARKER {
                return Ok(());
            }
            if bytes != LAYOUT_MARKER {
                return Err(invalid());
            }
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    let temporary = format!("layout-{}.tmp", uuid::Uuid::new_v4());
    archive::write_immutable(parent, &temporary, MARKER, deadline)?;
    fs::rename(parent.join(temporary), parent.join("layout.json"))?;
    File::open(parent)?.sync_all()
}

pub(in crate::send::claude_notice::store) fn proof_key(history: &str, offset: u64) -> String {
    format!("p{}:{offset:016x}", sha256(history.as_bytes()))
}

fn combine(key: &str, old: &str, new: &str) -> io::Result<String> {
    if old == new {
        return Ok(old.into());
    }
    match key.as_bytes().first() {
        Some(b'p') => Ok(String::new()), // Two operations bound one physical row.
        Some(b'o') => {
            let mut old: Operation = serde_json::from_str(old)?;
            let new: Operation = serde_json::from_str(new)?;
            if old.claim != new.claim
                || old
                    .terminal
                    .as_ref()
                    .zip(new.terminal.as_ref())
                    .is_some_and(|(a, b)| a != b)
            {
                return Err(invalid());
            }
            old.terminal = new.terminal.or(old.terminal);
            Ok(serde_json::to_string(&old)?)
        }
        _ => Err(invalid()),
    }
}

fn insert(entries: &mut BTreeMap<String, String>, key: String, value: String) -> io::Result<()> {
    let value = match entries.get(&key) {
        Some(old) => combine(&key, old, &value)?,
        None => value,
    };
    entries.insert(key, value);
    Ok(())
}

fn add_scan(
    entries: &mut BTreeMap<String, String>,
    scan: &Scan,
    deadline: Instant,
) -> io::Result<()> {
    for (claim, hash) in scan.claims.values() {
        check_time(deadline)?;
        if let Some(key) = key_id(claim) {
            insert(entries, key, claim.op.clone())?;
        }
        let terminal = scan.terminals.get(&claim.op);
        insert(
            entries,
            format!("o{}", sha256(claim.op.as_bytes())),
            serde_json::to_string(&Operation {
                claim: hash.clone(),
                terminal: terminal
                    .map(serde_json::to_vec)
                    .transpose()?
                    .map(|bytes| sha256(&bytes)),
            })?,
        )?;
        if let Some(proof) = terminal.and_then(|terminal| terminal.confirmed(claim)) {
            insert(
                entries,
                proof_key(&claim.history_id, proof.row_offset),
                serde_json::to_string(&proof)?,
            )?;
        }
    }
    Ok(())
}

fn entries(
    parent: &Path,
    layout: &Layout,
    deadline: Instant,
) -> io::Result<BTreeMap<String, String>> {
    let mut entries = BTreeMap::new();
    for (position, (_, manifest)) in layout.chain.iter().enumerate().rev() {
        let bytes = sealed::verified(
            parent,
            &manifest.archive,
            if manifest.archive.file == "ledger.jsonl" {
                MAX_BYTES
            } else {
                MAX_SEGMENT_BYTES
            },
            false,
            deadline,
        )?;
        let header = layout
            .chain
            .get(position + 1)
            .map(|(_, m)| Header::from_manifest(m));
        let scan = records::scan(&bytes, deadline, header.as_ref())?;
        add_scan(&mut entries, &scan, deadline)?;
    }
    let keys: BTreeSet<String> = entries
        .keys()
        .filter(|key| !key.starts_with('p'))
        .cloned()
        .collect();
    if keys != layout.keys {
        return Err(invalid());
    }
    Ok(entries)
}

fn bootstrap(parent: &Path, deadline: Instant) -> io::Result<()> {
    let full = load_verified(parent, deadline, true)?;
    let name = format!("index-{}.bin", uuid::Uuid::new_v4());
    let mut index = pages::Index::create(parent, &name)?;
    index.build(entries(parent, &full, deadline)?, deadline)?;
    publish(
        parent,
        full.tip.clone(),
        full.chain.first().map_or(0, |(_, m)| m.generation),
        &name,
        &index,
        deadline,
    )
}

fn tip(parent: &Path) -> io::Result<Option<(Reference, Manifest)>> {
    let bytes = match fs::symlink_metadata(parent.join("current.json")) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
        Ok(_) => read_private(parent, "current.json", 4096)?,
    };
    let pointer: Pointer = serde_json::from_slice(&bytes)?;
    if pointer.schema_version != POINTER_SCHEMA
        || !named(&pointer.manifest.file, "manifest-", ".json")
    {
        return Err(invalid());
    }
    let manifest: Manifest =
        serde_json::from_slice(&verified(parent, &pointer.manifest, MAX_MANIFEST)?)?;
    if manifest.schema_version != MANIFEST_SCHEMA
        || manifest.generation == 0
        || !named(&manifest.active, "segment-", ".jsonl")
        || !(manifest.archive.file == "ledger.jsonl"
            || named(&manifest.archive.file, "segment-", ".jsonl"))
        || !named(&manifest.key_index.file, "keys-", ".txt")
        || !hash_valid(&manifest.archive.sha256)
        || manifest.first_seen.len() > MAX_CLAIMS
        || manifest.first_seen.iter().any(|(op, time)| {
            uuid::Uuid::parse_str(op).is_err() || time.parse::<jiff::Timestamp>().is_err()
        })
    {
        return Err(invalid());
    }
    Ok(Some((pointer.manifest, manifest)))
}

pub(super) fn load(parent: &Path, deadline: Instant, write: bool) -> io::Result<Layout> {
    if !parent.join("checkpoint.json").try_exists()? {
        if fs::symlink_metadata(parent.join("layout.json")).is_ok()
            && read_private(parent, "layout.json", 4096)? == MARKER
        {
            return Err(invalid());
        }
        if write {
            // One-time upgrade completes under the store lock. The original send
            // cannot consume its caller key after its own deadline has elapsed.
            bootstrap(parent, Instant::now() + MIGRATION_BUDGET)?;
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    CHECKPOINT_INITIALIZED,
                ));
            }
        } else {
            if parent.join("current.json").try_exists()? {
                return Err(invalid());
            }
            return load_verified(parent, deadline, false);
        }
    }
    let checkpoint = read(parent)?;
    let marker_bytes = match read_private(parent, "layout.json", 4096) {
        Ok(bytes) => Some(bytes),
        Err(_)
            if write
                && fs::symlink_metadata(parent.join("layout.json"))
                    .is_err_and(|e| e.kind() == io::ErrorKind::NotFound) =>
        {
            None
        }
        Err(e) => return Err(e),
    };
    if marker_bytes.as_deref() != Some(MARKER) {
        if write {
            marker(parent, deadline)?;
        } else {
            return Err(invalid());
        }
    }
    let current = tip(parent)?;
    let current_ref = current.as_ref().map(|(reference, _)| reference.clone());
    let index = pages::Index::open(
        parent,
        &checkpoint.index,
        checkpoint.root,
        checkpoint.bytes,
        write,
    )?;
    if current_ref != checkpoint.manifest {
        if !write {
            return Err(invalid());
        }
        let (_, manifest) = current.as_ref().ok_or_else(invalid)?;
        if manifest.predecessor != checkpoint.manifest
            || checkpoint.generation.checked_add(1) != Some(manifest.generation)
        {
            return Err(invalid());
        }
        let previous = checkpoint
            .manifest
            .as_ref()
            .map(|r| {
                verified(parent, r, MAX_MANIFEST).and_then(|bytes| {
                    serde_json::from_slice::<Manifest>(&bytes).map_err(Into::into)
                })
            })
            .transpose()?;
        if manifest.archive.file
            != previous
                .as_ref()
                .map_or("ledger.jsonl", |m| m.active.as_str())
        {
            return Err(invalid());
        }
        let bytes = sealed::verified(
            parent,
            &manifest.archive,
            if previous.is_some() {
                MAX_SEGMENT_BYTES
            } else {
                MAX_BYTES
            },
            true,
            deadline,
        )?;
        let archived = records::scan(
            &bytes,
            deadline,
            previous.as_ref().map(Header::from_manifest).as_ref(),
        )?;
        let expected: BTreeSet<String> = archived
            .claims
            .values()
            .flat_map(|(c, _)| {
                key_id(c)
                    .into_iter()
                    .chain(std::iter::once(format!("o{}", sha256(c.op.as_bytes()))))
            })
            .collect();
        let mut expected_index = expected
            .into_iter()
            .map(|key| format!("{key}\n"))
            .collect::<String>()
            .into_bytes();
        let actual_index = verified(parent, &manifest.key_index, MAX_INDEX)?;
        if expected_index != actual_index {
            return Err(invalid());
        }
        expected_index.clear();
        let mut layout = Layout {
            active: manifest.active.clone(),
            tip: current_ref,
            chain: current.into_iter().collect(),
            keys: BTreeSet::new(),
            index: Some(index),
            index_name: Some(checkpoint.index),
        };
        update(parent, &mut layout, &archived, deadline)?;
        return Ok(layout);
    }
    if checkpoint.generation != current.as_ref().map_or(0, |(_, m)| m.generation) {
        return Err(invalid());
    }
    check_time(deadline)?;
    Ok(Layout {
        active: current
            .as_ref()
            .map_or_else(|| "ledger.jsonl".into(), |(_, m)| m.active.clone()),
        tip: current_ref,
        chain: current.into_iter().collect(),
        keys: BTreeSet::new(),
        index: Some(index),
        index_name: Some(checkpoint.index),
    })
}

/// In-process rotation already owns the verified live scan and the lock.
/// Interrupted publication still takes the independent verification path in load.
pub(super) fn advance(
    ledger: &mut Ledger,
    tip: Reference,
    manifest: Manifest,
    scan: &Scan,
    deadline: Instant,
) -> io::Result<Layout> {
    if manifest.predecessor != ledger.storage.layout.tip {
        return Err(invalid());
    }
    let mut layout = Layout {
        active: manifest.active.clone(),
        tip: Some(tip.clone()),
        chain: vec![(tip, manifest)],
        keys: BTreeSet::new(),
        index: ledger.storage.layout.index.take(),
        index_name: ledger.storage.layout.index_name.take(),
    };
    update(&ledger.parent, &mut layout, scan, deadline)?;
    Ok(layout)
}

pub(in crate::send::claude_notice::store) fn get(
    ledger: &Ledger,
    key: &str,
    deadline: Instant,
) -> io::Result<Option<String>> {
    match &ledger.storage.layout.index {
        Some(index) => index.get(key, deadline),
        None => Ok(None),
    }
}

pub(super) fn update(
    parent: &Path,
    layout: &mut Layout,
    archived: &Scan,
    deadline: Instant,
) -> io::Result<()> {
    let mut additions = BTreeMap::new();
    add_scan(&mut additions, archived, deadline)?;
    let index = layout.index.as_mut().ok_or_else(invalid)?;
    index.apply(additions, deadline, combine)?;
    publish(
        parent,
        layout.tip.clone(),
        layout.chain.first().map_or(0, |(_, m)| m.generation),
        layout.index_name.as_deref().ok_or_else(invalid)?,
        index,
        deadline,
    )
}

/// Explicit complete audit: originals and every reachable index page must agree.
pub(crate) fn audit(state: &Path, deadline: Instant) -> io::Result<()> {
    let Some(ledger) = open(state, true, deadline)? else {
        return Err(invalid());
    };
    let full = load_verified(&ledger.parent, deadline, false)?;
    let expected = entries(&ledger.parent, &full, deadline)?;
    let actual = ledger
        .storage
        .layout
        .index
        .as_ref()
        .ok_or_else(invalid)?
        .all(deadline)?;
    if expected != actual {
        return Err(invalid());
    }
    let mut ledger = ledger;
    ledger.scan(deadline)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn expired_initiating_deadline_still_leaves_a_completed_checkpoint() {
        let root = tempfile::tempdir().unwrap();
        let parent = root.path().join("history/claude-notices");
        fs::create_dir_all(&parent).unwrap();
        fs::set_permissions(&parent, fs::Permissions::from_mode(0o700)).unwrap();
        // Exercise the phase after the caller acquired its store lock. A
        // deliberately expired deadline avoids a wall-time race in this test.
        let error = load(&parent, Instant::now() - Duration::from_millis(1), true)
            .err()
            .unwrap();
        assert_eq!(error.to_string(), CHECKPOINT_INITIALIZED);
        assert_eq!(read_private(&parent, "layout.json", 4096).unwrap(), MARKER);
        assert!(parent.join("checkpoint.json").is_file());
        let retry = load(&parent, Instant::now() + Duration::from_secs(15), true).unwrap();
        assert_eq!(retry.active, "ledger.jsonl");
        assert!(retry.index.is_some());
    }
}
