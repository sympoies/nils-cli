use serde_json::json;

use crate::cli::{AddArgs, OutputMode};
use crate::errors::AppError;
use crate::output::{emit_json_result, format_item_id};
use crate::storage::Storage;
use crate::storage::repository;

pub fn run(storage: &Storage, args: &AddArgs, output_mode: OutputMode) -> Result<(), AppError> {
    let text = args.text.trim();
    if text.is_empty() {
        return Err(AppError::usage("add requires a non-empty text argument"));
    }

    let source = args.source.trim();
    if source.is_empty() {
        return Err(AppError::usage("--source must be non-empty"));
    }

    let added = storage.with_transaction(|tx| repository::add_item(tx, text, source))?;

    if output_mode.is_json() {
        return emit_json_result(
            "memo-cli.add.v1",
            "memo-cli add",
            json!({
                "item_id": format_item_id(added.item_id),
                "created_at": added.created_at,
                "source": added.source,
                "text": added.text,
            }),
        );
    }

    println!(
        "added {} at {}",
        format_item_id(added.item_id),
        added.created_at
    );
    Ok(())
}
