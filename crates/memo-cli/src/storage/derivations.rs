use std::collections::HashSet;

use rusqlite::{Transaction, params};
use serde::Serialize;

use crate::errors::AppError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IncomingStatus {
    Accepted,
    Rejected,
    Conflict,
}

impl IncomingStatus {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "accepted" => Some(Self::Accepted),
            "rejected" => Some(Self::Rejected),
            "conflict" => Some(Self::Conflict),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ApplyInputItem {
    pub item_id: i64,
    pub status: IncomingStatus,
    pub derivation_hash: String,
    pub base_derivation_id: Option<i64>,
    pub summary: Option<String>,
    pub category: Option<String>,
    pub priority: Option<String>,
    pub due_at: Option<String>,
    pub normalized_text: Option<String>,
    pub confidence: Option<f64>,
    pub payload_json: serde_json::Value,
    pub conflict_reason: Option<String>,
    pub tags: Vec<String>,
    pub agent_run_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ApplyItemError {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ApplyItemOutcome {
    pub item_id: i64,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub derivation_version: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ApplyItemError>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ApplySummary {
    pub dry_run: bool,
    pub processed: i64,
    pub accepted: i64,
    pub skipped: i64,
    pub failed: i64,
    pub items: Vec<ApplyItemOutcome>,
}

pub fn apply_items(
    tx: &Transaction<'_>,
    items: &[ApplyInputItem],
    dry_run: bool,
    default_agent_run_id: &str,
) -> Result<ApplySummary, AppError> {
    let mut accepted = 0_i64;
    let mut skipped = 0_i64;
    let mut failed = 0_i64;
    let mut outcomes = Vec::with_capacity(items.len());

    for item in items {
        if !item_exists(tx, item.item_id)? {
            failed += 1;
            outcomes.push(ApplyItemOutcome {
                item_id: item.item_id,
                status: "failed".to_string(),
                derivation_version: None,
                error: Some(ApplyItemError {
                    code: "invalid-apply-payload".to_string(),
                    message: "item_id does not exist".to_string(),
                    details: None,
                }),
            });
            continue;
        }

        let active = current_active(tx, item.item_id)?;
        if let Some(base_derivation_id) = item.base_derivation_id
            && active.map(|row| row.0) != Some(base_derivation_id)
        {
            skipped += 1;
            outcomes.push(ApplyItemOutcome {
                item_id: item.item_id,
                status: "skipped".to_string(),
                derivation_version: None,
                error: Some(ApplyItemError {
                    code: "apply-item-conflict".to_string(),
                    message: "incoming base derivation does not match active derivation"
                        .to_string(),
                    details: Some(serde_json::json!({
                        "incoming_base_derivation_id": base_derivation_id,
                        "active_derivation_id": active.map(|row| row.0),
                    })),
                }),
            });
            continue;
        }

        if let Some(existing_version) =
            derivation_version_by_hash(tx, item.item_id, &item.derivation_hash)?
        {
            skipped += 1;
            outcomes.push(ApplyItemOutcome {
                item_id: item.item_id,
                status: "skipped".to_string(),
                derivation_version: Some(existing_version),
                error: None,
            });
            continue;
        }

        if item.status == IncomingStatus::Conflict && item.conflict_reason.is_none() {
            failed += 1;
            outcomes.push(ApplyItemOutcome {
                item_id: item.item_id,
                status: "failed".to_string(),
                derivation_version: None,
                error: Some(ApplyItemError {
                    code: "invalid-apply-payload".to_string(),
                    message: "status=conflict requires conflict_reason".to_string(),
                    details: None,
                }),
            });
            continue;
        }

        let next_version = next_derivation_version(tx, item.item_id)?;
        if dry_run {
            accepted += 1;
            outcomes.push(ApplyItemOutcome {
                item_id: item.item_id,
                status: "accepted".to_string(),
                derivation_version: Some(next_version),
                error: None,
            });
            continue;
        }

        if item.status == IncomingStatus::Accepted {
            tx.execute(
                "update item_derivations
                 set is_active = 0
                 where item_id = ?1 and is_active = 1 and status = 'accepted'",
                params![item.item_id],
            )
            .map_err(AppError::db_write)?;
        }

        let payload_json = serde_json::to_string(&item.payload_json).map_err(|err| {
            AppError::invalid_apply_payload(
                format!(
                    "payload serialization failed for item {}: {err}",
                    item.item_id
                ),
                None,
            )
        })?;
        let status_str = match item.status {
            IncomingStatus::Accepted => "accepted",
            IncomingStatus::Rejected => "rejected",
            IncomingStatus::Conflict => "conflict",
        };
        let is_active = if item.status == IncomingStatus::Accepted {
            1
        } else {
            0
        };
        let agent_run_id = item
            .agent_run_id
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or(default_agent_run_id);

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
            ) values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
            params![
                item.item_id,
                next_version,
                status_str,
                is_active,
                item.base_derivation_id,
                item.derivation_hash,
                agent_run_id,
                item.summary,
                item.category,
                item.priority,
                item.due_at,
                item.normalized_text,
                item.confidence,
                payload_json,
                item.conflict_reason,
            ],
        )
        .map_err(|err| {
            if let rusqlite::Error::SqliteFailure(native, _) = &err
                && native.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE
            {
                return AppError::runtime("duplicate derivation hash for item")
                    .with_code("apply-item-conflict");
            }
            AppError::db_write(err)
        })?;
        let derivation_id = tx.last_insert_rowid();

        if item.status == IncomingStatus::Accepted {
            attach_tags(tx, derivation_id, &item.tags)?;
        }

        accepted += 1;
        outcomes.push(ApplyItemOutcome {
            item_id: item.item_id,
            status: "accepted".to_string(),
            derivation_version: Some(next_version),
            error: None,
        });
    }

    Ok(ApplySummary {
        dry_run,
        processed: items.len() as i64,
        accepted,
        skipped,
        failed,
        items: outcomes,
    })
}

fn item_exists(tx: &Transaction<'_>, item_id: i64) -> Result<bool, AppError> {
    let count: i64 = tx
        .query_row(
            "select count(*) from inbox_items where item_id = ?1",
            params![item_id],
            |row| row.get(0),
        )
        .map_err(AppError::db_query)?;
    Ok(count > 0)
}

fn current_active(tx: &Transaction<'_>, item_id: i64) -> Result<Option<(i64, i64)>, AppError> {
    tx.query_row(
        "select derivation_id, derivation_version
         from item_derivations
         where item_id = ?1 and is_active = 1 and status = 'accepted'
         order by derivation_version desc, derivation_id desc
         limit 1",
        params![item_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .map(Some)
    .or_else(|err| match err {
        rusqlite::Error::QueryReturnedNoRows => Ok(None),
        other => Err(AppError::db_query(other)),
    })
}

fn derivation_version_by_hash(
    tx: &Transaction<'_>,
    item_id: i64,
    derivation_hash: &str,
) -> Result<Option<i64>, AppError> {
    tx.query_row(
        "select derivation_version
         from item_derivations
         where item_id = ?1 and derivation_hash = ?2
         limit 1",
        params![item_id, derivation_hash],
        |row| row.get(0),
    )
    .map(Some)
    .or_else(|err| match err {
        rusqlite::Error::QueryReturnedNoRows => Ok(None),
        other => Err(AppError::db_query(other)),
    })
}

fn next_derivation_version(tx: &Transaction<'_>, item_id: i64) -> Result<i64, AppError> {
    let next_version = tx
        .query_row(
            "select coalesce(max(derivation_version), 0) + 1
             from item_derivations
             where item_id = ?1",
            params![item_id],
            |row| row.get(0),
        )
        .map_err(AppError::db_query)?;
    Ok(next_version)
}

fn attach_tags(tx: &Transaction<'_>, derivation_id: i64, tags: &[String]) -> Result<(), AppError> {
    let mut seen = HashSet::new();
    for tag in tags {
        let trimmed = tag.trim();
        if trimmed.is_empty() {
            continue;
        }
        let normalized = trimmed.to_lowercase();
        if !seen.insert(normalized.clone()) {
            continue;
        }

        tx.execute(
            "insert into tags(tag_name, tag_name_norm)
             values(?1, ?2)
             on conflict(tag_name_norm) do update set tag_name = excluded.tag_name",
            params![trimmed, normalized],
        )
        .map_err(AppError::db_write)?;

        let tag_id: i64 = tx
            .query_row(
                "select tag_id from tags where tag_name_norm = ?1",
                params![normalized],
                |row| row.get(0),
            )
            .map_err(AppError::db_query)?;

        tx.execute(
            "insert or ignore into item_tags(derivation_id, tag_id) values (?1, ?2)",
            params![derivation_id, tag_id],
        )
        .map_err(AppError::db_write)?;
    }

    Ok(())
}
