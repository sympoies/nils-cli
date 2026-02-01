use anyhow::{Context, Result};
use chrono::{TimeZone, Utc};
use serde_json::{Map, Value};
use std::path::Path;

use crate::fs;
use crate::json;
use crate::rate_limits::render;

pub fn write_weekly(target_file: &Path, usage_json: &Value) -> Result<()> {
    if !target_file.is_file() {
        anyhow::bail!("target file not found");
    }

    let usage = match render::parse_usage(usage_json) {
        Some(value) => value,
        None => return Ok(()),
    };
    let values = render::render_values(&usage);

    let primary_label = values.primary_label.clone();
    let secondary_label = values.secondary_label.clone();

    let (weekly_reset_epoch, non_weekly_reset_epoch) = if primary_label == "Weekly" {
        (values.primary_reset_epoch, Some(values.secondary_reset_epoch))
    } else if secondary_label == "Weekly" {
        (values.secondary_reset_epoch, Some(values.primary_reset_epoch))
    } else {
        (values.secondary_reset_epoch, Some(values.primary_reset_epoch))
    };

    if weekly_reset_epoch <= 0 {
        return Ok(());
    }

    let weekly_reset_iso = epoch_to_iso(weekly_reset_epoch)?;
    let non_weekly_reset_epoch = non_weekly_reset_epoch.filter(|epoch| *epoch > 0);
    let non_weekly_reset_iso = non_weekly_reset_epoch
        .and_then(|epoch| epoch_to_iso(epoch).ok());

    let fetched_at_iso = Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();

    let mut root = json::read_json(target_file).unwrap_or_else(|_| Value::Object(Map::new()));
    let root_obj = root.as_object_mut().ok_or_else(|| anyhow::anyhow!("root not object"))?;

    let mut codex_rate_limits = root_obj
        .get("codex_rate_limits")
        .and_then(|value| value.as_object())
        .cloned()
        .unwrap_or_else(Map::new);

    codex_rate_limits.insert(
        "weekly_reset_at".to_string(),
        Value::String(weekly_reset_iso.clone()),
    );
    codex_rate_limits.insert(
        "weekly_reset_at_epoch".to_string(),
        Value::Number(weekly_reset_epoch.into()),
    );
    codex_rate_limits.insert(
        "weekly_fetched_at".to_string(),
        Value::String(fetched_at_iso),
    );

    if let Some(epoch) = non_weekly_reset_epoch {
        if let Some(iso) = non_weekly_reset_iso {
            codex_rate_limits.insert(
                "non_weekly_reset_at".to_string(),
                Value::String(iso),
            );
            codex_rate_limits.insert(
                "non_weekly_reset_at_epoch".to_string(),
                Value::Number(epoch.into()),
            );
        }
    }

    root_obj.insert("codex_rate_limits".to_string(), Value::Object(codex_rate_limits));

    let out = serde_json::to_vec(&root).context("serialize writeback")?;
    fs::write_atomic(target_file, &out, fs::SECRET_FILE_MODE)?;

    Ok(())
}

fn epoch_to_iso(epoch: i64) -> Result<String> {
    if epoch <= 0 {
        anyhow::bail!("invalid epoch");
    }
    Ok(Utc.timestamp_opt(epoch, 0).single().context("epoch")?.format("%Y-%m-%dT%H:%M:%SZ").to_string())
}
