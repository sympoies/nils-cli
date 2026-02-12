use std::fs;
use std::io::{self, Read};

use serde::Serialize;

use crate::cli::{ApplyArgs, OutputMode};
use crate::errors::AppError;
use crate::output::{emit_json_result, format_item_id, parse_item_id, text};
use crate::storage::Storage;
use crate::storage::derivations::{self, ApplyInputItem, IncomingStatus};

const DEFAULT_AGENT_RUN_ID: &str = "memo-cli";

#[derive(Debug, Serialize)]
struct JsonApplyResult<'a> {
    dry_run: bool,
    processed: i64,
    accepted: i64,
    skipped: i64,
    failed: i64,
    items: Vec<JsonApplyItem<'a>>,
}

#[derive(Debug, Serialize)]
struct JsonApplyItem<'a> {
    item_id: String,
    status: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    derivation_version: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<&'a derivations::ApplyItemError>,
}

pub fn run(storage: &Storage, output_mode: OutputMode, args: &ApplyArgs) -> Result<(), AppError> {
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
                .with_code("io-read-failed")
        })?;
        buffer
    };

    let value: serde_json::Value = serde_json::from_str(&payload).map_err(|err| {
        AppError::invalid_apply_payload(format!("invalid apply payload JSON: {err}"), None)
    })?;
    let parsed = parse_apply_items(value)?;
    if parsed.items.is_empty() {
        return Err(AppError::invalid_apply_payload(
            "payload must include at least one item",
            Some("payload.items"),
        ));
    }

    let default_agent_run_id = parsed
        .agent_run_id
        .unwrap_or_else(|| DEFAULT_AGENT_RUN_ID.to_string());
    let summary = storage.with_transaction(|tx| {
        derivations::apply_items(tx, &parsed.items, args.dry_run, &default_agent_run_id)
    })?;

    if output_mode.is_json() {
        let result = JsonApplyResult {
            dry_run: summary.dry_run,
            processed: summary.processed,
            accepted: summary.accepted,
            skipped: summary.skipped,
            failed: summary.failed,
            items: summary
                .items
                .iter()
                .map(|item| JsonApplyItem {
                    item_id: format_item_id(item.item_id),
                    status: &item.status,
                    derivation_version: item.derivation_version,
                    error: item.error.as_ref(),
                })
                .collect(),
        };
        return emit_json_result("memo-cli.apply.v1", "memo-cli apply", result);
    }

    text::print_apply(&summary);
    Ok(())
}

#[derive(Debug)]
struct ParsedApplyPayload {
    agent_run_id: Option<String>,
    items: Vec<ApplyInputItem>,
}

fn parse_apply_items(value: serde_json::Value) -> Result<ParsedApplyPayload, AppError> {
    match value {
        serde_json::Value::Array(items) => parse_apply_item_array(items, None),
        serde_json::Value::Object(mut object) => {
            let agent_run_id =
                optional_trimmed_string(object.remove("agent_run_id"), "payload.agent_run_id")?;
            let items = object.remove("items").ok_or_else(|| {
                AppError::invalid_apply_payload("payload.items is required", Some("payload.items"))
            })?;
            match items {
                serde_json::Value::Array(array) => parse_apply_item_array(array, agent_run_id),
                _ => Err(AppError::invalid_apply_payload(
                    "payload.items must be an array",
                    Some("payload.items"),
                )),
            }
        }
        _ => Err(AppError::invalid_apply_payload(
            "payload must be an object with items[] or a top-level array",
            Some("payload"),
        )),
    }
}

fn parse_apply_item_array(
    items: Vec<serde_json::Value>,
    default_agent_run_id: Option<String>,
) -> Result<ParsedApplyPayload, AppError> {
    let mut parsed = Vec::with_capacity(items.len());
    for (index, item_value) in items.into_iter().enumerate() {
        let path = format!("payload.items[{index}]");
        let object = item_value.as_object().ok_or_else(|| {
            AppError::invalid_apply_payload("item must be an object", Some(&path))
        })?;

        let item_id = parse_item_id_value(object.get("item_id"), &format!("{path}.item_id"))?;
        let status = parse_status(object.get("status"), &format!("{path}.status"))?;
        let derivation_hash = optional_trimmed_string(
            object.get("derivation_hash").cloned(),
            &format!("{path}.derivation_hash"),
        )?
        .unwrap_or_else(|| derive_hash(&item_value));
        if derivation_hash.is_empty() {
            return Err(AppError::invalid_apply_payload(
                "derivation_hash must be non-empty when provided",
                Some(&format!("{path}.derivation_hash")),
            ));
        }

        let base_derivation_id = optional_i64(
            object.get("base_derivation_id"),
            &format!("{path}.base_derivation_id"),
        )?;
        let summary =
            optional_trimmed_string(object.get("summary").cloned(), &format!("{path}.summary"))?;
        let category =
            optional_trimmed_string(object.get("category").cloned(), &format!("{path}.category"))?;
        let priority = optional_priority(object.get("priority"), &format!("{path}.priority"))?;
        let due_at =
            optional_trimmed_string(object.get("due_at").cloned(), &format!("{path}.due_at"))?;
        let normalized_text = optional_trimmed_string(
            object.get("normalized_text").cloned(),
            &format!("{path}.normalized_text"),
        )?;
        let confidence = optional_f64(object.get("confidence"), &format!("{path}.confidence"))?;
        let payload_json = object
            .get("payload")
            .cloned()
            .unwrap_or_else(|| item_value.clone());
        let conflict_reason = optional_trimmed_string(
            object.get("conflict_reason").cloned(),
            &format!("{path}.conflict_reason"),
        )?;
        let tags = parse_tags(object.get("tags"), &format!("{path}.tags"))?;
        let agent_run_id = optional_trimmed_string(
            object.get("agent_run_id").cloned(),
            &format!("{path}.agent_run_id"),
        )?;

        parsed.push(ApplyInputItem {
            item_id,
            status,
            derivation_hash,
            base_derivation_id,
            summary,
            category,
            priority,
            due_at,
            normalized_text,
            confidence,
            payload_json,
            conflict_reason,
            tags,
            agent_run_id,
        });
    }

    Ok(ParsedApplyPayload {
        agent_run_id: default_agent_run_id,
        items: parsed,
    })
}

fn parse_item_id_value(value: Option<&serde_json::Value>, path: &str) -> Result<i64, AppError> {
    let value =
        value.ok_or_else(|| AppError::invalid_apply_payload("item_id is required", Some(path)))?;

    match value {
        serde_json::Value::Number(number) => {
            number.as_i64().filter(|id| *id > 0).ok_or_else(|| {
                AppError::invalid_apply_payload("item_id must be a positive integer", Some(path))
            })
        }
        serde_json::Value::String(raw) => parse_item_id(raw).ok_or_else(|| {
            AppError::invalid_apply_payload(
                "item_id must be a positive integer or itm_* identifier",
                Some(path),
            )
        }),
        _ => Err(AppError::invalid_apply_payload(
            "item_id must be a positive integer or itm_* identifier",
            Some(path),
        )),
    }
}

fn parse_status(value: Option<&serde_json::Value>, path: &str) -> Result<IncomingStatus, AppError> {
    let Some(value) = value else {
        return Ok(IncomingStatus::Accepted);
    };

    let raw = value
        .as_str()
        .ok_or_else(|| AppError::invalid_apply_payload("status must be a string", Some(path)))?;
    let Some(status) = IncomingStatus::parse(raw) else {
        return Err(AppError::invalid_apply_payload(
            "status must be accepted in v1",
            Some(path),
        ));
    };
    if status != IncomingStatus::Accepted {
        return Err(AppError::invalid_apply_payload(
            "status must be accepted in v1",
            Some(path),
        ));
    }
    Ok(status)
}

fn optional_trimmed_string(
    value: Option<serde_json::Value>,
    path: &str,
) -> Result<Option<String>, AppError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let raw = value
        .as_str()
        .ok_or_else(|| AppError::invalid_apply_payload("value must be a string", Some(path)))?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    Ok(Some(trimmed.to_string()))
}

fn optional_priority(
    value: Option<&serde_json::Value>,
    path: &str,
) -> Result<Option<String>, AppError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let raw = value
        .as_str()
        .ok_or_else(|| AppError::invalid_apply_payload("priority must be a string", Some(path)))?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }

    match trimmed {
        "low" | "medium" | "high" | "urgent" => Ok(Some(trimmed.to_string())),
        _ => Err(AppError::invalid_apply_payload(
            "priority must be low|medium|high|urgent",
            Some(path),
        )),
    }
}

fn optional_i64(value: Option<&serde_json::Value>, path: &str) -> Result<Option<i64>, AppError> {
    let Some(value) = value else {
        return Ok(None);
    };
    match value {
        serde_json::Value::Number(number) => number
            .as_i64()
            .filter(|value| *value > 0)
            .map(Some)
            .ok_or_else(|| {
                AppError::invalid_apply_payload("value must be a positive integer", Some(path))
            }),
        serde_json::Value::Null => Ok(None),
        _ => Err(AppError::invalid_apply_payload(
            "value must be a positive integer",
            Some(path),
        )),
    }
}

fn optional_f64(value: Option<&serde_json::Value>, path: &str) -> Result<Option<f64>, AppError> {
    let Some(value) = value else {
        return Ok(None);
    };
    match value {
        serde_json::Value::Number(number) => {
            let confidence = number.as_f64().ok_or_else(|| {
                AppError::invalid_apply_payload("confidence must be a number", Some(path))
            })?;
            if !(0.0..=1.0).contains(&confidence) {
                return Err(AppError::invalid_apply_payload(
                    "confidence must be between 0.0 and 1.0",
                    Some(path),
                ));
            }
            Ok(Some(confidence))
        }
        serde_json::Value::Null => Ok(None),
        _ => Err(AppError::invalid_apply_payload(
            "confidence must be a number",
            Some(path),
        )),
    }
}

fn parse_tags(value: Option<&serde_json::Value>, path: &str) -> Result<Vec<String>, AppError> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };

    match value {
        serde_json::Value::Array(values) => {
            let mut tags = Vec::new();
            for (index, tag) in values.iter().enumerate() {
                let raw = tag.as_str().ok_or_else(|| {
                    AppError::invalid_apply_payload(
                        "tags must be an array of strings",
                        Some(&format!("{path}[{index}]")),
                    )
                })?;
                let trimmed = raw.trim();
                if !trimmed.is_empty() {
                    tags.push(trimmed.to_string());
                }
            }
            Ok(tags)
        }
        serde_json::Value::Null => Ok(Vec::new()),
        _ => Err(AppError::invalid_apply_payload(
            "tags must be an array of strings",
            Some(path),
        )),
    }
}

fn derive_hash(value: &serde_json::Value) -> String {
    let canonical = serde_json::to_string(value).unwrap_or_default();
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in canonical.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("h{hash:016x}")
}
