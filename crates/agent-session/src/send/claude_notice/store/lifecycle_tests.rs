//! Lifecycle acceptance at the durable claim/attestation boundary; synthetic state only.
use super::*;
use crate::provider_prompt::native_notice::Row;
use crate::send::claude_notice::Launch;
use pretty_assertions::assert_eq;
use serde_json::{Value, json};
use std::cell::Cell;
use std::os::unix::fs::PermissionsExt;

const POINTER_SCHEMA: &str = "agent-session.claude-notice-store-current.v1";
const MANIFEST_SCHEMA: &str = "agent-session.claude-notice-store-manifest.v1";
const SEGMENT_SCHEMA: &str = "agent-session.claude-notice-segment.v1";

fn context(root: &Path) -> CliContext {
    let parent = root.join("history/claude-notices");
    fs::create_dir_all(&parent).unwrap();
    fs::set_permissions(parent, fs::Permissions::from_mode(0o700)).unwrap();
    CliContext {
        state_dir: root.to_path_buf(),
        host: None,
    }
}

fn private_file(path: &Path, bytes: &[u8]) {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .unwrap();
    file.write_all(bytes).unwrap();
}

fn claim(index: usize, key: Option<&str>) -> Claim {
    Claim::new(
        format!("{index:08x}-0000-4000-8000-000000000000"),
        Binding {
            launch: Launch {
                session_id: "synthetic".into(),
                incarnation: "runtime-one".into(),
                generation: 1,
                profile: None,
            },
            provider: "claude".into(),
            provider_session_id: "provider-one".into(),
            provider_version: "2.1.317".into(),
            module_sha256: sha256(runtime::MODULE.as_bytes()),
        },
        "<!-- agent-notice:v1 kind=mailbox source=synthetic -->\n\nfixture\n",
        key,
        Stamp {
            device: 1,
            inode: 2,
            eof: 0,
            continuity_sha256: sha256(b""),
        },
        Duration::from_secs(15),
    )
}

fn claim_line(claim: &Claim) -> Vec<u8> {
    let mut value = serde_json::to_value(claim).unwrap();
    value["kind"] = json!("claim");
    serde_json::to_vec(&value).unwrap()
}

fn pair(claim: &Claim) -> (Vec<u8>, Value) {
    let raw = claim_line(claim);
    let text = "<!-- agent-notice:v1 kind=mailbox source=synthetic -->\n\nfixture\n";
    let row = json!({"type":"user", "sessionId":"provider-one", "uuid":claim.op,
        "version":"2.1.317", "promptSource":"system", "isSidechain":false,
        "timestamp":"2000-01-01T00:00:00Z", "cwd":"/workspace/project",
        "origin":{"kind":"plugin", "name":"agent-session-claude-notice", "asUser":true},
        "message":{"role":"user", "content":text}});
    let terminal = json!({"schema_version":SCHEMA, "kind":"terminal", "op":claim.op,
        "claim_sha256":sha256(&raw), "state":"submitted", "native_uuid":claim.op,
        "row_offset":4, "row_payload_sha256":sha256(&serde_json::to_vec(&row).unwrap()),
        "envelope_sha256":claim.envelope_sha256, "timestamp":"2000-01-01T00:00:00Z",
        "confirmation_elapsed_ms":1});
    let mut bytes = raw;
    bytes.push(b'\n');
    bytes.extend(serde_json::to_vec(&terminal).unwrap());
    bytes.push(b'\n');
    (bytes, row)
}

fn reference(file: &str, bytes: &[u8]) -> Value {
    json!({"file":file, "sha256":sha256(bytes)})
}

fn header(generation: u64, archive: &[u8], keys: &[u8]) -> Vec<u8> {
    let value = json!({"schema_version":SEGMENT_SCHEMA, "kind":"segment", "generation":generation,
        "previous_sha256":sha256(archive), "key_index_sha256":sha256(keys)});
    let mut bytes = serde_json::to_vec(&value).unwrap();
    bytes.push(b'\n');
    bytes
}

/// A committed second generation: the proof was written after genesis, then archived.
fn archived_fixture(context: &CliContext, archived: &Claim) -> Value {
    archived_fixture_with_live(context, archived, b"", json!({}))
}

fn archived_fixture_with_live(
    context: &CliContext,
    archived: &Claim,
    live: &[u8],
    first_seen: Value,
) -> Value {
    let parent = context.state_dir.join("history/claude-notices");
    let first = "00000000-0000-4000-8000-000000000001";
    let second = "00000000-0000-4000-8000-000000000002";
    let segment1 = format!("segment-{first}.jsonl");
    let segment2 = format!("segment-{second}.jsonl");
    let manifest1 = format!("manifest-{first}.json");
    let manifest2 = format!("manifest-{second}.json");
    let keys1 = format!("keys-{first}.txt");
    let keys2 = format!("keys-{second}.txt");
    private_file(&parent.join("ledger.jsonl"), b"");
    fs::set_permissions(
        parent.join("ledger.jsonl"),
        fs::Permissions::from_mode(0o400),
    )
    .unwrap();
    private_file(&parent.join(&keys1), b"");
    let (pair, row) = pair(archived);
    let mut old_segment = header(1, b"", b"");
    old_segment.extend(pair);
    private_file(&parent.join(&segment1), &old_segment);
    fs::set_permissions(parent.join(&segment1), fs::Permissions::from_mode(0o400)).unwrap();
    let m1 = serde_json::to_vec(&json!({"schema_version":MANIFEST_SCHEMA, "generation":1,
        "active":segment1, "predecessor":null, "archive":reference("ledger.jsonl", b""),
        "key_index":reference(&keys1,b""), "first_seen":{}}))
    .unwrap();
    private_file(&parent.join(&manifest1), &m1);
    let key_id = sha256(
        format!(
            "agent-session.claude-notice-key.v1\0{}\0{}",
            archived.binding.launch.session_id,
            archived.caller_key_sha256.as_deref().unwrap()
        )
        .as_bytes(),
    );
    let keys = format!("k{key_id}\no{}\n", sha256(archived.op.as_bytes())).into_bytes();
    private_file(&parent.join(&keys2), &keys);
    let mut active_bytes = header(2, &old_segment, &keys);
    active_bytes.extend_from_slice(live);
    private_file(&parent.join(&segment2), &active_bytes);
    let m2 = serde_json::to_vec(&json!({"schema_version":MANIFEST_SCHEMA, "generation":2,
        "active":segment2, "predecessor":reference(&manifest1,&m1),
        "archive":reference(&segment1,&old_segment), "key_index":reference(&keys2,&keys),
        "first_seen":first_seen}))
    .unwrap();
    private_file(&parent.join(&manifest2), &m2);
    private_file(
        &parent.join("current.json"),
        &serde_json::to_vec(&json!({
        "schema_version":POINTER_SCHEMA,"manifest":reference(&manifest2,&m2)}))
        .unwrap(),
    );
    private_file(
        &parent.join("layout.json"),
        b"{\"schema_version\":\"agent-session.claude-notice-layout.v1\"}\n",
    );
    row
}

#[test]
fn full_finished_ledger_archives_before_new_claim_admission() {
    let root = tempfile::tempdir().unwrap();
    let context = context(root.path());
    let parent = root.path().join("history/claude-notices");
    let mut bytes = Vec::new();
    for index in 0..MAX_CLAIMS {
        let claim = claim(index, None);
        if index == 0 {
            bytes.extend(claim_line(&claim));
            bytes.push(b'\n');
        } else if index == 1 {
            let (pair, _) = pair(&claim);
            let mut lines = pair.split_inclusive(|byte| *byte == b'\n');
            bytes.extend(lines.next().unwrap());
            let mut terminal: Value = serde_json::from_slice(lines.next().unwrap()).unwrap();
            for field in [
                "native_uuid",
                "row_offset",
                "row_payload_sha256",
                "envelope_sha256",
                "timestamp",
                "confirmation_elapsed_ms",
            ] {
                terminal.as_object_mut().unwrap().remove(field);
            }
            terminal["state"] = json!("unknown");
            terminal["reason"] = json!("timeout");
            bytes.extend(serde_json::to_vec(&terminal).unwrap());
            bytes.push(b'\n');
        } else {
            bytes.extend(pair(&claim).0);
        }
    }
    private_file(&parent.join("ledger.jsonl"), &bytes);
    // A crash before pointer publication may leave partial, unreferenced copies.
    let orphan = parent.join("segment-00000000-0000-4000-8000-000000000099.jsonl");
    private_file(&orphan, b"{\"kind\":");
    let consumed = Cell::new(false);
    let result = consume(
        &context,
        claim(MAX_CLAIMS + 1, Some("new-key")),
        Instant::now() + Duration::from_secs(15),
        || {
            consumed.set(true);
            Ok(())
        },
    );
    assert!(
        result.is_ok(),
        "full finished ledger must admit after archival: {:?}",
        result.err()
    );
    assert!(consumed.get());
    assert_eq!(
        fs::read(parent.join("ledger.jsonl")).unwrap(),
        bytes,
        "preserve original bytes"
    );
    assert!(parent.join("current.json").exists());
    assert!(parent.join("layout.json").exists());
    assert_eq!(fs::read(&orphan).unwrap(), b"{\"kind\":");
    {
        let mut live = Ledger::open(root.path(), false, Instant::now() + Duration::from_secs(15))
            .unwrap()
            .unwrap();
        let scan = live.scan(Instant::now() + Duration::from_secs(15)).unwrap();
        assert_eq!(
            scan.claims.len(),
            3,
            "unfinished, unknown and the new claim stay live"
        );
        assert!(scan.claims.contains_key(&claim(0, None).op));
        assert_eq!(scan.terminals[&claim(1, None).op].state, State::Unknown);
    }
    // Losing committed metadata cannot silently return to the genesis ledger.
    fs::remove_file(parent.join("current.json")).unwrap();
    let consumed = Cell::new(false);
    let refused = consume(
        &context,
        claim(MAX_CLAIMS + 2, Some("another-key")),
        Instant::now() + Duration::from_secs(15),
        || {
            consumed.set(true);
            Ok(())
        },
    );
    assert!(refused.is_err());
    assert!(!consumed.get());
    assert!(!parent.join("current.json").exists());
    assert_eq!(fs::read(parent.join("ledger.jsonl")).unwrap(), bytes);
}

#[test]
fn archived_key_cannot_be_consumed_without_its_original_session_key_file() {
    let root = tempfile::tempdir().unwrap();
    let context = context(root.path());
    let parent = root.path().join("history/claude-notices");
    let mut bytes = Vec::new();
    for index in 0..MAX_CLAIMS {
        bytes.extend(pair(&claim(index, (index == 1).then_some("old-key"))).0);
    }
    private_file(&parent.join("ledger.jsonl"), &bytes);
    consume(
        &context,
        claim(MAX_CLAIMS + 1, Some("new-key")),
        Instant::now() + Duration::from_secs(15),
        || Ok(()),
    )
    .unwrap();
    let consumed = Cell::new(false);
    let result = consume(
        &context,
        claim(MAX_CLAIMS + 2, Some("old-key")),
        Instant::now() + Duration::from_secs(15),
        || {
            consumed.set(true);
            Ok(())
        },
    );
    let error = result.err().expect("archived key must remain consumed");
    assert_eq!(error.code(), "send-idempotency-key-used");
    assert!(!consumed.get(), "never enter the key-consuming callback");
}

#[test]
fn archived_native_attestation_keeps_its_exact_row_binding() {
    let root = tempfile::tempdir().unwrap();
    let context = context(root.path());
    let archived = claim(1, Some("old-key"));
    let row = archived_fixture(&context, &archived);
    let parent = root.path().join("history/claude-notices");
    private_file(
        &parent.join("keys-00000000-0000-4000-8000-000000000099.txt"),
        b"partial",
    );
    fs::remove_file(parent.join("layout.json")).unwrap();
    let sealed = parent.join("segment-00000000-0000-4000-8000-000000000001.jsonl");
    // Publication was durable, but the writer stopped before sealing the tip.
    fs::set_permissions(&sealed, fs::Permissions::from_mode(0o600)).unwrap();
    assert!(confirmed(root.path(), Instant::now() + Duration::from_secs(15)).is_err());
    consume(
        &context,
        claim(9000, Some("after-publication")),
        Instant::now() + Duration::from_secs(15),
        || Ok(()),
    )
    .unwrap();
    assert_eq!(fs::metadata(&sealed).unwrap().mode() & 0o777, 0o400);
    assert!(
        parent.join("layout.json").exists(),
        "finish marker before unique successor writes"
    );
    let proofs = confirmed(root.path(), Instant::now() + Duration::from_secs(15)).unwrap();
    assert_eq!(proofs.len(), 1, "archived proof remains discoverable");
    let proof = &proofs[0];
    assert_eq!(proof.claim.op, archived.op);
    assert_eq!(proof.native_uuid, archived.op);
    assert_eq!(proof.row_offset, 4);
    assert_eq!(
        proof.row_payload_sha256,
        sha256(&serde_json::to_vec(&row).unwrap())
    );
    let native: Row =
        crate::provider_prompt::native_notice::row(&row, &proof.claim.binding).unwrap();
    assert_eq!(sha256(native.text.as_bytes()), proof.claim.envelope_sha256);
    assert_eq!(native.native_uuid, proof.native_uuid);
    // A sealed segment's digest is checked before its proof or key index is trusted.
    fs::set_permissions(&sealed, fs::Permissions::from_mode(0o600)).unwrap();
    OpenOptions::new()
        .append(true)
        .open(&sealed)
        .unwrap()
        .write_all(b" ")
        .unwrap();
    fs::set_permissions(&sealed, fs::Permissions::from_mode(0o400)).unwrap();
    assert!(confirmed(root.path(), Instant::now() + Duration::from_secs(15)).is_err());
}

#[test]
fn retired_segment_refuses_an_older_writer_and_keeps_its_proofs() {
    let root = tempfile::tempdir().unwrap();
    let context = context(root.path());
    let parent = root.path().join("history/claude-notices");
    let mut bytes = claim_line(&claim(0, None));
    bytes.push(b'\n');
    for index in 1..MAX_CLAIMS {
        bytes.extend(pair(&claim(index, None)).0);
    }
    private_file(&parent.join("ledger.jsonl"), &bytes);
    consume(
        &context,
        claim(MAX_CLAIMS + 1, Some("fresh")),
        Instant::now() + Duration::from_secs(15),
        || Ok(()),
    )
    .unwrap();
    // Model the prior writer's append-open and exact 0600 admission check.
    // The metadata check also makes this fixture valid for privileged test users.
    let append_attempt = (|| -> io::Result<()> {
        let mut file = OpenOptions::new()
            .read(true)
            .append(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(parent.join("ledger.jsonl"))?;
        if file.metadata()?.mode() & 0o777 != 0o600 {
            return Err(io::ErrorKind::PermissionDenied.into());
        }
        let terminal = pair(&claim(0, None))
            .0
            .split_inclusive(|byte| *byte == b'\n')
            .nth(1)
            .unwrap()
            .to_vec();
        file.write_all(&terminal)?;
        file.sync_all()
    })();
    assert!(
        append_attempt.is_err(),
        "retired bytes reject an older writer"
    );
    assert_eq!(fs::read(parent.join("ledger.jsonl")).unwrap(), bytes);
    assert_eq!(
        confirmed(root.path(), Instant::now() + Duration::from_secs(15))
            .unwrap()
            .len(),
        MAX_CLAIMS - 1
    );
    let unfinished = claim(0, None);
    finish(
        &context,
        &Token {
            hash: sha256(&claim_line(&unfinished)),
            claim: unfinished,
        },
        None,
        Reason::Drop,
        Duration::from_millis(1),
        Instant::now() + Duration::from_secs(15),
    )
    .unwrap();
}

#[test]
fn missing_referenced_state_refuses_without_consuming_or_recreating_it() {
    for missing in [
        "manifest-00000000-0000-4000-8000-000000000002.json",
        "keys-00000000-0000-4000-8000-000000000002.txt",
        "segment-00000000-0000-4000-8000-000000000002.jsonl",
    ] {
        let root = tempfile::tempdir().unwrap();
        let context = context(root.path());
        archived_fixture(&context, &claim(1, Some("old-key")));
        let parent = root.path().join("history/claude-notices");
        let genesis = fs::read(parent.join("ledger.jsonl")).unwrap();
        fs::remove_file(parent.join(missing)).unwrap();
        let consumed = Cell::new(false);
        let result = consume(
            &context,
            claim(9000, Some("fresh")),
            Instant::now() + Duration::from_secs(15),
            || {
                consumed.set(true);
                Ok(())
            },
        );
        assert!(result.is_err(), "missing reference: {missing}");
        assert!(!consumed.get(), "missing reference: {missing}");
        assert!(!parent.join(missing).exists(), "never recreate {missing}");
        assert!(confirmed(root.path(), Instant::now() + Duration::from_secs(15)).is_err());
        assert_eq!(fs::read(parent.join("ledger.jsonl")).unwrap(), genesis);
    }
}

/// Explicit measurement keeps filesystem/cache timing out of ordinary CI assertions.
#[test]
#[ignore = "synthetic archive admission measurement; run explicitly with --ignored --nocapture"]
fn admission_across_many_archive_generations() {
    for archived_count in [1_000, 10_000, 50_000] {
        let root = tempfile::tempdir().unwrap();
        let context = context(root.path());
        let parent = root.path().join("history/claude-notices");
        let mut previous = Vec::new();
        let mut previous_name = "ledger.jsonl".to_string();
        let mut predecessor = Value::Null;
        let mut archive_bytes = 0usize;
        for generation in 1..=archived_count / 1_000 {
            let mut keys = String::new();
            for index in (generation - 1) * 1_000..generation * 1_000 {
                let claim = claim(index, None);
                previous.extend(pair(&claim).0);
                keys.push_str(&format!("o{}\n", sha256(claim.op.as_bytes())));
            }
            private_file(&parent.join(&previous_name), &previous);
            fs::set_permissions(
                parent.join(&previous_name),
                fs::Permissions::from_mode(0o400),
            )
            .unwrap();
            archive_bytes += previous.len();
            let id = format!("{generation:08x}-0000-4000-8000-000000000000");
            let active = format!("segment-{id}.jsonl");
            let index_name = format!("keys-{id}.txt");
            let manifest_name = format!("manifest-{id}.json");
            private_file(&parent.join(&index_name), keys.as_bytes());
            let manifest = serde_json::to_vec(&json!({
                "schema_version":MANIFEST_SCHEMA, "generation":generation,
                "active":active, "predecessor":predecessor,
                "archive":reference(&previous_name,&previous),
                "key_index":reference(&index_name,keys.as_bytes()), "first_seen":{}
            }))
            .unwrap();
            private_file(&parent.join(&manifest_name), &manifest);
            predecessor = reference(&manifest_name, &manifest);
            previous = header(generation as u64, &previous, keys.as_bytes());
            previous_name = active;
        }
        private_file(&parent.join(&previous_name), &previous);
        private_file(
            &parent.join("current.json"),
            &serde_json::to_vec(&json!({
                "schema_version":POINTER_SCHEMA,"manifest":predecessor
            }))
            .unwrap(),
        );
        private_file(
            &parent.join("layout.json"),
            b"{\"schema_version\":\"agent-session.claude-notice-layout.v1\"}\n",
        );
        let started = Instant::now();
        consume(
            &context,
            claim(archived_count + 1, Some("fresh")),
            started + Duration::from_secs(15),
            || Ok(()),
        )
        .unwrap();
        eprintln!(
            "archive-admission claims={archived_count} generations={} bytes={archive_bytes} elapsed_ms={:.3}",
            archived_count / 1_000,
            started.elapsed().as_secs_f64() * 1_000.0
        );
        let consumed = Cell::new(false);
        assert!(
            consume(
                &context,
                claim(0, Some("different-key")),
                Instant::now() + Duration::from_secs(15),
                || {
                    consumed.set(true);
                    Ok(())
                }
            )
            .is_err(),
            "operation in the oldest generation remains reserved"
        );
        assert!(!consumed.get());
    }
}

#[test]
fn undated_finished_claims_keep_their_grace_period_then_leave_the_live_cap() {
    let mut dropped = Vec::new();
    let mut first_seen = serde_json::Map::new();
    for index in 0..MAX_CLAIMS {
        let claim = claim(index, None);
        let (pair, _) = pair(&claim);
        let mut lines = pair.split_inclusive(|byte| *byte == b'\n');
        dropped.extend(lines.next().unwrap());
        let mut terminal: Value = serde_json::from_slice(lines.next().unwrap()).unwrap();
        for field in [
            "native_uuid",
            "row_offset",
            "row_payload_sha256",
            "envelope_sha256",
            "timestamp",
            "confirmation_elapsed_ms",
        ] {
            terminal.as_object_mut().unwrap().remove(field);
        }
        terminal["state"] = json!("dropped");
        terminal["reason"] = json!("drop");
        dropped.extend(serde_json::to_vec(&terminal).unwrap());
        dropped.push(b'\n');
        first_seen.insert(claim.op, json!("2000-01-01T00:00:00Z"));
    }
    let root = tempfile::tempdir().unwrap();
    let context = context(root.path());
    let parent = root.path().join("history/claude-notices");
    private_file(&parent.join("ledger.jsonl"), &dropped);
    let consumed = Cell::new(false);
    let result = consume(
        &context,
        claim(2000, Some("fresh")),
        Instant::now() + Duration::from_secs(15),
        || {
            consumed.set(true);
            Ok(())
        },
    );
    assert_eq!(
        result.err().unwrap().details().unwrap()["reason"],
        "claim-ledger-full"
    );
    assert!(!consumed.get());
    let pointer = fs::read(parent.join("current.json")).unwrap();
    let current: Value = serde_json::from_slice(&pointer).unwrap();
    let manifest: Value = serde_json::from_slice(
        &fs::read(parent.join(current["manifest"]["file"].as_str().unwrap())).unwrap(),
    )
    .unwrap();
    assert_eq!(
        manifest["first_seen"].as_object().unwrap().len(),
        MAX_CLAIMS
    );
    let result = consume(
        &context,
        claim(2000, Some("fresh")),
        Instant::now() + Duration::from_secs(15),
        || Ok(()),
    );
    assert_eq!(
        result.err().unwrap().details().unwrap()["reason"],
        "claim-ledger-full"
    );
    assert_eq!(
        fs::read(parent.join("current.json")).unwrap(),
        pointer,
        "do not reset the grace period"
    );

    let older_root = tempfile::tempdir().unwrap();
    let older = self::context(older_root.path());
    archived_fixture_with_live(
        &older,
        &claim(5000, Some("prior")),
        &dropped,
        Value::Object(first_seen),
    );
    let result = consume(
        &older,
        claim(2000, Some("fresh")),
        Instant::now() + Duration::from_secs(15),
        || Ok(()),
    );
    assert!(
        result.is_ok(),
        "the persisted old observation permits archival: {:?}",
        result.err()
    );
}
