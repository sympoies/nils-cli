use super::*;
use pretty_assertions::assert_eq;

#[test]
fn provider_version_parser_handles_audited_cli_formats() {
    assert_eq!(
        parse_version_triplet("codex-cli 0.144.1"),
        Some((0, 144, 1))
    );
    assert_eq!(
        parse_version_triplet("2.1.206 (Claude Code)"),
        Some((2, 1, 206))
    );
    assert_eq!(parse_version_triplet("development build"), None);
    assert_eq!(audited_floor(AgentKind::Claude), (2, 1, 206));
}

#[test]
fn event_confidence_is_required_by_the_v1_wire_contract() {
    let missing = json!({
        "schema_version": TURN_EVENT_VERSION,
        "event_id": "event-1",
        "runtime_id": "runtime-1",
        "provider": "codex",
        "kind": "progress"
    });
    assert!(serde_json::from_value::<TurnEvent>(missing).is_err());
}

#[test]
fn claude_uncorrelated_progress_does_not_advance_the_exact_replay_window() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let (context, created) = test_session_for_agent(&tmp, AgentKind::Claude);
    let runtime_id = created
        .record
        .runtime
        .as_ref()
        .expect("runtime")
        .launch_id
        .clone();
    let dir = session_dir(&context, &created.record.id);
    let path = dir.join(ACTIVITY_FILE);
    let mut snapshot = read_document(&path).expect("activity snapshot");
    snapshot.seen_event_count = MAX_DEDUPE_EVENTS - 1;
    write_document(&path, &mut snapshot).expect("window-boundary snapshot");

    let progress = normalize_provider_hook(
        AgentKind::Claude,
        None,
        &runtime_id,
        &json!({
            "hook_event_name": "PreToolUse",
            "session_id": "session-1",
            "tool_name": "Bash",
            "tool_use_id": "tool-secret"
        }),
    )
    .expect("progress normalization")
    .expect("recognized progress");
    let working = ingest_event(&context, &created.record.id, progress)
        .expect("uncorrelated Claude progress is ingested at a window boundary");
    assert_eq!(working.turn_state.phase, TurnPhase::Working);
    assert_eq!(
        read_document(&path)
            .expect("progress snapshot")
            .seen_event_count,
        MAX_DEDUPE_EVENTS - 1,
        "uncorrelated Claude progress must not advance the exact replay window index"
    );

    let completed = normalize_provider_hook(
        AgentKind::Claude,
        None,
        &runtime_id,
        &json!({
            "hook_event_name": "Notification",
            "notification_type": "idle_prompt",
            "session_id": "session-1"
        }),
    )
    .expect("completion normalization")
    .expect("recognized completion");
    let waiting = ingest_event(&context, &created.record.id, completed)
        .expect("a lifecycle event takes the next exact window index");
    assert_eq!(waiting.turn_state.phase, TurnPhase::Waiting);
    assert_eq!(
        read_document(&path)
            .expect("completion snapshot")
            .seen_event_count,
        MAX_DEDUPE_EVENTS
    );
}

#[test]
fn claude_progress_pending_journal_repairs_without_exact_replay_slot() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let (context, created) = test_session_for_agent(&tmp, AgentKind::Claude);
    let runtime_id = created
        .record
        .runtime
        .as_ref()
        .expect("runtime")
        .launch_id
        .clone();
    let dir = session_dir(&context, &created.record.id);
    let journal_path = dir.join(ACTIVITY_JOURNAL_FILE);
    fs::create_dir(&journal_path).expect("block journal target");
    let progress = normalize_provider_hook(
        AgentKind::Claude,
        None,
        &runtime_id,
        &json!({
            "hook_event_name": "PreToolUse",
            "session_id": "session-1",
            "tool_name": "Task",
            "tool_use_id": "tool-secret"
        }),
    )
    .expect("progress normalization")
    .expect("recognized progress");

    assert!(
        ingest_event(&context, &created.record.id, progress.clone()).is_err(),
        "blocked journal target must interrupt the split write"
    );
    let pending = read_document(&dir.join(ACTIVITY_FILE)).expect("pending snapshot");
    assert!(pending.pending_journal.is_some());
    assert_eq!(pending.seen_event_count, 0);

    fs::remove_dir(&journal_path).expect("restore journal target");
    let repaired = ingest_event(&context, &created.record.id, progress)
        .expect("repair replay-exempt progress");
    assert!(repaired.duplicate);
    let repaired_snapshot = read_document(&dir.join(ACTIVITY_FILE)).expect("repaired snapshot");
    assert!(repaired_snapshot.pending_journal.is_none());
    assert_eq!(repaired_snapshot.seen_event_count, 0);
    let journal = fs::read_to_string(journal_path).expect("repaired journal");
    assert_eq!(journal.matches("\"kind\":\"progress\"").count(), 1);
}

#[test]
fn exact_replay_horizon_rotates_instead_of_refusing_at_capacity() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let (context, created) = test_session(&tmp);
    let runtime_id = created
        .record
        .runtime
        .as_ref()
        .expect("runtime")
        .launch_id
        .clone();
    let dir = session_dir(&context, &created.record.id);
    let path = dir.join(ACTIVITY_FILE);
    let exact = |index: usize| {
        let mut current = event(TurnEventKind::Progress, &format!("rotating-{index}"));
        current.runtime_id.clone_from(&runtime_id);
        current.provider_turn_id = Some(format!("turn-{index}"));
        current
    };
    let is_duplicate = |index: usize| {
        ingest_event(&context, &created.record.id, exact(index))
            .expect("replay probe")
            .duplicate
    };
    // Cross three table boundaries; each ingests the last event of one
    // window and the first two of the next.
    let boundaries = [
        MAX_DEDUPE_EVENTS,
        2 * MAX_DEDUPE_EVENTS,
        3 * MAX_DEDUPE_EVENTS,
    ];
    for (position, boundary) in boundaries.into_iter().enumerate() {
        let mut snapshot = read_document(&path).expect("activity snapshot");
        snapshot.seen_event_count = boundary - 1;
        write_document(&path, &mut snapshot).expect("boundary snapshot");
        for index in boundary - 1..boundary + 2 {
            let result = ingest_event(&context, &created.record.id, exact(index))
                .expect("an exact-horizon event past capacity is still accepted");
            assert!(!result.duplicate, "event {index} is new");
        }
        assert_eq!(
            read_document(&path).expect("snapshot").seen_event_count,
            boundary + 2
        );
        assert!(is_duplicate(boundary + 1), "the new window deduplicates");
        assert!(
            is_duplicate(boundary - 1),
            "the previous window survives rotation at {boundary}"
        );
        if position > 0 {
            // The table this boundary reset held the window two back.
            let two_windows_back = boundaries[position - 1] - 1;
            assert!(
                !is_duplicate(two_windows_back),
                "event {two_windows_back} left the replay window at {boundary}"
            );
        }
    }
}

#[test]
fn three_windows_of_exact_events_stay_ingestible_and_recent_replays_deduplicate() {
    // The checked owner also removes the nested session fixture on panic.
    // Keep the existing shared session helper's TempDir interface.
    let owner = nils_test_support::tempdir::ScopedTempDir::new();
    let tmp = tempfile::tempdir_in(owner.path()).expect("session fixture");
    let started = Instant::now();
    let check_deadline = |index: usize| {
        assert!(
            started.elapsed() < Duration::from_secs(60),
            "three replay windows exceeded the 60 s deadline at event {index}"
        );
    };
    let (context, created) = test_session(&tmp);
    let runtime = created.record.runtime.as_ref().expect("runtime");
    let runtime_id = &runtime.launch_id;
    let exact = |index: usize| {
        let mut current = event(TurnEventKind::Progress, &format!("window-{index}"));
        current.runtime_id.clone_from(runtime_id);
        current.provider_turn_id = Some(format!("turn-{index}"));
        current
    };
    let ingest = |index: usize| {
        check_deadline(index);
        let result = ingest_event_with_lock(
            &context,
            &created.record.id,
            exact(index),
            ActivityLockMode::Timed(Duration::from_secs(1)),
            EventAdmission::Generic,
        )
        .expect("exact-horizon event is accepted");
        check_deadline(index);
        result
    };
    let dir = session_dir(&context, &created.record.id);
    let snapshot_path = dir.join(ACTIVITY_FILE);
    let replay_path = dir.join(ACTIVITY_REPLAY_FILE);
    // Past the third boundary, so the last window spans both tables.
    let total = 3 * MAX_DEDUPE_EVENTS + MAX_DEDUPE_EVENTS / 2;
    for start in (0..total).step_by(MAX_DEDUPE_EVENTS) {
        let end = (start + MAX_DEDUPE_EVENTS).min(total);
        // Exercise real ingestion at every rotation, including reuse of
        // both full tables. Seed the interior as a persisted fixture:
        // thousands of durable snapshot/journal commits made this a
        // storage benchmark rather than a bounded replay regression.
        assert!(!ingest(start).duplicate, "event {start} is new");
        if start > 0 {
            assert!(ingest(start - 1).duplicate, "the previous window survives");
        }
        if start >= 2 * MAX_DEDUPE_EVENTS {
            let evicted = exact(start - 2 * MAX_DEDUPE_EVENTS);
            assert!(
                !replay_contains(
                    &replay_path,
                    runtime_id,
                    runtime.generation,
                    start + 1,
                    &event_dedupe_key(runtime_id, &evicted.event_id),
                )
                .expect("evicted replay probe"),
                "rotation evicts the window two back"
            );
        }
        let table_path = replay_table_path(&replay_path, replay_table(start));
        let mut table = fs::read(&table_path).expect("initialized replay table");
        for index in start + 1..end - 1 {
            check_deadline(index);
            let key = event_dedupe_key(runtime_id, &exact(index).event_id);
            let offset = (0..REPLAY_SLOT_COUNT)
                .map(|probe| replay_slot(&key, probe) as usize)
                .find(|&offset| {
                    table[offset..offset + REPLAY_SLOT_BYTES]
                        .iter()
                        .all(|byte| *byte == 0)
                })
                .expect("free slot at half load");
            table[offset..offset + REPLAY_SLOT_BYTES].copy_from_slice(&key);
        }
        fs::write(&table_path, table).expect("full-window replay fixture");
        let mut snapshot = read_document(&snapshot_path).expect("activity snapshot");
        snapshot.seen_event_count = end - 1;
        write_document(&snapshot_path, &mut snapshot).expect("full-window snapshot");
        assert!(!ingest(end - 1).duplicate, "the last event is new");
        assert_eq!(
            read_document(&snapshot_path)
                .expect("snapshot")
                .seen_event_count,
            end
        );
        assert!(
            ingest(start).duplicate,
            "the first event survives a full table"
        );
    }
    for index in total - MAX_DEDUPE_EVENTS..total {
        assert!(
            ingest(index).duplicate,
            "event {index} is inside the replay window"
        );
    }
    let two_windows_back = MAX_DEDUPE_EVENTS + 1;
    assert!(
        !ingest(two_windows_back).duplicate,
        "event {two_windows_back} left the replay window"
    );
    check_deadline(total);
    drop(created);
    drop(tmp);
    owner.close().expect("remove replay-window fixture");
}

#[test]
fn a_full_table_zero_index_from_before_the_window_keeps_ingesting() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let (context, created) = test_session(&tmp);
    let runtime = created.record.runtime.as_ref().expect("runtime");
    let runtime_id = runtime.launch_id.clone();
    let generation = runtime.generation;
    let dir = session_dir(&context, &created.record.id);
    let old = |index: usize| {
        let mut current = event(TurnEventKind::Progress, &format!("old-{index}"));
        current.runtime_id.clone_from(&runtime_id);
        current.provider_turn_id = Some(format!("turn-{index}"));
        current
    };

    // What a binary without the window left behind at its capacity refusal:
    // a full table-0 file in the unchanged format and no table-1 file.
    let mut table_zero = vec![0_u8; REPLAY_FILE_BYTES];
    table_zero[..REPLAY_HEADER_BYTES].copy_from_slice(&replay_header(&runtime_id, generation));
    for index in 0..MAX_DEDUPE_EVENTS {
        let key = event_dedupe_key(&runtime_id, &old(index).event_id);
        let offset = (0..REPLAY_SLOT_COUNT)
            .map(|probe| replay_slot(&key, probe) as usize)
            .find(|&offset| {
                table_zero[offset..offset + REPLAY_SLOT_BYTES]
                    .iter()
                    .all(|b| *b == 0)
            })
            .expect("free slot at half load");
        table_zero[offset..offset + REPLAY_SLOT_BYTES].copy_from_slice(&key);
    }
    let replay_path = dir.join(ACTIVITY_REPLAY_FILE);
    fs::write(&replay_path, &table_zero).expect("full table-0 index");
    let path = dir.join(ACTIVITY_FILE);
    let mut snapshot = read_document(&path).expect("activity snapshot");
    snapshot.seen_event_count = MAX_DEDUPE_EVENTS;
    write_document(&path, &mut snapshot).expect("full snapshot");

    let mut next = old(MAX_DEDUPE_EVENTS);
    next.event_id = "after-upgrade".to_string();
    assert!(
        !ingest_event(&context, &created.record.id, next)
            .expect("the next exact event is accepted past the old capacity")
            .duplicate
    );
    for index in [0, MAX_DEDUPE_EVENTS - 1] {
        assert!(
            ingest_event(&context, &created.record.id, old(index))
                .expect("old replay")
                .duplicate,
            "old event {index} still replays as a duplicate after the first rotation"
        );
    }
    let kept = fs::read(&replay_path).expect("table-0 index");
    assert_eq!(kept.len(), REPLAY_FILE_BYTES, "table 0 keeps its format");
    assert_eq!(
        &kept[..REPLAY_HEADER_BYTES],
        &replay_header(&runtime_id, generation)
    );
    assert_eq!(
        fs::metadata(dir.join(ACTIVITY_REPLAY_TABLE_ONE_FILE))
            .expect("table-1 index")
            .len(),
        REPLAY_FILE_BYTES as u64
    );
}

#[test]
fn a_pending_boundary_insert_converges_on_repair() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let (context, created) = test_session(&tmp);
    let runtime_id = created
        .record
        .runtime
        .as_ref()
        .expect("runtime")
        .launch_id
        .clone();
    let dir = session_dir(&context, &created.record.id);
    let path = dir.join(ACTIVITY_FILE);
    let journal_path = dir.join(ACTIVITY_JOURNAL_FILE);
    let exact = |index: usize| {
        let mut current = event(TurnEventKind::Progress, &format!("redo-{index}"));
        current.runtime_id.clone_from(&runtime_id);
        current.provider_turn_id = Some(format!("turn-{index}"));
        current
    };
    let mut snapshot = read_document(&path).expect("activity snapshot");
    snapshot.seen_event_count = MAX_DEDUPE_EVENTS - 1;
    write_document(&path, &mut snapshot).expect("boundary snapshot");
    ingest_event(&context, &created.record.id, exact(MAX_DEDUPE_EVENTS - 1))
        .expect("last event of the first window");

    // Interrupt the first event of the next window after its table reset
    // and insert, before the journal append.
    fs::remove_file(&journal_path).expect("clear journal");
    fs::create_dir(&journal_path).expect("block journal target");
    assert!(
        ingest_event(&context, &created.record.id, exact(MAX_DEDUPE_EVENTS)).is_err(),
        "blocked journal target interrupts the boundary write"
    );
    let pending = read_document(&path).expect("pending snapshot");
    assert!(pending.pending_journal.is_some());
    assert_eq!(pending.seen_event_count, MAX_DEDUPE_EVENTS + 1);

    fs::remove_dir(&journal_path).expect("restore journal target");
    let repaired = ingest_event(&context, &created.record.id, exact(MAX_DEDUPE_EVENTS))
        .expect("repair redoes the boundary insert");
    assert!(
        repaired.duplicate,
        "the redone boundary key is in the new window"
    );
    let snapshot = read_document(&path).expect("repaired snapshot");
    assert!(snapshot.pending_journal.is_none());
    assert_eq!(snapshot.seen_event_count, MAX_DEDUPE_EVENTS + 1);
    assert!(
        ingest_event(&context, &created.record.id, exact(MAX_DEDUPE_EVENTS - 1))
            .expect("previous window replay")
            .duplicate,
        "the redo reset only the new window's table"
    );
    assert!(
        !ingest_event(&context, &created.record.id, exact(MAX_DEDUPE_EVENTS + 1))
            .expect("next event")
            .duplicate
    );
}

#[test]
fn a_stale_or_missing_table_one_before_its_boundary_reset_keeps_the_view_valid() {
    for stale in [true, false] {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let (context, created) = test_session(&tmp);
        let runtime = created.record.runtime.as_ref().expect("runtime");
        let runtime_id = runtime.launch_id.clone();
        let generation = runtime.generation;
        let dir = session_dir(&context, &created.record.id);
        let path = dir.join(ACTIVITY_FILE);
        let journal_path = dir.join(ACTIVITY_JOURNAL_FILE);
        let table_one = dir.join(ACTIVITY_REPLAY_TABLE_ONE_FILE);
        let exact = |index: usize| {
            let mut current = event(TurnEventKind::Progress, &format!("stale-{index}"));
            current.runtime_id.clone_from(&runtime_id);
            current.provider_turn_id = Some(format!("turn-{index}"));
            current
        };
        let mut snapshot = read_document(&path).expect("activity snapshot");
        snapshot.seen_event_count = MAX_DEDUPE_EVENTS - 1;
        write_document(&path, &mut snapshot).expect("boundary snapshot");
        ingest_event(&context, &created.record.id, exact(MAX_DEDUPE_EVENTS - 1))
            .expect("last event of the first window");
        if stale {
            // Left behind by an earlier runtime generation.
            let mut old = vec![0_u8; REPLAY_FILE_BYTES];
            old[..REPLAY_HEADER_BYTES]
                .copy_from_slice(&replay_header("earlier-runtime", generation));
            fs::write(&table_one, &old).expect("stale table one");
        }

        // Stop right after the pending snapshot write for index 4096.
        fs::remove_file(&journal_path).expect("clear journal");
        fs::create_dir(&journal_path).expect("block journal target");
        let mut snapshot = read_document(&path).expect("activity snapshot");
        snapshot.seen_event_count = MAX_DEDUPE_EVENTS + 1;
        snapshot.pending_journal = Some(JournalEntry {
            received_at: "2026-07-10T00:00:00Z".to_string(),
            event: exact(MAX_DEDUPE_EVENTS),
        });
        write_document(&path, &mut snapshot).expect("pending boundary snapshot");
        assert_ne!(
            state_for_view(&context, &created.record)
                .expect("view state")
                .phase,
            TurnPhase::Unknown,
            "stale={stale}: the view stays valid before the boundary reset"
        );

        fs::remove_dir(&journal_path).expect("restore journal target");
        let repaired = ingest_event(&context, &created.record.id, exact(MAX_DEDUPE_EVENTS))
            .expect("repair converges");
        assert!(repaired.duplicate);
        assert_eq!(
            &fs::read(&table_one).expect("reset table one")[..REPLAY_HEADER_BYTES],
            &replay_header(&runtime_id, generation)
        );
        assert_ne!(
            state_for_view(&context, &created.record)
                .expect("view state")
                .phase,
            TurnPhase::Unknown
        );

        // Snapshot and restore carry table one's bytes.
        let captured = capture_snapshot(&context, &created.record.id).expect("capture");
        let before = fs::read(&table_one).expect("table one bytes");
        fs::write(&table_one, vec![0_u8; REPLAY_FILE_BYTES]).expect("mutate table one");
        restore_snapshot(&context, &created.record.id, &captured).expect("restore");
        assert_eq!(fs::read(&table_one).expect("restored table one"), before);
    }
}

/// A table file in the unchanged format holding `keys` at their probe slots.
fn replay_table_with_keys(
    runtime_id: &str,
    generation: u64,
    keys: impl IntoIterator<Item = [u8; REPLAY_SLOT_BYTES]>,
) -> Vec<u8> {
    let mut table = vec![0_u8; REPLAY_FILE_BYTES];
    table[..REPLAY_HEADER_BYTES].copy_from_slice(&replay_header(runtime_id, generation));
    for key in keys {
        let offset = (0..REPLAY_SLOT_COUNT)
            .map(|probe| replay_slot(&key, probe) as usize)
            .find(|&offset| {
                table[offset..offset + REPLAY_SLOT_BYTES]
                    .iter()
                    .all(|byte| *byte == 0)
            })
            .expect("free slot");
        table[offset..offset + REPLAY_SLOT_BYTES].copy_from_slice(&key);
    }
    table
}

#[test]
fn a_missing_or_stale_table_one_after_an_old_repair_self_heals() {
    for stale in [false, true] {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let (context, created) = test_session(&tmp);
        let runtime = created.record.runtime.as_ref().expect("runtime");
        let runtime_id = runtime.launch_id.clone();
        let generation = runtime.generation;
        let dir = session_dir(&context, &created.record.id);
        let exact = |index: usize| {
            let mut current = event(TurnEventKind::Progress, &format!("heal-{index}"));
            current.runtime_id.clone_from(&runtime_id);
            current.provider_turn_id = Some(format!("turn-{index}"));
            current
        };
        // What a binary without the window leaves after repairing the
        // boundary event: every key in table 0, the count past the first
        // window, no pending entry, and no table 1 of this runtime.
        let keys = (0..=MAX_DEDUPE_EVENTS)
            .map(|index| event_dedupe_key(&runtime_id, &exact(index).event_id));
        fs::write(
            dir.join(ACTIVITY_REPLAY_FILE),
            replay_table_with_keys(&runtime_id, generation, keys),
        )
        .expect("table zero");
        let table_one = dir.join(ACTIVITY_REPLAY_TABLE_ONE_FILE);
        if stale {
            fs::write(
                &table_one,
                replay_table_with_keys("earlier-runtime", generation, []),
            )
            .expect("stale table one");
        }
        let path = dir.join(ACTIVITY_FILE);
        let mut snapshot = read_document(&path).expect("activity snapshot");
        snapshot.seen_event_count = MAX_DEDUPE_EVENTS + 1;
        write_document(&path, &mut snapshot).expect("repaired snapshot");

        assert_ne!(
            state_for_view(&context, &created.record)
                .expect("view state")
                .phase,
            TurnPhase::Unknown,
            "stale={stale}: the view accepts a self-healable table one"
        );
        let next = ingest_event(&context, &created.record.id, exact(MAX_DEDUPE_EVENTS + 1))
            .expect("the next exact event is ingested");
        assert!(!next.duplicate);
        assert!(
            ingest_event(&context, &created.record.id, exact(MAX_DEDUPE_EVENTS + 1))
                .expect("replay")
                .duplicate
        );
        assert_eq!(
            &fs::read(&table_one).expect("healed table one")[..REPLAY_HEADER_BYTES],
            &replay_header(&runtime_id, generation)
        );
        assert_ne!(
            state_for_view(&context, &created.record)
                .expect("view state")
                .phase,
            TurnPhase::Unknown
        );
    }
}

#[test]
fn a_failed_boundary_reset_leaves_the_count_unchanged() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let (context, created) = test_session(&tmp);
    let runtime_id = created
        .record
        .runtime
        .as_ref()
        .expect("runtime")
        .launch_id
        .clone();
    let dir = session_dir(&context, &created.record.id);
    let path = dir.join(ACTIVITY_FILE);
    let exact = |index: usize| {
        let mut current = event(TurnEventKind::Progress, &format!("reset-{index}"));
        current.runtime_id.clone_from(&runtime_id);
        current.provider_turn_id = Some(format!("turn-{index}"));
        current
    };
    let mut snapshot = read_document(&path).expect("activity snapshot");
    snapshot.seen_event_count = MAX_DEDUPE_EVENTS - 1;
    write_document(&path, &mut snapshot).expect("boundary snapshot");
    ingest_event(&context, &created.record.id, exact(MAX_DEDUPE_EVENTS - 1))
        .expect("last event of the first window");

    // Table one cannot be created: the boundary reset fails.
    let table_one = dir.join(ACTIVITY_REPLAY_TABLE_ONE_FILE);
    fs::create_dir(&table_one).expect("block table one");
    assert!(ingest_event(&context, &created.record.id, exact(MAX_DEDUPE_EVENTS)).is_err());
    let unchanged = read_document(&path).expect("snapshot after failed reset");
    assert_eq!(
        unchanged.seen_event_count, MAX_DEDUPE_EVENTS,
        "the count that selects the new window is not durable before its reset"
    );
    assert!(unchanged.pending_journal.is_none());

    fs::remove_dir(&table_one).expect("unblock table one");
    assert!(
        !ingest_event(&context, &created.record.id, exact(MAX_DEDUPE_EVENTS))
            .expect("the boundary event converges")
            .duplicate
    );
    assert!(
        ingest_event(&context, &created.record.id, exact(MAX_DEDUPE_EVENTS - 1))
            .expect("previous window replay")
            .duplicate
    );
}

#[test]
fn a_table_left_uncleared_into_the_next_window_self_heals() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let (context, created) = test_session(&tmp);
    let runtime = created.record.runtime.as_ref().expect("runtime");
    let runtime_id = runtime.launch_id.clone();
    let generation = runtime.generation;
    let dir = session_dir(&context, &created.record.id);
    let exact = |index: usize| {
        let mut current = event(TurnEventKind::Progress, &format!("full-{index}"));
        current.runtime_id.clone_from(&runtime_id);
        current.provider_turn_id = Some(format!("turn-{index}"));
        current
    };
    // Table 0 missed its clear at the start of window 2 and filled up.
    let keys = (0..REPLAY_SLOT_COUNT)
        .map(|index| event_dedupe_key(&runtime_id, &format!("uncleared-{index}")));
    fs::write(
        dir.join(ACTIVITY_REPLAY_FILE),
        replay_table_with_keys(&runtime_id, generation, keys),
    )
    .expect("full table zero");
    fs::write(
        dir.join(ACTIVITY_REPLAY_TABLE_ONE_FILE),
        replay_table_with_keys(&runtime_id, generation, []),
    )
    .expect("table one");
    let path = dir.join(ACTIVITY_FILE);
    let mut snapshot = read_document(&path).expect("activity snapshot");
    snapshot.seen_event_count = 2 * MAX_DEDUPE_EVENTS + 1;
    write_document(&path, &mut snapshot).expect("window-two snapshot");

    let next = ingest_event(
        &context,
        &created.record.id,
        exact(2 * MAX_DEDUPE_EVENTS + 1),
    )
    .expect("a full table self-heals instead of refusing");
    assert!(!next.duplicate);
    assert!(
        ingest_event(
            &context,
            &created.record.id,
            exact(2 * MAX_DEDUPE_EVENTS + 1)
        )
        .expect("replay")
        .duplicate
    );
}

#[test]
fn dedupe_horizon_is_independent_from_the_bounded_journal() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let (context, created) = test_session(&tmp);
    let runtime_id = created
        .record
        .runtime
        .as_ref()
        .expect("runtime")
        .launch_id
        .clone();
    let first = event(TurnEventKind::Progress, "event-0");
    for index in 0..=MAX_JOURNAL_EVENTS {
        let mut current = if index == 0 {
            first.clone()
        } else {
            event(TurnEventKind::Progress, &format!("event-{index}"))
        };
        current.runtime_id.clone_from(&runtime_id);
        ingest_event(&context, &created.record.id, current).expect("accepted event");
    }
    let before = activity_status(&context, &created.record.id)
        .expect("status")
        .turn_state
        .revision;
    let mut replay = first;
    replay.runtime_id = runtime_id;
    let result = ingest_event(&context, &created.record.id, replay).expect("duplicate replay");
    assert!(result.duplicate);
    assert_eq!(result.turn_state.revision, before);
    let dir = session_dir(&context, &created.record.id);
    let snapshot = fs::read_to_string(dir.join(ACTIVITY_FILE)).expect("snapshot");
    let journal = fs::read_to_string(dir.join(ACTIVITY_JOURNAL_FILE)).expect("journal");
    assert!(!snapshot.contains("session-1"));
    assert!(!snapshot.contains("turn-1"));
    assert!(!journal.contains("session-1"));
    assert!(!journal.contains("turn-1"));
}

#[test]
fn pending_journal_is_repaired_idempotently_after_a_split_write() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let (context, created) = test_session(&tmp);
    let runtime_id = created
        .record
        .runtime
        .as_ref()
        .expect("runtime")
        .launch_id
        .clone();
    let dir = session_dir(&context, &created.record.id);
    let journal_path = dir.join(ACTIVITY_JOURNAL_FILE);
    fs::create_dir(&journal_path).expect("block journal target");
    let mut progress = event(TurnEventKind::Progress, "split-write-event");
    progress.runtime_id = runtime_id;
    assert!(ingest_event(&context, &created.record.id, progress.clone()).is_err());
    let pending = read_document(&dir.join(ACTIVITY_FILE)).expect("pending snapshot");
    assert!(pending.pending_journal.is_some());

    fs::remove_dir(&journal_path).expect("restore journal target");
    let repaired =
        ingest_event(&context, &created.record.id, progress).expect("repaired duplicate");
    assert!(repaired.duplicate);
    let document = read_document(&dir.join(ACTIVITY_FILE)).expect("repaired snapshot");
    assert!(document.pending_journal.is_none());
    let journal = fs::read_to_string(journal_path).expect("journal");
    assert_eq!(journal.matches("split-write-event").count(), 1);
}

#[test]
fn runtime_generation_mismatch_never_exposes_or_accepts_stale_activity() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let (context, created) = test_session(&tmp);
    let mut progress = event(TurnEventKind::Progress, "generation-one-event");
    progress.runtime_id = created
        .record
        .runtime
        .as_ref()
        .expect("runtime")
        .launch_id
        .clone();
    ingest_event(&context, &created.record.id, progress.clone()).expect("generation one event");

    let mut downgraded_resume = created.record.clone();
    downgraded_resume
        .runtime
        .as_mut()
        .expect("runtime")
        .generation += 1;
    write_session_record(&context, &downgraded_resume).expect("downgraded resume record");
    let status = activity_status(&context, &created.record.id).expect("safe status");
    assert_eq!(status.turn_state.phase, TurnPhase::Unknown);
    progress.event_id = "generation-two-event".to_string();
    assert_eq!(
        ingest_event(&context, &created.record.id, progress)
            .expect_err("stale activity generation")
            .code(),
        "runtime-id-mismatch"
    );
}

#[test]
fn runtime_activation_repairs_pending_journal_before_transition() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let (context, created) = test_session(&tmp);
    let dir = session_dir(&context, &created.record.id);
    let journal_path = dir.join(ACTIVITY_JOURNAL_FILE);
    fs::create_dir(&journal_path).expect("block journal target");
    let mut progress = event(TurnEventKind::Progress, "pre-resume-pending");
    progress.runtime_id = created
        .record
        .runtime
        .as_ref()
        .expect("runtime")
        .launch_id
        .clone();
    assert!(ingest_event(&context, &created.record.id, progress).is_err());
    fs::remove_dir(&journal_path).expect("restore journal target");

    let mut resumed = created.record.clone();
    let runtime = resumed.runtime.as_mut().expect("runtime");
    runtime.generation += 1;
    runtime.launch_id = "runtime-2".to_string();
    runtime.started_at = "2026-07-10T00:02:00Z".to_string();
    write_session_record(&context, &resumed).expect("resumed record");
    activate_runtime(&context, &resumed).expect("activate next generation");

    let journal = fs::read_to_string(journal_path).expect("repaired journal");
    assert_eq!(journal.matches("pre-resume-pending").count(), 1);
    let document = read_document(&dir.join(ACTIVITY_FILE)).expect("new activity");
    assert_eq!(document.runtime_generation, 2);
    assert!(document.pending_journal.is_none());
}

#[test]
fn runtime_activation_preserves_additive_fields_and_quarantines_future_schema() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let (context, created) = test_session(&tmp);
    let dir = session_dir(&context, &created.record.id);
    let path = dir.join(ACTIVITY_FILE);
    let mut value: Value =
        serde_json::from_slice(&fs::read(&path).expect("activity")).expect("activity json");
    value["future_top"] = json!({ "enabled": true });
    value["state"]["future_state"] = json!("preserve-me");
    value["state"]["source"]["future_source"] = json!("preserve-source");
    value["state"]["current_turn"] = json!({
        "provider_turn_id": null,
        "started_at": "2026-07-10T00:01:00Z",
        "future_turn": "preserve-turn"
    });
    fs::write(
        &path,
        serde_json::to_vec_pretty(&value).expect("activity bytes"),
    )
    .expect("extended activity");

    let mut next = created.record.clone();
    let runtime = next.runtime.as_mut().expect("runtime");
    runtime.generation += 1;
    runtime.launch_id = "runtime-2".to_string();
    runtime.started_at = "2026-07-10T00:02:00Z".to_string();
    write_session_record(&context, &next).expect("next record");
    activate_runtime(&context, &next).expect("activate with additive fields");
    let preserved: Value = serde_json::from_slice(&fs::read(&path).expect("preserved activity"))
        .expect("preserved json");
    assert_eq!(preserved["future_top"]["enabled"], true);
    assert_eq!(preserved["state"]["future_state"], "preserve-me");
    assert_eq!(
        preserved["state"]["source"]["future_source"],
        "preserve-source"
    );
    assert_eq!(
        preserved["state"]["last_turn"]["future_turn"],
        "preserve-turn"
    );

    let mut future = preserved;
    future["schema_version"] = json!("agent-session.activity.v99");
    let future_bytes = serde_json::to_vec_pretty(&future).expect("future bytes");
    fs::write(&path, &future_bytes).expect("future activity");
    let runtime = next.runtime.as_mut().expect("runtime");
    runtime.generation += 1;
    runtime.launch_id = "runtime-3".to_string();
    runtime.started_at = "2026-07-10T00:03:00Z".to_string();
    write_session_record(&context, &next).expect("third record");
    activate_runtime(&context, &next).expect("quarantine future activity");

    let quarantine = fs::read_dir(&dir)
        .expect("session files")
        .flatten()
        .map(|entry| entry.path())
        .find(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| {
                    name.starts_with("activity.quarantine.activity-version-unsupported")
                })
        })
        .expect("future activity quarantine");
    assert_eq!(
        fs::read(quarantine).expect("quarantine bytes"),
        future_bytes
    );
    let current = read_document(&path).expect("current activity");
    assert_eq!(current.runtime_generation, 3);
}

#[test]
fn provider_version_probe_times_out_without_blocking_diagnostics() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let binary = tmp.path().join("slow-provider");
    fs::write(&binary, "#!/usr/bin/env sh\nsleep 5\n").expect("slow provider");
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o755)).expect("provider mode");
    let started = Instant::now();
    let probe = probe_version_command(
        binary.to_str().expect("provider path"),
        Duration::from_millis(50),
    );
    assert_eq!(probe.error.as_deref(), Some("timeout"));
    assert!(started.elapsed() < Duration::from_secs(1));
}

#[test]
fn repeated_provider_hook_semantics_are_idempotent() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let (context, created) = test_session(&tmp);
    let runtime_id = created
        .record
        .runtime
        .as_ref()
        .expect("runtime")
        .launch_id
        .clone();
    let mut start = event(TurnEventKind::TurnStarted, "hook-start-1");
    start.runtime_id = runtime_id.clone();
    let first = ingest_event(&context, &created.record.id, start.clone()).expect("first start");
    start.event_id = "hook-start-2".to_string();
    let repeated = ingest_event(&context, &created.record.id, start).expect("repeated start");
    assert!(repeated.duplicate);
    assert_eq!(repeated.turn_state.revision, first.turn_state.revision);

    let mut attention = event(TurnEventKind::AttentionRequested, "hook-attention-1");
    attention.runtime_id = runtime_id.clone();
    attention.attention_id = Some("generated-attention-1".to_string());
    attention.attention_kind = Some("approval".to_string());
    let first =
        ingest_event(&context, &created.record.id, attention.clone()).expect("first attention");
    attention.event_id = "hook-attention-2".to_string();
    attention.attention_id = Some("generated-attention-2".to_string());
    let repeated =
        ingest_event(&context, &created.record.id, attention).expect("repeated attention");
    assert!(repeated.duplicate);
    assert_eq!(repeated.turn_state.revision, first.turn_state.revision);
    assert_eq!(
        repeated
            .turn_state
            .current_turn
            .and_then(|turn| turn.attention)
            .map(|attention| attention.pending_count),
        Some(1)
    );

    let mut complete = event(TurnEventKind::TurnCompleted, "hook-complete-1");
    complete.runtime_id = runtime_id;
    let first =
        ingest_event(&context, &created.record.id, complete.clone()).expect("first completion");
    let mut stop = event(TurnEventKind::StopObserved, "hook-stop-after-completion");
    stop.runtime_id = complete.runtime_id.clone();
    let after_stop =
        ingest_event(&context, &created.record.id, stop).expect("interleaved raw stop");
    complete.event_id = "hook-complete-2".to_string();
    let repeated =
        ingest_event(&context, &created.record.id, complete).expect("repeated completion");
    assert!(repeated.duplicate);
    assert_eq!(
        repeated.turn_state.revision, after_stop.turn_state.revision,
        "intervening non-final observations must not reopen completion dedupe"
    );
    assert!(after_stop.turn_state.revision > first.turn_state.revision);
}

#[test]
fn missing_replay_index_never_reopens_a_nonempty_dedupe_horizon() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let (context, created) = test_session(&tmp);
    let mut progress = event(TurnEventKind::Progress, "durable-event");
    progress.runtime_id = created
        .record
        .runtime
        .as_ref()
        .expect("runtime")
        .launch_id
        .clone();
    ingest_event(&context, &created.record.id, progress.clone()).expect("first event");
    fs::remove_file(session_dir(&context, &created.record.id).join(ACTIVITY_REPLAY_FILE))
        .expect("remove replay index");

    assert!(ingest_event(&context, &created.record.id, progress).is_err());
    assert_eq!(
        state_for_view(&context, &created.record)
            .expect("safe state")
            .phase,
        TurnPhase::Unknown
    );
}

#[test]
fn replay_index_from_another_runtime_is_rejected() {
    let first_tmp = tempfile::TempDir::new().expect("first tempdir");
    let (first_context, first) = test_session(&first_tmp);
    let mut first_event = event(TurnEventKind::Progress, "first-event");
    first_event.runtime_id = first
        .record
        .runtime
        .as_ref()
        .expect("first runtime")
        .launch_id
        .clone();
    ingest_event(&first_context, &first.record.id, first_event.clone()).expect("first event");

    let second_tmp = tempfile::TempDir::new().expect("second tempdir");
    let (second_context, second) = test_session(&second_tmp);
    let mut second_event = event(TurnEventKind::Progress, "second-event");
    second_event.runtime_id = second
        .record
        .runtime
        .as_ref()
        .expect("second runtime")
        .launch_id
        .clone();
    ingest_event(&second_context, &second.record.id, second_event).expect("second event");

    fs::copy(
        session_dir(&second_context, &second.record.id).join(ACTIVITY_REPLAY_FILE),
        session_dir(&first_context, &first.record.id).join(ACTIVITY_REPLAY_FILE),
    )
    .expect("swap same-size replay index");
    assert!(ingest_event(&first_context, &first.record.id, first_event).is_err());
    assert_eq!(
        state_for_view(&first_context, &first.record)
            .expect("safe state")
            .phase,
        TurnPhase::Unknown
    );
}

#[test]
fn configured_provider_specs_require_the_owned_timeout() {
    let codex = json!({
        "hooks": {
            "UserPromptSubmit": [{
                "hooks": [{
                    "type": "command",
                    "command": owned_command(AgentKind::Codex, None),
                    "timeout": 1
                }]
            }]
        }
    });
    assert!(!json_has_spec(
        &codex,
        AgentKind::Codex,
        provider_specs(AgentKind::Codex)[0]
    ));
    let permission_command = owned_command(AgentKind::Codex, Some("PermissionRequest"));
    assert!(permission_command.contains("AGENT_SESSION_ATTENTION_AUTHORITY"));
    assert!(permission_command.contains("= protocol"));
    assert!(permission_command.contains("exec agent-session activity hook --agent codex"));

    let claude_specs = provider_specs(AgentKind::Claude);
    assert!(
        claude_specs
            .iter()
            .any(|spec| spec.event == "PreToolUse" && spec.matcher.is_none())
    );
    assert!(
        claude_specs
            .iter()
            .any(|spec| { spec.event == "PreToolUse" && spec.matcher == Some("AskUserQuestion") })
    );
    assert!(
        claude_specs
            .iter()
            .any(|spec| { spec.event == "PostToolUse" && spec.matcher == Some("AskUserQuestion") })
    );
    assert!(claude_specs.iter().any(|spec| {
        spec.event == "PostToolUseFailure" && spec.matcher == Some("AskUserQuestion")
    }));
    assert!(claude_specs.iter().any(|spec| spec.event == "Elicitation"));
    assert!(
        claude_specs
            .iter()
            .any(|spec| spec.event == "ElicitationResult")
    );
    assert!(
        !claude_specs
            .iter()
            .any(|spec| spec.event == "SubagentStop" && spec.matcher.is_none())
    );
    assert!(
        retired_provider_specs(AgentKind::Claude)
            .iter()
            .any(|spec| spec.event == "SubagentStop" && spec.matcher.is_none())
    );
}

#[test]
fn helper_resolution_requires_an_executable_on_the_hook_path() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let helper = tmp.path().join("agent-session");
    fs::write(&helper, "fixture").expect("helper");
    fs::set_permissions(&helper, fs::Permissions::from_mode(0o600)).expect("non-executable");
    assert!(!command_resolves_on_path(
        "agent-session",
        Some(tmp.path().as_os_str())
    ));
    fs::set_permissions(&helper, fs::Permissions::from_mode(0o700)).expect("executable");
    assert!(command_resolves_on_path(
        "agent-session",
        Some(tmp.path().as_os_str())
    ));
    assert!(!command_resolves_on_path("agent-session", None));
}

#[test]
fn codex_marker_lines_inside_multiline_strings_are_not_owned_blocks() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let path = tmp.path().join("config.toml");
    for quotes in ["\"\"\"", "'''"] {
        let raw = format!(
            "note = {quotes}\n{CODEX_HOOK_BLOCK_START}\nprivate\n{CODEX_HOOK_BLOCK_END}\n{quotes}\n"
        );

        let analysis = analyze_codex_toml_hooks(&path, &raw).expect("marker-shaped value");
        assert_eq!(analysis.marker_layout, CodexTomlHookMarkerLayout::Absent);
    }
}

#[test]
fn codex_single_orphan_marker_is_an_owned_repair_fragment() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let path = tmp.path().join("config.toml");
    for (marker, expected_layout) in [
        (
            CODEX_HOOK_BLOCK_START,
            CodexTomlHookMarkerLayout::OrphanStart,
        ),
        (CODEX_HOOK_BLOCK_END, CodexTomlHookMarkerLayout::OrphanEnd),
    ] {
        let raw = format!("keep = true\n{marker}\n");
        let analysis = analyze_codex_toml_hooks(&path, &raw).expect("owned orphan marker");
        assert_eq!(analysis.marker_layout, expected_layout);
    }
}

#[test]
fn codex_duplicate_or_reversed_markers_remain_ambiguous() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let path = tmp.path().join("config.toml");
    for raw in [
        format!("{CODEX_HOOK_BLOCK_START}\n{CODEX_HOOK_BLOCK_START}\n"),
        format!("{CODEX_HOOK_BLOCK_END}\n{CODEX_HOOK_BLOCK_END}\n"),
        format!("{CODEX_HOOK_BLOCK_END}\n{CODEX_HOOK_BLOCK_START}\n"),
    ] {
        assert_eq!(
            analyze_codex_toml_hooks(&path, &raw)
                .expect_err("ambiguous marker layout")
                .code(),
            "provider-config-invalid"
        );
    }
}

#[test]
fn codex_inline_permission_source_guard_rejects_noncanonical_reporters() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let config_path = tmp.path().join("config.toml");
    let canonical = format!(
        "[[hooks.PermissionRequest]]\n\n[[hooks.PermissionRequest.hooks]]\ntype = \"command\"\ncommand = {}\ntimeout = 5\n",
        toml_edit::Value::from(owned_command(AgentKind::Codex, Some("PermissionRequest")))
    );
    fs::write(&config_path, &canonical).expect("owned inline hooks");
    assert!(codex_toml_permission_source_guard(&config_path));

    for command in [
        owned_command(AgentKind::Codex, Some("PermissionRequest")),
        "agent-session activity hook --agent=codex".to_string(),
    ] {
        let mut duplicate = canonical.clone();
        duplicate.push_str(&format!(
                "\n[[hooks.PermissionRequest]]\n\n[[hooks.PermissionRequest.hooks]]\ntype = \"command\"\ncommand = {}\ntimeout = 5\n",
                toml_edit::Value::from(command)
            ));
        fs::write(&config_path, duplicate).expect("duplicate reporter");
        assert!(!codex_toml_permission_source_guard(&config_path));
    }

    for matcher in ["5", "false", "[]", "{}"] {
        let malformed = canonical.replacen(
            "[[hooks.PermissionRequest]]",
            &format!("[[hooks.PermissionRequest]]\nmatcher = {matcher}"),
            1,
        );
        fs::write(&config_path, malformed).expect("malformed matcher");
        assert!(
            !codex_toml_permission_source_guard(&config_path),
            "matcher {matcher} must fail closed"
        );
    }
}

#[test]
fn codex_json_permission_source_guard_rejects_noncanonical_reporters() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let path = tmp.path().join("hooks.json");
    let canonical = json!({
        "hooks": {
            "PermissionRequest": [{
                "hooks": [{
                    "type": "command",
                    "command": owned_command(AgentKind::Codex, Some("PermissionRequest")),
                    "timeout": 5
                }]
            }]
        }
    });
    fs::write(
        &path,
        serde_json::to_vec_pretty(&canonical).expect("JSON bytes"),
    )
    .expect("JSON hooks");
    assert!(codex_json_permission_source_guard(&path));

    for matcher in [Value::Null, json!(5), json!(false), json!([]), json!({})] {
        let mut malformed = canonical.clone();
        malformed["hooks"]["PermissionRequest"][0]["matcher"] = matcher;
        fs::write(
            &path,
            serde_json::to_vec_pretty(&malformed).expect("JSON bytes"),
        )
        .expect("malformed matcher");
        assert!(!codex_json_permission_source_guard(&path));
    }

    let mut value = canonical;
    value["hooks"]["PermissionRequest"][0]["hooks"]
        .as_array_mut()
        .expect("PermissionRequest handlers")
        .push(json!({
            "type": "command",
            "command": "agent-session activity hook --agent=codex",
            "timeout": 5
        }));
    fs::write(
        &path,
        serde_json::to_vec_pretty(&value).expect("JSON bytes"),
    )
    .expect("duplicate reporter");

    assert!(!codex_json_permission_source_guard(&path));
}

#[test]
fn doctor_selects_the_newest_active_runtime_diagnostic() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let (context, created) = test_session(&tmp);
    let write_diagnostic =
        |record: &SessionRecord, observed_at: &str, code: &str, runtime_id: &str| {
            let diagnostic = ActivityDiagnostic {
                schema_version: "agent-session.activity-diagnostic.v1".to_string(),
                provider: record.agent.clone(),
                runtime_id: runtime_id.to_string(),
                runtime_generation: record.runtime.as_ref().expect("runtime").generation,
                code: code.to_string(),
                observed_at: observed_at.to_string(),
            };
            write_atomic(
                &session_dir(&context, &record.id).join(ACTIVITY_DIAGNOSTIC_FILE),
                &serde_json::to_vec_pretty(&diagnostic).expect("diagnostic json"),
                SECRET_FILE_MODE,
            )
            .expect("diagnostic");
        };
    let first_runtime = created
        .record
        .runtime
        .as_ref()
        .expect("first runtime")
        .launch_id
        .clone();
    write_diagnostic(
        &created.record,
        "2026-07-10T00:01:00Z",
        "older-error",
        &first_runtime,
    );

    let mut second = created.record.clone();
    second.id = "activity-test-2".to_string();
    second.tmux_session = "activity-test-2".to_string();
    let runtime = second.runtime.as_mut().expect("second runtime");
    runtime.launch_id = "runtime-second".to_string();
    runtime.generation = 2;
    fs::create_dir_all(session_dir(&context, &second.id)).expect("second session dir");
    write_session_record(&context, &second).expect("second record");
    write_diagnostic(
        &second,
        "2026-07-10T00:02:00Z",
        "newer-error",
        "runtime-second",
    );

    let mut stale = second.clone();
    stale.id = "activity-test-3".to_string();
    stale.tmux_session = "activity-test-3".to_string();
    stale.runtime.as_mut().expect("stale runtime").launch_id = "runtime-third".to_string();
    fs::create_dir_all(session_dir(&context, &stale.id)).expect("third session dir");
    write_session_record(&context, &stale).expect("third record");
    write_diagnostic(
        &stale,
        "2026-07-10T00:03:00Z",
        "stale-error",
        "wrong-runtime",
    );

    let summary = latest_provider_activity(&context)
        .remove("codex")
        .expect("Codex summary");
    assert_eq!(
        summary.last_error,
        Some((
            "2026-07-10T00:02:00Z".to_string(),
            "newer-error".to_string()
        ))
    );
}

#[test]
fn journal_is_bounded_and_metadata_only() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let path = tmp.path().join(ACTIVITY_JOURNAL_FILE);
    for index in 0..(MAX_JOURNAL_EVENTS + 20) {
        let event = event(TurnEventKind::Progress, &format!("event-{index}"));
        append_journal(&path, &event, "2026-07-10T00:00:00Z").expect("append");
    }
    let contents = fs::read_to_string(&path).expect("journal");
    assert!(contents.lines().count() <= MAX_JOURNAL_EVENTS);
    assert!(contents.len() <= MAX_JOURNAL_BYTES);
    assert!(!contents.contains("prompt"));
    assert!(!contents.contains("tool_input"));
}

#[test]
fn journal_idempotency_is_scoped_to_the_runtime_generation() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let path = tmp.path().join(ACTIVITY_JOURNAL_FILE);
    let first = event(TurnEventKind::Progress, "reused-event-id");
    append_journal(&path, &first, "2026-07-10T00:00:00Z").expect("first runtime");
    let mut second = first;
    second.runtime_id = "runtime-2".to_string();
    append_journal(&path, &second, "2026-07-10T00:01:00Z").expect("second runtime");
    let entries = fs::read_to_string(path).expect("journal");
    assert_eq!(entries.matches("reused-event-id").count(), 2);
    assert!(entries.contains("runtime-1"));
    assert!(entries.contains("runtime-2"));
}

#[test]
fn frozen_normalized_fixtures_parse_and_contain_no_content_keys() {
    let events = include_str!("../../../tests/fixtures/activity/turn-events.jsonl");
    for line in events.lines() {
        let event: TurnEvent = serde_json::from_str(line).expect("turn event fixture");
        validate_event(&event, EventAdmission::Generic).expect("valid turn event fixture");
    }
    let states: Vec<TurnState> = serde_json::from_str(include_str!(
        "../../../tests/fixtures/activity/turn-states.json"
    ))
    .expect("turn state fixtures");
    assert_eq!(states.len(), 3);
    assert_eq!(
        states[1]
            .current_turn
            .as_ref()
            .and_then(|turn| turn.last_progress_at.as_deref()),
        Some("2026-07-10T00:00:03Z")
    );

    for fixture in [
        include_str!("../../../tests/fixtures/activity/codex-events.jsonl"),
        include_str!("../../../tests/fixtures/activity/claude-events.jsonl"),
        include_str!("../../../tests/fixtures/activity/dsh-events.jsonl"),
        events,
    ] {
        for forbidden in [
            "\"prompt\":",
            "assistant_response",
            "last_assistant_message",
            "tool_input",
            "tool_response",
            "transcript_path",
            "\"command\":",
            "\"questions\":",
            "\"options\":",
            "\"answers\":",
            "token",
        ] {
            assert!(!fixture.contains(forbidden), "forbidden key {forbidden}");
        }
    }
}

#[test]
fn provider_capacity_is_reserved_for_internal_protocol_v2_admission() {
    let mut capacity = event(TurnEventKind::TurnFailed, "provider-capacity");
    capacity.confidence = Confidence::Authoritative;
    capacity.source_kind = SourceKind::ProviderHook;
    capacity.failure_reason = Some("provider_capacity".to_string());

    let v1_error = validate_event(&capacity, EventAdmission::Generic)
        .expect_err("the closed public v1 failure union must reject provider capacity");
    assert_eq!(v1_error.code(), "activity-failure-reason-invalid");

    capacity.schema_version = CODEX_PROTOCOL_TURN_EVENT_VERSION.to_string();
    let generic_v2_error = validate_event(&capacity, EventAdmission::Generic)
        .expect_err("generic activity ingress must not impersonate provider protocol v2");
    assert_eq!(generic_v2_error.code(), "unsupported-turn-event-version");
    validate_event(&capacity, EventAdmission::CodexProtocol)
        .expect("internal protocol admission accepts the exact v2 capacity reason");
}
