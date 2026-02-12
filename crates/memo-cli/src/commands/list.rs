use serde_json::json;

use crate::cli::OutputMode;
use crate::errors::AppError;
use crate::output::{emit_json_results_with_meta, format_item_id, text};
use crate::storage::Storage;
use crate::storage::repository::{self, QueryState};

pub fn run(
    storage: &Storage,
    output_mode: OutputMode,
    state: QueryState,
    limit: usize,
    offset: usize,
) -> Result<(), AppError> {
    let rows =
        storage.with_connection(|conn| repository::list_items(conn, state, limit, offset))?;

    if output_mode.is_json() {
        let results = rows
            .iter()
            .map(|row| {
                json!({
                    "item_id": format_item_id(row.item_id),
                    "created_at": row.created_at,
                    "state": row.state,
                    "text_preview": row.text_preview,
                    "content_type": row.content_type,
                    "validation_status": row.validation_status,
                })
            })
            .collect::<Vec<_>>();
        return emit_json_results_with_meta(
            "memo-cli.list.v1",
            "memo-cli list",
            results,
            Some(json!({
                "limit": limit,
                "offset": offset,
                "returned": rows.len(),
            })),
            None,
        );
    }

    text::print_list(&rows);

    Ok(())
}
