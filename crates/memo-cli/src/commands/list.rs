use serde_json::json;

use crate::cli::OutputMode;
use crate::errors::AppError;
use crate::output::{emit_json_results, format_item_id};
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
                })
            })
            .collect::<Vec<_>>();
        return emit_json_results("memo-cli.list.v1", "memo-cli list", results);
    }

    if rows.is_empty() {
        println!("(no items)");
        return Ok(());
    }

    println!("item_id\tcreated_at\tstate\tpreview");
    for row in rows {
        println!(
            "{}\t{}\t{}\t{}",
            format_item_id(row.item_id),
            row.created_at,
            row.state,
            row.text_preview
        );
    }

    Ok(())
}
