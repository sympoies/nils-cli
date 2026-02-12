use serde_json::json;

use crate::cli::OutputMode;
use crate::errors::AppError;
use crate::output::{emit_json_results, format_item_id};
use crate::storage::Storage;
use crate::storage::repository;

pub fn run(
    storage: &Storage,
    output_mode: OutputMode,
    limit: usize,
    cursor: Option<&str>,
) -> Result<(), AppError> {
    if let Some(raw_cursor) = cursor
        && raw_cursor.trim().is_empty()
    {
        return Err(AppError::usage("--cursor must be non-empty when provided"));
    }

    let rows = storage.with_connection(|conn| repository::fetch_pending(conn, limit))?;

    if output_mode.is_json() {
        let results = rows
            .iter()
            .map(|row| {
                json!({
                    "item_id": format_item_id(row.item_id),
                    "created_at": row.created_at,
                    "source": row.source,
                    "text": row.text,
                    "state": row.state,
                })
            })
            .collect::<Vec<_>>();

        return emit_json_results("memo-cli.fetch.v1", "memo-cli fetch", results);
    }

    println!("pending items: {}", rows.len());
    for row in rows {
        println!(
            "{}\t{}\t{}\t{}",
            format_item_id(row.item_id),
            row.created_at,
            row.source,
            row.text
        );
    }

    Ok(())
}
