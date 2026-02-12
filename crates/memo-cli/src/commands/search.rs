use serde_json::json;

use crate::cli::OutputMode;
use crate::errors::AppError;
use crate::output::{emit_json_results, format_item_id, text};
use crate::storage::Storage;
use crate::storage::repository::QueryState;
use crate::storage::search;

pub fn run(
    storage: &Storage,
    output_mode: OutputMode,
    state: QueryState,
    query: &str,
    limit: usize,
) -> Result<(), AppError> {
    let query = query.trim();
    if query.is_empty() {
        return Err(AppError::usage("search requires a non-empty query"));
    }

    let rows = storage.with_connection(|conn| search::search_items(conn, query, state, limit))?;

    if output_mode.is_json() {
        let results = rows
            .iter()
            .map(|row| {
                json!({
                    "item_id": format_item_id(row.item_id),
                    "created_at": row.created_at,
                    "score": row.score,
                    "matched_fields": row.matched_fields,
                    "preview": row.preview,
                })
            })
            .collect::<Vec<_>>();
        return emit_json_results("memo-cli.search.v1", "memo-cli search", results);
    }

    text::print_search(&rows);

    Ok(())
}
