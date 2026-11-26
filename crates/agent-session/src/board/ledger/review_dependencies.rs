use std::collections::HashSet;
use std::io;
use std::path::Path;

use serde::Deserialize;
use serde_json::Value;

use super::unavailable;
use crate::{CliError, coordination};

const FILE: &str = "review-handoff-coordinators.json";
const SCHEMA: &str = "agent-session.board-review-handoff-coordinators.v1";
const MAX_BYTES: u64 = 1024 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Snapshot {
    schema_version: String,
    coordinator_digests: HashSet<String>,
}

#[derive(Default)]
pub(super) struct ReviewDependencies(HashSet<String>);

impl ReviewDependencies {
    pub(super) fn load(board_root: &Path) -> Result<Self, CliError> {
        let bytes = match coordination::read_private_file(&board_root.join(FILE), MAX_BYTES) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(_) => return Err(unavailable()),
        };
        let snapshot: Snapshot = serde_json::from_slice(&bytes).map_err(|_| unavailable())?;
        if snapshot.schema_version != SCHEMA
            || !snapshot.coordinator_digests.iter().all(|digest| {
                digest.strip_prefix("sha256:").is_some_and(|hex| {
                    hex.len() == 64 && hex.bytes().all(|byte| b"0123456789abcdef".contains(&byte))
                })
            })
        {
            return Err(unavailable());
        }
        Ok(Self(snapshot.coordinator_digests))
    }

    pub(super) fn references(&self, record: &Value) -> bool {
        if self.0.is_empty()
            || record["state"] != "closed"
            || !record["runtime_status"].is_null()
            || !matches!(
                record["close_reason"].as_str(),
                Some("deleted" | "archived")
            )
        {
            return false;
        }
        record["session_id"].as_str().is_some_and(|id| {
            let digest = coordination::digest_bytes(id.as_bytes());
            self.0.contains(&format!("sha256:{digest}"))
        })
    }
}
