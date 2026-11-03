use super::*;
use pretty_assertions::assert_eq;
use serde_json::json;
use std::io::{self, Cursor, Read};

fn rows(values: &[serde_json::Value]) -> Vec<u8> {
    values
        .iter()
        .map(|v| format!("{v}\n"))
        .collect::<String>()
        .into_bytes()
}

fn count(bytes: &[u8], provider: &str) -> Counts {
    scan(
        &mut Cursor::new(bytes),
        bytes.len() as u64,
        provider,
        "conversation",
        || false,
    )
}

fn boundary(id: &str) -> serde_json::Value {
    json!({"type":"system","subtype":"compact_boundary","sessionId":"conversation","uuid":id})
}

#[test]
fn claude_distinct_primary_boundaries_deduplicate_uuid_and_ignore_echoes() {
    let bytes = rows(&[
        boundary("a"),
        boundary("a"),
        boundary("b"),
        json!({"type":"user","message":{"content":"/compact"}}),
        json!({"type":"system","subtype":"compaction_started"}),
        json!({"type":"assistant","message":{"content":"summary"}}),
        json!({"type":"system","subtype":"compact_boundary","sessionId":"other","uuid":"c"}),
        json!({"type":"system","subtype":"compact_boundary","sessionId":"conversation","uuid":"d","isSidechain":true}),
    ]);
    let result = count(&bytes, "claude");
    assert_eq!(result.lines, Metric::known(8));
    assert_eq!(result.compactions, Metric::known(2));
    assert_eq!(
        count(&rows(&[json!({"type":"user"})]), "claude").compactions,
        Metric::known(0)
    );
}

#[test]
fn codex_canonical_boundary_and_notices_count_once_but_ambiguous_evidence_is_unknown() {
    let compacted = json!({"type":"compacted","payload":{"message":"CANARY"}});
    let notice = json!({"type":"event_msg","payload":{"type":"context_compacted"}});
    assert_eq!(
        count(&rows(&[compacted.clone(), notice.clone()]), "codex").compactions,
        Metric::known(1)
    );
    assert_eq!(
        count(&rows(&[compacted.clone(), compacted]), "codex").compactions,
        Metric::unknown(Reason::AmbiguousCompletionEvidence)
    );
    assert_eq!(
        count(&rows(&[notice]), "codex").compactions,
        Metric::unknown(Reason::UnsupportedFormat)
    );
    assert_eq!(
        count(
            &rows(&[
                json!({"type":"compacted","id":"a"}),
                json!({"type":"compacted","id":"a"}),
                json!({"type":"compacted","id":"b"})
            ]),
            "codex"
        )
        .compactions,
        Metric::known(2)
    );
    assert_eq!(
        count(
            &rows(&[json!({"type":"event_msg","payload":{"type":"compaction_started"}})]),
            "codex"
        )
        .compactions,
        Metric::known(0)
    );
    assert_eq!(
        count(
            &rows(&[
                json!({"type":"system","subtype":"compact_boundary","sessionId":"conversation"})
            ]),
            "claude"
        )
        .compactions,
        Metric::unknown(Reason::AmbiguousCompletionEvidence)
    );
}

#[test]
fn physical_lines_survive_malformed_oversized_and_unterminated_json() {
    let bytes = "{\"type\":\"user\",\"text\":\"世界\\nCANARY\"}\r\n\ninvalid\n{}".as_bytes();
    let result = count(bytes, "claude");
    assert_eq!(result.lines, Metric::known(3));
    assert_eq!(
        result.compactions,
        Metric::unknown(Reason::IncompleteSource)
    );
    let mut oversized = vec![b' '; MAX_RECORD_BYTES + 1];
    oversized.push(b'\n');
    let result = count(&oversized, "codex");
    assert_eq!(result.lines, Metric::known(1));
    assert_eq!(
        result.compactions,
        Metric::unknown(Reason::IncompleteSource)
    );
    assert!(result.stats.max_record_buffer <= MAX_RECORD_BYTES);
    assert_eq!(count(b"{}", "codex").lines, Metric::known(0));
    assert_eq!(count(b"", "codex").compactions, Metric::known(0));
}

struct Spy {
    bytes: Cursor<Vec<u8>>,
    calls: usize,
    maximum: usize,
    read: usize,
}
impl Read for Spy {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.calls += 1;
        self.maximum = self.maximum.max(buffer.len());
        let n = self.bytes.read(buffer)?;
        self.read += n;
        Ok(n)
    }
}

#[test]
fn exact_cap_is_one_forward_scan_with_bounded_chunks_and_cap_plus_one_reads_nothing() {
    let mut spy = Spy {
        bytes: Cursor::new(vec![b'\n'; MAX_COUNT_BYTES as usize]),
        calls: 0,
        maximum: 0,
        read: 0,
    };
    let result = scan(&mut spy, MAX_COUNT_BYTES, "codex", "conversation", || false);
    assert_eq!(result.lines, Metric::known(MAX_COUNT_BYTES));
    assert_eq!(result.compactions, Metric::known(0));
    assert_eq!(spy.read, MAX_COUNT_BYTES as usize);
    assert_eq!(spy.calls, MAX_COUNT_BYTES as usize / CHUNK_BYTES);
    assert_eq!(spy.maximum, CHUNK_BYTES);
    assert_eq!(result.stats.bytes, MAX_COUNT_BYTES);
    spy.calls = 0;
    let result = scan(
        &mut spy,
        MAX_COUNT_BYTES + 1,
        "codex",
        "conversation",
        || false,
    );
    assert_eq!(result.lines, Metric::unknown(Reason::ReadBudgetExceeded));
    assert_eq!(spy.calls, 0);
}

#[test]
fn deadline_short_read_and_io_failure_never_return_partial_totals() {
    let mut reader = Cursor::new(b"{}\n".as_slice());
    let result = scan(&mut reader, 4, "codex", "conversation", || false);
    assert_eq!(result.lines, Metric::unknown(Reason::IncompleteSource));
    let result = scan(&mut reader, 3, "codex", "conversation", || true);
    assert_eq!(result.lines, Metric::unknown(Reason::ReadBudgetExceeded));
    assert_eq!(result.stats.bytes, 0);
    struct Denied;
    impl Read for Denied {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "ERROR_CANARY",
            ))
        }
    }
    assert_eq!(
        scan(&mut Denied, 1, "codex", "conversation", || false).lines,
        Metric::unknown(Reason::SourceUnavailable)
    );
}

#[test]
fn boundary_identity_set_has_a_fixed_memory_bound() {
    let bytes = rows(
        &(0..=MAX_BOUNDARIES)
            .map(|i| boundary(&i.to_string()))
            .collect::<Vec<_>>(),
    );
    let result = count(&bytes, "claude");
    assert_eq!(
        result.compactions,
        Metric::unknown(Reason::ReadBudgetExceeded)
    );
    assert!(result.stats.max_identities <= MAX_BOUNDARIES);
    assert_eq!(result.lines, Metric::known(MAX_BOUNDARIES as u64 + 1));
}

#[test]
fn deadline_after_one_chunk_discards_the_partial_full_file_count() {
    let bytes = vec![b'\n'; CHUNK_BYTES * 2];
    let mut checks = 0;
    let result = scan(
        &mut Cursor::new(bytes),
        (CHUNK_BYTES * 2) as u64,
        "codex",
        "conversation",
        || {
            checks += 1;
            checks > 1
        },
    );
    assert_eq!(result.lines, Metric::unknown(Reason::ReadBudgetExceeded));
    assert_eq!(
        result.compactions,
        Metric::unknown(Reason::ReadBudgetExceeded)
    );
    assert_eq!(result.stats.bytes, CHUNK_BYTES as u64);
}
