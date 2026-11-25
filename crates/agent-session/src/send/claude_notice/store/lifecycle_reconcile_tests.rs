use super::*;
use pretty_assertions::assert_eq;

#[test]
fn full_unfinished_ledger_seals_expired_claims_without_replay() {
    let root = tempfile::tempdir().unwrap();
    let context = context(root.path());
    let expired = jiff::Timestamp::now().as_second() as u64 - 121;
    let timestamp = uuid::Timestamp::from_unix(uuid::NoContext, expired, 0);
    let mut first_seen = serde_json::Map::new();
    let mut bytes = Vec::new();
    let mut stale_key_claim = None;
    for index in 0..MAX_CLAIMS {
        let mut record = claim(index, (index == 0).then_some("stale-key"));
        if index == 0 {
            stale_key_claim = Some(record.clone());
            first_seen.insert(record.op.clone(), json!("2000-01-01T00:00:00Z"));
        } else {
            record.op = uuid::Uuid::new_v7(timestamp).to_string();
        }
        bytes.extend(claim_line(&record));
        bytes.push(b'\n');
    }
    archived_fixture_with_live(
        &context,
        &claim(5000, Some("archived")),
        &bytes,
        Value::Object(first_seen),
    );
    let deadline = Instant::now() + Duration::from_secs(15);
    let admitted = consume(&context, claim(6000, Some("fresh")), deadline, || Ok(()));
    assert!(
        admitted.is_ok(),
        "a full ledger must release expired claims: {:?}",
        admitted.err()
    );

    let mut ledger = Ledger::open(&context.state_dir, false, deadline)
        .unwrap()
        .unwrap();
    let mut scan = ledger.scan(deadline).unwrap();
    assert_eq!(scan.claims.len(), 1);
    assert!(scan.terminals.is_empty());
    lifecycle::merge_archived(&ledger, &mut scan, deadline).unwrap();
    assert_eq!(
        scan.terminals
            .values()
            .filter(|terminal| terminal.state == State::Unknown)
            .count(),
        MAX_CLAIMS
    );
    assert_eq!(
        scan.terminals
            .values()
            .filter(|terminal| terminal.state == State::Submitted)
            .count(),
        1
    );
    let stale = stale_key_claim.unwrap();
    let token = Token {
        hash: sha256(&claim_line(&stale)),
        claim: stale,
    };
    drop(ledger);
    let replay = consume(&context, claim(6001, Some("stale-key")), deadline, || {
        panic!("the expired claim's caller key is still consumed")
    });
    assert_eq!(replay.err().unwrap().code(), "send-idempotency-key-used");
    assert!(
        finish(
            &context,
            &token,
            None,
            Reason::Drop,
            Duration::ZERO,
            deadline
        )
        .is_err()
    );
}

#[test]
fn reconciliation_retires_stopped_and_replaced_launches_without_losing_proof() {
    for replacement in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let context = context(root.path());
        let deadline = Instant::now() + Duration::from_secs(15);
        let mut session: crate::SessionRecord = serde_json::from_value(json!({
            "schema_version": crate::SESSION_DOCUMENT_VERSION,
            "id": "synthetic",
            "agent": "claude",
            "mode": "interactive",
            "cwd": root.path(),
            "tmux_session": "synthetic",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z",
            "runtime": {
                "kind": "tmux",
                "tmux_session": "synthetic",
                "generation": 1,
                "started_at": "2026-01-01T00:00:00Z",
                "launch_id": "runtime-one"
            }
        }))
        .unwrap();
        crate::write_session_record(&context, &session).unwrap();
        let claim_for_launch = |index, key| {
            let mut pending = claim(index, Some(key));
            pending.op = uuid::Uuid::now_v7().to_string();
            pending
        };
        let stranded = consume(&context, claim_for_launch(1, "stranded"), deadline, || {
            Ok(())
        })
        .unwrap();
        let submitted = consume(&context, claim_for_launch(2, "submitted"), deadline, || {
            Ok(())
        })
        .unwrap();
        let (_, row) = pair(&submitted.claim);
        let proof = Proof {
            row: crate::provider_prompt::native_notice::row(&row, &submitted.claim.binding)
                .unwrap(),
            row_offset: submitted.claim.baseline.eof + 4,
            row_payload_sha256: sha256(&serde_json::to_vec(&row).unwrap()),
            codex: None,
        };
        finish(
            &context,
            &submitted,
            Some(&proof),
            Reason::Guard,
            Duration::from_millis(1),
            deadline,
        )
        .unwrap();

        reconcile(&context).unwrap();
        let mut ledger = Ledger::open(&context.state_dir, false, deadline)
            .unwrap()
            .unwrap();
        let before = ledger.scan(deadline).unwrap();
        assert!(before.claims.contains_key(&stranded.claim.op));
        assert!(!before.terminals.contains_key(&stranded.claim.op));
        drop(ledger);

        if replacement {
            let runtime = session.runtime.as_mut().unwrap();
            runtime.generation += 1;
            runtime.launch_id = "runtime-two".into();
        } else {
            session.runtime = None;
        }
        crate::write_session_record(&context, &session).unwrap();
        reconcile(&context).unwrap();

        let mut ledger = Ledger::open(&context.state_dir, false, deadline)
            .unwrap()
            .unwrap();
        let mut live = ledger.scan(deadline).unwrap();
        assert!(!live.claims.contains_key(&stranded.claim.op));
        assert_eq!(live.terminals[&submitted.claim.op].state, State::Submitted);
        lifecycle::merge_archived(&ledger, &mut live, deadline).unwrap();
        let terminal = &live.terminals[&stranded.claim.op];
        assert_eq!(terminal.state, State::Unknown);
        assert!(matches!(terminal.reason, Some(Reason::RuntimeRetired)));
        assert!(terminal.confirmed(&stranded.claim).is_none());
        assert!(
            live.terminals[&submitted.claim.op]
                .confirmed(&submitted.claim)
                .is_some()
        );
    }
}
