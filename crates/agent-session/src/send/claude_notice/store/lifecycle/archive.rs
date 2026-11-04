//! Prepare immutable objects, then publish exactly one current-pointer change.
use super::*;

const TERMINAL_AGE_SECONDS: i64 = 60 * 60;

fn write_immutable(parent: &Path, name: &str, bytes: &[u8], deadline: Instant) -> io::Result<()> {
    check_time(deadline)?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(parent.join(name))?;
    file.write_all(bytes)?;
    file.sync_all()?;
    check_time(deadline)
}

pub(super) fn compact(ledger: &mut Ledger, scan: &Scan, deadline: Instant) -> io::Result<()> {
    check_time(deadline)?;
    let now = jiff::Timestamp::now();
    let before = ledger
        .storage
        .layout
        .chain
        .first()
        .map(|(_, m)| m.first_seen.clone())
        .unwrap_or_default();
    let mut first_seen = before.clone();
    let mut eligible = HashSet::new();
    for terminal in scan.terminals.values() {
        if terminal.state == State::Unknown {
            continue;
        }
        let recorded = terminal
            .timestamp
            .as_deref()
            .and_then(|s| s.parse::<jiff::Timestamp>().ok())
            .filter(|time| *time <= now);
        let observed = recorded.or_else(|| {
            first_seen
                .get(&terminal.op)
                .and_then(|s| s.parse::<jiff::Timestamp>().ok())
        });
        let Some(time) = observed else {
            first_seen.insert(terminal.op.clone(), now.to_string());
            continue;
        };
        if now.as_second().saturating_sub(time.as_second()) >= TERMINAL_AGE_SECONDS {
            eligible.insert(terminal.op.clone());
        }
    }
    first_seen.retain(|op, _| scan.terminals.contains_key(op) && !eligible.contains(op));
    if eligible.is_empty() && first_seen == before {
        return Ok(());
    }

    ledger.file.seek(SeekFrom::Start(0))?;
    let mut previous = Vec::new();
    (&ledger.file)
        .take(segment_limit(ledger) as u64 + 1)
        .read_to_end(&mut previous)?;
    if previous.len() > segment_limit(ledger) {
        return Err(invalid());
    }
    let archive = Reference {
        file: ledger.storage.layout.active.clone(),
        sha256: sha256(&previous),
    };
    let mut entries = BTreeSet::new();
    for (claim, _) in scan.claims.values() {
        if let Some(key) = key_id(claim) {
            entries.insert(key);
        }
        entries.insert(format!("o{}", sha256(claim.op.as_bytes())));
    }
    let index = entries
        .into_iter()
        .map(|key| format!("{key}\n"))
        .collect::<String>()
        .into_bytes();
    if index.len() > MAX_INDEX {
        return Err(invalid());
    }
    let id = uuid::Uuid::new_v4().to_string();
    let active = format!("segment-{id}.jsonl");
    let index_name = format!("keys-{id}.txt");
    let manifest_name = format!("manifest-{id}.json");
    let generation = ledger
        .storage
        .layout
        .chain
        .first()
        .map_or(Some(1), |(_, m)| m.generation.checked_add(1))
        .ok_or_else(invalid)?;
    let manifest = Manifest {
        schema_version: MANIFEST_SCHEMA.into(),
        generation,
        active: active.clone(),
        predecessor: ledger.storage.layout.tip.clone(),
        archive,
        key_index: Reference {
            file: index_name.clone(),
            sha256: sha256(&index),
        },
        first_seen,
    };
    let header = Header::from_manifest(&manifest);
    let mut successor = serde_json::to_vec(&header)?;
    successor.push(b'\n');
    for (position, line) in previous.split_inclusive(|byte| *byte == b'\n').enumerate() {
        check_time(deadline)?;
        if position == 0 && expected_header(ledger).is_some() {
            continue;
        }
        let value: serde_json::Value = serde_json::from_slice(line)?;
        let op = value["op"].as_str().ok_or_else(invalid)?;
        if !eligible.contains(op) {
            successor.extend_from_slice(line);
        }
    }
    if successor.len() > MAX_SEGMENT_BYTES {
        return Err(invalid());
    }
    let manifest_bytes = serde_json::to_vec(&manifest)?;
    if manifest_bytes.len() > MAX_MANIFEST {
        return Err(invalid());
    }
    // Everything referenced is durable before the pointer can expose it.
    write_immutable(&ledger.parent, &index_name, &index, deadline)?;
    write_immutable(&ledger.parent, &active, &successor, deadline)?;
    write_immutable(&ledger.parent, &manifest_name, &manifest_bytes, deadline)?;
    File::open(&ledger.parent)?.sync_all()?;
    let pointer = Pointer {
        schema_version: POINTER_SCHEMA.into(),
        manifest: Reference {
            file: manifest_name,
            sha256: sha256(&manifest_bytes),
        },
    };
    let temporary = format!("current-{id}.tmp");
    write_immutable(
        &ledger.parent,
        &temporary,
        &serde_json::to_vec(&pointer)?,
        deadline,
    )?;
    check_time(deadline)?;
    fs::rename(
        ledger.parent.join(&temporary),
        ledger.parent.join("current.json"),
    )?;
    File::open(&ledger.parent)?.sync_all()?;
    ensure_marker(&ledger.parent, deadline)?;
    let layout = load(&ledger.parent, deadline)?;
    let file = private_file(
        &ledger.parent,
        &layout.active,
        true,
        MAX_SEGMENT_BYTES,
        deadline,
    )?;
    ledger.file = file;
    ledger.storage.layout = layout;
    check_time(deadline)
}
