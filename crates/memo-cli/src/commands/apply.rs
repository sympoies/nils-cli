use std::fs;
use std::io::{self, Read};

use serde_json::json;

use crate::cli::{ApplyArgs, OutputMode};
use crate::errors::AppError;
use crate::output::emit_json_result;

pub fn run(output_mode: OutputMode, args: &ApplyArgs) -> Result<(), AppError> {
    if args.input.is_some() == args.stdin {
        return Err(AppError::usage(
            "apply requires exactly one input source: --input <file> or --stdin",
        ));
    }

    let payload = if let Some(path) = &args.input {
        fs::read_to_string(path).map_err(|err| {
            AppError::runtime(format!(
                "failed to read apply payload from {}: {err}",
                path.display()
            ))
            .with_code("io-read-failed")
        })?
    } else {
        let mut buffer = String::new();
        io::stdin().read_to_string(&mut buffer).map_err(|err| {
            AppError::runtime(format!("failed to read apply payload from stdin: {err}"))
        })?;
        buffer
    };

    let value: serde_json::Value = serde_json::from_str(&payload).map_err(|err| {
        AppError::data(format!("invalid apply payload JSON: {err}"))
            .with_code("invalid-apply-payload")
    })?;

    let processed = value
        .get("items")
        .and_then(|items| items.as_array())
        .map(|items| items.len() as i64)
        .or_else(|| value.as_array().map(|items| items.len() as i64))
        .unwrap_or(0);

    let result = json!({
        "dry_run": args.dry_run,
        "processed": processed,
        "accepted": 0,
        "skipped": processed,
        "failed": 0,
        "items": []
    });

    if output_mode.is_json() {
        return emit_json_result("memo-cli.apply.v1", "memo-cli apply", result);
    }

    println!(
        "apply payload processed={} accepted=0 skipped={} failed=0 dry_run={}",
        processed, processed, args.dry_run
    );
    println!("note: write-back implementation is planned for sprint 3");
    Ok(())
}
