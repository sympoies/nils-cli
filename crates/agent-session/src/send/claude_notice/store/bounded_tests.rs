//! Bounded I/O regression and explicit scale measurements; fixture state only.
use super::*;
use pretty_assertions::assert_eq;

fn archived_claim(index: usize) -> Claim {
    let mut claim = claim(index, Some(&format!("key-{index}")));
    claim.binding.provider_session_id = "11111111-1111-4111-8111-111111111111".into();
    claim.history_id = crate::provider_history::stable_history_id(
        "claude",
        None,
        &claim.binding.provider_session_id,
    );
    claim.baseline.eof = index as u64 * 4096;
    claim
}

pub(super) fn seed(root: &Path, count: usize, width: usize) -> CliContext {
    let context = context(root);
    let parent = root.join("history/claude-notices");
    let mut previous = Vec::new();
    let mut previous_name = "ledger.jsonl".to_string();
    let mut predecessor = Value::Null;
    for generation in 1..=count.div_ceil(width) {
        let mut keys = String::new();
        for index in (generation - 1) * width..count.min(generation * width) {
            let claim = archived_claim(index);
            previous.extend(pair(&claim).0);
            let key = format!(
                "k{}",
                sha256(
                    format!(
                        "agent-session.claude-notice-key.v1\0{}\0{}",
                        claim.binding.launch.session_id,
                        claim.caller_key_sha256.as_ref().unwrap()
                    )
                    .as_bytes()
                )
            );
            keys.push_str(&format!("{key}\no{}\n", sha256(claim.op.as_bytes())));
        }
        private_file(&parent.join(&previous_name), &previous);
        fs::set_permissions(
            parent.join(&previous_name),
            fs::Permissions::from_mode(0o400),
        )
        .unwrap();
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
    context
}

#[cfg(target_os = "linux")]
fn read_bytes() -> u64 {
    let stats = fs::read_to_string("/proc/thread-self/io").unwrap();
    stats
        .lines()
        .find_map(|line| line.strip_prefix("rchar: "))
        .unwrap()
        .trim()
        .parse()
        .unwrap()
}

#[cfg(target_os = "linux")]
#[test]
fn admission_does_not_read_all_checkpointed_archive_bytes() {
    let root = tempfile::tempdir().unwrap();
    let context = seed(root.path(), 10_000, 1_000);
    consume(
        &context,
        claim(20_000, Some("warmup")),
        Instant::now() + Duration::from_secs(15),
        || Ok(()),
    )
    .unwrap();
    let before = read_bytes();
    consume(
        &context,
        claim(20_001, Some("measured")),
        Instant::now() + Duration::from_secs(15),
        || Ok(()),
    )
    .unwrap();
    let read = read_bytes() - before;
    assert!(
        read < 256 * 1024,
        "admission read {read} bytes; archived claims must not be rescanned"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn history_does_not_read_all_checkpointed_archive_bytes() {
    let root = tempfile::tempdir().unwrap();
    let context = seed(root.path(), 10_000, 1_000);
    consume(
        &context,
        claim(20_000, Some("upgrade")),
        Instant::now() + Duration::from_secs(15),
        || Ok(()),
    )
    .unwrap();
    let (catalog, history) = catalog(root.path());
    history_page(&catalog, &history);
    let before = read_bytes();
    history_page(&catalog, &history);
    let read = read_bytes() - before;
    assert!(
        read < 256 * 1024,
        "History read {read} bytes; unrelated archived claims must not be rescanned"
    );
}

fn catalog(root: &Path) -> (crate::provider_history::HistoryCatalog, String) {
    use crate::provider_history::*;
    let proof = archived_claim(0);
    let (_, row) = pair(&proof);
    let directory = root.join("provider/project");
    fs::create_dir_all(&directory).unwrap();
    fs::write(
        directory.join(format!("{}.jsonl", proof.binding.provider_session_id)),
        format!(" \n \n{}\n", serde_json::to_string(&row).unwrap()),
    )
    .unwrap();
    let catalog = HistoryCatalog::new(
        vec![HistorySource {
            provider: "claude".into(),
            agent_profile: None,
            root: root.join("provider"),
        }],
        archive_root(root),
        star_root(root),
    )
    .with_session_state_dir(root.to_path_buf());
    (catalog, proof.history_id)
}

fn history_page(catalog: &crate::provider_history::HistoryCatalog, history: &str) {
    use crate::provider_history::HistoryMessageDirection;
    let page = catalog
        .messages(history, None, 10, HistoryMessageDirection::Latest)
        .unwrap();
    assert_eq!(page.messages.len(), 1);
    assert_eq!(page.messages[0].role, "notice");
}

#[test]
fn first_writer_builds_checkpoint_and_preserves_originals() {
    let root = tempfile::tempdir().unwrap();
    let context = seed(root.path(), 10_000, 1_000);
    let parent = root.path().join("history/claude-notices");
    let before = fs::read(parent.join("ledger.jsonl")).unwrap();
    let consumed = Cell::new(false);
    consume(
        &context,
        claim(20_000, Some("retry")),
        Instant::now() + Duration::from_secs(15),
        || {
            assert!(
                parent.join("checkpoint.json").is_file(),
                "migration must finish before consuming the key"
            );
            consumed.set(true);
            Ok(())
        },
    )
    .unwrap();
    assert!(consumed.get());
    assert!(parent.join("checkpoint.json").is_file());
    assert_eq!(fs::read(parent.join("ledger.jsonl")).unwrap(), before);
    let mut reused = archived_claim(0);
    reused.op = claim(30_000, None).op;
    assert_eq!(
        consume(
            &context,
            reused,
            Instant::now() + Duration::from_secs(15),
            || panic!("archived key must refuse")
        )
        .err()
        .unwrap()
        .code(),
        "send-idempotency-key-used"
    );
}

#[test]
fn checkpoint_damage_refuses_before_the_key_callback() {
    use std::os::unix::fs::FileExt;
    for damage in [
        "missing-root",
        "missing-index",
        "root-page",
        "truncated-index",
        "wrong-mode",
    ] {
        let root = tempfile::tempdir().unwrap();
        let context = seed(root.path(), 100, 20);
        consume(
            &context,
            claim(20_000, Some("upgrade")),
            Instant::now() + Duration::from_secs(15),
            || Ok(()),
        )
        .unwrap();
        let parent = root.path().join("history/claude-notices");
        let checkpoint: Value =
            serde_json::from_slice(&fs::read(parent.join("checkpoint.json")).unwrap()).unwrap();
        let index = parent.join(checkpoint["index"].as_str().unwrap());
        match damage {
            "missing-root" => fs::remove_file(parent.join("checkpoint.json")).unwrap(),
            "missing-index" => fs::remove_file(&index).unwrap(),
            "root-page" => {
                OpenOptions::new()
                    .write(true)
                    .open(&index)
                    .unwrap()
                    .write_all_at(b"!", checkpoint["root"]["offset"].as_u64().unwrap())
                    .unwrap();
            }
            "truncated-index" => OpenOptions::new()
                .write(true)
                .open(&index)
                .unwrap()
                .set_len(0)
                .unwrap(),
            "wrong-mode" => fs::set_permissions(&index, fs::Permissions::from_mode(0o644)).unwrap(),
            _ => unreachable!(),
        }
        let result = consume(
            &context,
            claim(20_001, Some("measured")),
            Instant::now() + Duration::from_secs(15),
            || panic!("damage {damage} must refuse before key consumption"),
        );
        assert!(result.is_err(), "{damage}");
        assert!(
            Reader::open(
                root.path(),
                &archived_claim(0).history_id,
                Instant::now() + Duration::from_secs(15)
            )
            .and_then(|r| r.unwrap().proof(4))
            .is_err(),
            "{damage}"
        );
    }
}

#[test]
fn full_audit_detects_corruption_of_unvisited_checkpointed_originals() {
    let root = tempfile::tempdir().unwrap();
    let context = seed(root.path(), 100, 20);
    consume(
        &context,
        claim(20_000, Some("upgrade")),
        Instant::now() + Duration::from_secs(15),
        || Ok(()),
    )
    .unwrap();
    lifecycle::checkpoint::audit(root.path(), Instant::now() + Duration::from_secs(15)).unwrap();
    let original = root.path().join("history/claude-notices/ledger.jsonl");
    fs::set_permissions(&original, fs::Permissions::from_mode(0o600)).unwrap();
    OpenOptions::new()
        .append(true)
        .open(&original)
        .unwrap()
        .write_all(b" ")
        .unwrap();
    fs::set_permissions(&original, fs::Permissions::from_mode(0o400)).unwrap();
    // Per-call authority is the authenticated checkpoint; full original-file
    // corruption detection is deliberately the explicit audit obligation.
    consume(
        &context,
        claim(20_001, Some("after-corruption")),
        Instant::now() + Duration::from_secs(15),
        || Ok(()),
    )
    .unwrap();
    assert!(
        lifecycle::checkpoint::audit(root.path(), Instant::now() + Duration::from_secs(15))
            .is_err()
    );
}

#[test]
fn interrupted_checkpoint_publication_recovers_one_verified_generation() {
    let root = tempfile::tempdir().unwrap();
    let context = seed(root.path(), 100, 20);
    let parent = root.path().join("history/claude-notices");
    consume(
        &context,
        claim(20_000, Some("upgrade")),
        Instant::now() + Duration::from_secs(15),
        || Ok(()),
    )
    .unwrap();
    let checkpoint = fs::read(parent.join("checkpoint.json")).unwrap();
    let pointer: Value =
        serde_json::from_slice(&fs::read(parent.join("current.json")).unwrap()).unwrap();
    let manifest: Value = serde_json::from_slice(
        &fs::read(parent.join(pointer["manifest"]["file"].as_str().unwrap())).unwrap(),
    )
    .unwrap();
    let active = parent.join(manifest["active"].as_str().unwrap());
    let mut file = OpenOptions::new().append(true).open(&active).unwrap();
    for index in 0..MAX_CLAIMS - 1 {
        file.write_all(&pair(&claim(30_000 + index, None)).0)
            .unwrap();
    }
    drop(file);
    consume(
        &context,
        claim(40_000, Some("rotation")),
        Instant::now() + Duration::from_secs(15),
        || Ok(()),
    )
    .unwrap();
    let sealed_bytes = fs::read(&active).unwrap();
    // Model a crash after current.json publication and sealing, before the
    // new checkpoint root was committed; stale/unreferenced pages confer no authority.
    fs::write(parent.join("checkpoint.json"), checkpoint).unwrap();
    assert!(
        Reader::open(
            root.path(),
            &archived_claim(0).history_id,
            Instant::now() + Duration::from_secs(15)
        )
        .is_err()
    );
    consume(
        &context,
        claim(40_001, Some("recovery")),
        Instant::now() + Duration::from_secs(15),
        || Ok(()),
    )
    .unwrap();
    assert_eq!(fs::read(&active).unwrap(), sealed_bytes);
    lifecycle::checkpoint::audit(root.path(), Instant::now() + Duration::from_secs(15)).unwrap();
    let mut reused = archived_claim(0);
    reused.op = claim(50_000, None).op;
    assert_eq!(
        consume(
            &context,
            reused,
            Instant::now() + Duration::from_secs(15),
            || panic!("old key must survive recovery")
        )
        .err()
        .unwrap()
        .code(),
        "send-idempotency-key-used"
    );
    fs::remove_file(parent.join("layout.json")).unwrap();
    consume(
        &context,
        claim(40_002, Some("marker-recovery")),
        Instant::now() + Duration::from_secs(15),
        || Ok(()),
    )
    .unwrap();
}

#[test]
#[ignore = "explicit cold/warm 10k/100k/1M archive measurements, outside ordinary CI"]
fn admission_and_history_scale_measurement() {
    for width in [1_000, 20] {
        for count in [10_000, 100_000, 1_000_000] {
            let root = tempfile::tempdir().unwrap();
            let context = seed(root.path(), count, width);
            let parent = root.path().join("history/claude-notices");
            let pointer: Value =
                serde_json::from_slice(&fs::read(parent.join("current.json")).unwrap()).unwrap();
            let manifest: Value = serde_json::from_slice(
                &fs::read(parent.join(pointer["manifest"]["file"].as_str().unwrap())).unwrap(),
            )
            .unwrap();
            let mut live = OpenOptions::new()
                .append(true)
                .open(parent.join(manifest["active"].as_str().unwrap()))
                .unwrap();
            for index in 0..MAX_CLAIMS - 6 {
                let mut bytes = claim_line(&claim(count + 1000 + index, None));
                bytes.push(b'\n');
                live.write_all(&bytes).unwrap();
            }
            drop(live);
            let originals: Vec<_> = fs::read_dir(&parent)
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .collect();
            let sweep = Instant::now();
            for path in &originals {
                fs::symlink_metadata(path).unwrap();
            }
            let metadata_ms = sweep.elapsed().as_secs_f64() * 1000.;
            let upgrade = Instant::now();
            consume(
                &context,
                claim(count + 10, Some("upgrade")),
                Instant::now() + Duration::from_secs(3600),
                || Ok(()),
            )
            .unwrap();
            let upgrade_ms = upgrade.elapsed().as_secs_f64() * 1000.;
            let (catalog, history) = catalog(root.path());
            for (iteration, cold) in [true, false].into_iter().enumerate() {
                if cold {
                    evict(&parent);
                }
                let started = Instant::now();
                let before_callback = Cell::new(Duration::ZERO);
                consume(
                    &context,
                    claim(
                        count + 11 + iteration,
                        Some(if cold { "cold" } else { "warm" }),
                    ),
                    started + Duration::from_secs(15),
                    || {
                        before_callback.set(started.elapsed());
                        Ok(())
                    },
                )
                .unwrap();
                let admission_ms = before_callback.get().as_secs_f64() * 1000.;
                if cold {
                    evict(&parent);
                }
                let started = Instant::now();
                history_page(&catalog, &history);
                let history_ms = started.elapsed().as_secs_f64() * 1000.;
                eprintln!(
                    "notice-scale claims={count} width={width} cold={cold} migration_ms={upgrade_ms:.3} metadata_files={} metadata_ms={metadata_ms:.3} admission_ms={admission_ms:.3} history_ms={history_ms:.3}",
                    originals.len()
                );
                assert!(admission_ms < if cold { 500. } else { 100. });
                assert!(history_ms < if cold { 500. } else { 100. });
            }
            for (rotation, cold) in [true, false].into_iter().enumerate() {
                finish_and_fill(&context, count + 50_000 + rotation * 2_000);
                if cold {
                    evict(&parent);
                }
                let started = Instant::now();
                let before_callback = Cell::new(Duration::ZERO);
                consume(
                    &context,
                    claim(
                        count + 100_000 + rotation,
                        Some(if cold {
                            "rotation-cold"
                        } else {
                            "rotation-warm"
                        }),
                    ),
                    started + Duration::from_secs(15),
                    || {
                        before_callback.set(started.elapsed());
                        Ok(())
                    },
                )
                .unwrap();
                let rotation_ms = before_callback.get().as_secs_f64() * 1000.;
                eprintln!(
                    "notice-rotation claims={count} width={width} cold={cold} admission_ms={rotation_ms:.3}"
                );
                assert!(rotation_ms < 500.);
            }
        }
    }
}

fn finish_and_fill(context: &CliContext, first: usize) {
    let parent = context.state_dir.join("history/claude-notices");
    let scan = {
        let mut ledger = Ledger::open(
            &context.state_dir,
            false,
            Instant::now() + Duration::from_secs(15),
        )
        .unwrap()
        .unwrap();
        ledger
            .scan(Instant::now() + Duration::from_secs(15))
            .unwrap()
    };
    let pointer: Value =
        serde_json::from_slice(&fs::read(parent.join("current.json")).unwrap()).unwrap();
    let manifest: Value = serde_json::from_slice(
        &fs::read(parent.join(pointer["manifest"]["file"].as_str().unwrap())).unwrap(),
    )
    .unwrap();
    let mut live = OpenOptions::new()
        .append(true)
        .open(parent.join(manifest["active"].as_str().unwrap()))
        .unwrap();
    for (claim, _) in scan.claims.values() {
        if !scan.terminals.contains_key(&claim.op) {
            let bytes = pair(claim).0;
            live.write_all(bytes.split_inclusive(|b| *b == b'\n').nth(1).unwrap())
                .unwrap();
        }
    }
    for index in 0..MAX_CLAIMS - scan.claims.len() {
        live.write_all(&pair(&claim(first + index, None)).0)
            .unwrap();
    }
    live.sync_all().unwrap();
}

#[cfg(target_os = "linux")]
fn evict(parent: &Path) {
    let checkpoint: Value =
        serde_json::from_slice(&fs::read(parent.join("checkpoint.json")).unwrap()).unwrap();
    let pointer: Value =
        serde_json::from_slice(&fs::read(parent.join("current.json")).unwrap()).unwrap();
    let manifest_name = pointer["manifest"]["file"].as_str().unwrap();
    let manifest: Value =
        serde_json::from_slice(&fs::read(parent.join(manifest_name)).unwrap()).unwrap();
    let mut paths: Vec<PathBuf> = [
        checkpoint["index"].as_str().unwrap(),
        "checkpoint.json",
        "current.json",
        "layout.json",
        manifest_name,
        manifest["active"].as_str().unwrap(),
    ]
    .into_iter()
    .map(|name| parent.join(name))
    .collect();
    paths.push(
        parent
            .ancestors()
            .nth(2)
            .unwrap()
            .join("provider/project")
            .join(format!(
                "{}.jsonl",
                archived_claim(0).binding.provider_session_id
            )),
    );
    for path in paths {
        let file = File::open(path).unwrap();
        file.sync_all().unwrap();
        // SAFETY: a live fixture descriptor; the advice changes cache residency
        // only for this temporary fixture, never the host's global page cache.
        assert_eq!(
            unsafe { libc::posix_fadvise(file.as_raw_fd(), 0, 0, libc::POSIX_FADV_DONTNEED) },
            0
        );
    }
}

#[cfg(not(target_os = "linux"))]
fn evict(_: &Path) {
    panic!("cold measurement requires fixture-local Linux page-cache advice");
}
