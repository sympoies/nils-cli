use std::path::PathBuf;

use memo_cli::storage::Storage;
use memo_cli::storage::repository::{self, QueryState};
use memo_cli::storage::search::{self, ReportPeriod};
use pretty_assertions::assert_eq;

fn test_db_path(name: &str) -> PathBuf {
    let dir = tempfile::tempdir().expect("tempdir should be created");
    dir.keep().join(format!("{name}.db"))
}

#[test]
fn search_and_report() {
    let db_path = test_db_path("search_and_report");
    let storage = Storage::new(db_path);

    storage
        .with_transaction(|tx| {
            let first = repository::add_item(tx, "3/24 in tokyo", "cli", None)?;
            let second = repository::add_item(tx, "buy 1tb ssd for mom", "cli", None)?;

            tx.execute(
                "insert into item_derivations(
                    item_id,
                    derivation_version,
                    status,
                    is_active,
                    base_derivation_id,
                    derivation_hash,
                    agent_run_id,
                    summary,
                    category,
                    priority,
                    due_at,
                    normalized_text,
                    confidence,
                    payload_json,
                    conflict_reason
                ) values (?1, 1, 'accepted', 1, null, ?2, 'agent-run-1', ?3, ?4, null, null, ?5, 0.9, ?6, null)",
                rusqlite::params![
                    first.item_id,
                    format!("hash-{}", first.item_id),
                    "tokyo travel event",
                    "travel",
                    "tokyo travel event",
                    "{\"ok\":true}"
                ],
            )
            .map_err(memo_cli::errors::AppError::db)?;

            tx.execute(
                "insert into item_derivations(
                    item_id,
                    derivation_version,
                    status,
                    is_active,
                    base_derivation_id,
                    derivation_hash,
                    agent_run_id,
                    summary,
                    category,
                    priority,
                    due_at,
                    normalized_text,
                    confidence,
                    payload_json,
                    conflict_reason
                ) values (?1, 1, 'accepted', 1, null, ?2, 'agent-run-2', ?3, ?4, null, null, ?5, 0.8, ?6, null)",
                rusqlite::params![
                    second.item_id,
                    format!("hash-{}", second.item_id),
                    "buy ssd for mom",
                    "shopping",
                    "buy ssd for mom",
                    "{\"ok\":true}"
                ],
            )
            .map_err(memo_cli::errors::AppError::db)?;

            Ok(())
        })
        .expect("seed should succeed");

    let search_rows = storage
        .with_connection(|conn| search::search_items(conn, "tokyo", QueryState::All, 20))
        .expect("search should succeed");

    assert!(!search_rows.is_empty());
    assert!(
        search_rows
            .iter()
            .any(|row| row.preview.to_lowercase().contains("tokyo"))
    );

    let report = storage
        .with_connection(|conn| search::report_summary(conn, ReportPeriod::Week))
        .expect("report should succeed");

    assert_eq!(report.period, "week");
    assert!(report.totals.captured >= 2);
}
