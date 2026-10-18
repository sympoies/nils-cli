//! Private, bounded operation receipts. An admitted operation is never submitted twice.
use std::io;

use nils_common::fs::{SECRET_FILE_MODE, write_atomic};
use serde_json::{Value, json};

use super::super::{
    coordination_root, digest_bytes, lock_private_store, read_private_file, store_corrupt,
    store_unavailable,
};
use super::{RESULT_SCHEMA, client, read_json};
use crate::{CliContext, CliError};

pub(super) const PREFIX: &str = "console-start-";
pub(super) const MAX_OPERATIONS: usize = 1024;
pub(super) const MAX_RECEIPT_BYTES: u64 = 16 * 1024;

pub(super) fn valid_id(value: &str) -> bool {
    super::retention::timestamp_ms(value).is_some()
}

fn invalid(code: &str, message: &str) -> CliError {
    CliError::usage(code, message, None)
}

pub(super) struct Receipt {
    name: String,
    value: Value,
}
impl Receipt {
    pub(super) fn begin(
        context: &CliContext,
        caller: &str,
        incarnation: &str,
        operation: &str,
        body: &Value,
        machine: &str,
        caller_created_at: &str,
    ) -> Result<(Self, bool), CliError> {
        let key = digest_bytes(format!("{caller}\n{incarnation}\n{operation}").as_bytes());
        let name = format!("{PREFIX}{key}");
        let _lock = lock_private_store(context, "console-start-operations.lock")?;
        let root = coordination_root(context)?;
        let path = root.join(format!("{name}.json"));
        let mut submission = body.clone();
        submission
            .as_object_mut()
            .ok_or_else(super::invalid)?
            .remove("operation_id");
        let fingerprint =
            digest_bytes(&serde_json::to_vec(&submission).map_err(|_| super::invalid())?);
        match read_private_file(&path, MAX_RECEIPT_BYTES) {
            Ok(bytes) => {
                let value: Value = serde_json::from_slice(&bytes).map_err(|_| store_corrupt())?;
                if value["schema_version"] != "agent-session.console-start-operation.v1" {
                    return Err(store_corrupt());
                }
                if body["reconcile"] != true && value["request_hash"] != fingerprint {
                    return Err(invalid(
                        "console-start-operation-conflict",
                        "operation id is already bound to another start request",
                    ));
                }
                Ok((Self { name, value }, false))
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                if body["reconcile"] == true {
                    return Err(invalid(
                        "console-start-operation-not-found",
                        "no start receipt exists for this caller incarnation and operation",
                    ));
                }
                let started_ms =
                    super::retention::timestamp_ms(operation).ok_or_else(super::invalid)?;
                super::retention::admit(&root, started_ms)?;
                let receipt = Self {
                    name,
                    value: json!({
                        "schema_version":"agent-session.console-start-operation.v1",
                        "request_hash":fingerprint,"machine":machine,"state":"ambiguous",
                        "no_parent":body["no_parent"] == true,"safe_to_retry":false,
                        "caller_created_at":caller_created_at,"operation_started_at_ms":started_ms
                    }),
                };
                receipt.save(context)?;
                Ok((receipt, true))
            }
            Err(_) => Err(store_unavailable()),
        }
    }

    fn save(&self, context: &CliContext) -> Result<(), CliError> {
        let path = coordination_root(context)?.join(format!("{}.json", self.name));
        let bytes = serde_json::to_vec(&self.value).map_err(|_| store_unavailable())?;
        write_atomic(&path, &bytes, SECRET_FILE_MODE).map_err(|_| store_unavailable())
    }

    pub(super) fn finish(
        &mut self,
        context: &CliContext,
        result: &Result<Value, CliError>,
    ) -> Result<(), CliError> {
        let _lock = lock_private_store(context, "console-start-operations.lock")?;
        let path = coordination_root(context)?.join(format!("{}.json", self.name));
        // Pruning publishes its rejection fence before removing the reservation.
        // Serialize with that removal so a late response cannot consume a reclaimed slot.
        if !path.try_exists().map_err(|_| store_unavailable())? {
            return Ok(());
        }
        let created = match result {
            Ok(value) => Some(value["session"].clone()),
            Err(error) => error
                .details()
                .and_then(|details| details.get("created_session_id"))
                .map(|id| json!({"id":id})),
        };
        if let Some(created) = created.filter(|session| identifier(&session["id"]).is_some()) {
            let mut session = json!({"id":created["id"]});
            // Persist metadata only; never prompts, launch env values, or arbitrary upstream details.
            for key in ["agent", "session_incarnation", "title_mode"] {
                if let Some(text) = identifier(&created[key]) {
                    session[key] = json!(text);
                }
            }
            if let Some(revision) = created["display_revision"].as_u64() {
                session["display_revision"] = json!(revision);
            }
            self.value["session"] = session;
            self.value["state"] = json!("created");
        } else if let Err(error) = result
            && (error
                .details()
                .is_some_and(|d| d["recovery"]["safe_to_retry"] == true)
                || matches!(error.0.exit_code, 64 | 65))
        {
            self.value["state"] = json!("not_created");
            self.value["safe_to_retry"] = json!(true);
        }
        self.save(context)
    }

    pub(super) fn reconcile(
        &self,
        config: &super::Config,
        caller: &str,
        incarnation: &str,
    ) -> Result<Value, CliError> {
        let mut result = json!({"schema_version":RESULT_SCHEMA,"machine":self.value["machine"],
            "reconciliation":{"state":self.value["state"],"safe_to_retry":self.value["safe_to_retry"]}});
        if self.value["state"] == "created" {
            result["session"] = self.value["session"].clone();
            return Ok(result);
        }
        if self.value["state"] != "ambiguous" {
            return Ok(result);
        }
        // One bounded read, never another POST create. Candidate identity is not adoption proof.
        let mut url = reqwest::Url::parse(&format!("{}/api/coordination/board/v1", config.url))
            .map_err(|_| store_unavailable())?;
        url.query_pairs_mut()
            .append_pair("source_session_id", caller)
            .append_pair("source_incarnation", incarnation)
            .append_pair(
                "machine",
                self.value["machine"].as_str().unwrap_or_default(),
            );
        let response = client()?
            .get(url)
            .timeout(std::time::Duration::from_secs(5))
            .bearer_auth(&config.token)
            .send();
        let board = response
            .ok()
            .filter(|response| response.status().is_success())
            .and_then(read_json);
        let mut candidates = Vec::new();
        if let Some(board) = &board
            && board["schema_version"] == "agent-session.board-view.v1"
            && let Some(records) = board["records"].as_array()
        {
            for record in records.iter().take(1024) {
                let parent = &record["lineage"]["parent"];
                let matches_parent = parent["session_id"] == caller
                    && parent["machine"] == config.machine
                    && parent["session_created_at"] == self.value["caller_created_at"];
                if record["machine"] != self.value["machine"]
                    || !(matches_parent || self.value["no_parent"] == true && parent.is_null())
                {
                    continue;
                }
                if let Some(id) = identifier(&record["session_id"]) {
                    candidates.push(json!({"session_id":id,"machine":record["machine"]}));
                }
                if candidates.len() == 64 {
                    break;
                }
            }
        }
        result["reconciliation"]["candidates"] = json!(candidates);
        result["reconciliation"]["candidate_scan"] =
            json!(
                if board.as_ref().is_some_and(|board| board["schema_version"]
                    == "agent-session.board-view.v1"
                    && board["records"].is_array())
                {
                    "observed"
                } else {
                    "unavailable"
                }
            );
        result["reconciliation"]["next_action"] = json!("inspect_console_before_retry");
        Ok(result)
    }
}

fn identifier(value: &Value) -> Option<&str> {
    value.as_str().filter(|text| {
        !text.is_empty()
            && text.len() <= 128
            && text
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn late_completion_cannot_resurrect_a_pruned_receipt() {
        let root = tempfile::tempdir().unwrap();
        let context = CliContext {
            state_dir: root.path().to_path_buf(),
            host: None,
        };
        let operation = uuid::Uuid::now_v7().to_string();
        let (mut receipt, submitted) = Receipt::begin(
            &context,
            "caller",
            "launch",
            &operation,
            &json!({"session":{"agent":"claude"}}),
            "target",
            "2030-01-01T00:00:00Z",
        )
        .unwrap();
        assert!(submitted);
        let directory = coordination_root(&context).unwrap();
        let path = directory.join(format!("{}.json", receipt.name));
        // Model pruning after the operation's admission window expires while its response is pending.
        let cutoff = super::super::retention::timestamp_ms(&operation).unwrap();
        write_atomic(
            &directory.join("console-start-expiry-watermark"),
            &serde_json::to_vec(&cutoff).unwrap(),
            SECRET_FILE_MODE,
        )
        .unwrap();
        std::fs::remove_file(&path).unwrap();
        receipt
            .finish(&context, &Ok(json!({"session":{"id":"created-child"}})))
            .unwrap();
        assert!(
            !path.exists(),
            "a late result must not recreate an evicted receipt and exceed capacity"
        );
    }
}
