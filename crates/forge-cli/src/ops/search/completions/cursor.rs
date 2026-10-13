use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::cli::CompletionQueryArgs;
use crate::error::ForgeError;
use crate::provider::ProviderContext;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Pending {
    pub node_id: String,
    pub labels_after: Option<String>,
    pub references_after: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Cursor {
    pub version: u8,
    pub scope: String,
    pub after: Option<String>,
    pub scanned: u32,
    pub total_count: Option<u32>,
    pub search_done: bool,
    pub pending: Vec<Pending>,
}

impl Cursor {
    pub fn new(scope: String) -> Self {
        Self {
            version: 1,
            scope,
            after: None,
            scanned: 0,
            total_count: None,
            search_done: false,
            pending: Vec::new(),
        }
    }

    pub fn read(raw: Option<&str>, scope: String) -> Result<Self, ForgeError> {
        let Some(raw) = raw else {
            return Ok(Self::new(scope));
        };
        let cursor: Self = serde_json::from_str(raw).map_err(|_| invalid())?;
        if cursor.version != 1
            || cursor.scope != scope
            || cursor.scanned > 1000
            || (cursor.search_done && (cursor.total_count.is_none() || cursor.pending.is_empty()))
            || cursor.pending.len() > 100
            || cursor.after.as_deref() == Some("")
            || cursor.pending.iter().any(|pending| {
                pending.node_id.is_empty()
                    || pending.labels_after.as_deref() == Some("")
                    || pending.references_after.as_deref() == Some("")
            })
        {
            return Err(invalid());
        }
        Ok(cursor)
    }

    pub fn encode(&self) -> String {
        // Only strings, numbers and booleans; serialization cannot fail.
        serde_json::to_string(self).expect("completion cursor is JSON serializable")
    }
}

pub(super) fn scope(
    ctx: &ProviderContext,
    effective_query: &str,
    args: &CompletionQueryArgs,
) -> String {
    let bytes = serde_json::to_vec(&(
        ctx.provider.as_str(),
        &ctx.host,
        effective_query,
        &args.since,
        &args.until,
        args.kind,
        args.exclude_forks,
        args.exclude_archived,
    ))
    .expect("completion scope is JSON serializable");
    format!("{:x}", Sha256::digest(bytes))
}

fn invalid() -> ForgeError {
    ForgeError::validation(
        super::super::schema_err(),
        "completion_cursor_invalid",
        "completion cursor is invalid or belongs to a different query, window, filter or provider authority",
        None,
    )
}
