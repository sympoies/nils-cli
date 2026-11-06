//! Additive Codex attestations in the existing bounded notice ledger.
use super::*;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Claim {
    pub(crate) identity_sha256: String,
    pub(crate) client_message_id: String,
}

impl Claim {
    pub(super) fn valid(&self) -> bool {
        hash_valid(&self.identity_sha256)
            && self.client_message_id.starts_with("agent-session-")
            && id_valid(&self.client_message_id, 128)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Row {
    pub(crate) event_offset: u64,
    pub(crate) event_payload_sha256: String,
    pub(crate) turn_id: String,
}

impl Row {
    pub(crate) fn valid(&self) -> bool {
        hash_valid(&self.event_payload_sha256) && id_valid(&self.turn_id, 256)
    }
}

impl super::Claim {
    pub(crate) fn for_codex(
        record: &crate::SessionRecord,
        text: &str,
        baseline: Stamp,
        budget: Duration,
        recovery: Option<crate::auto_resume::notice::Recovery>,
    ) -> Result<Self, CliError> {
        let unavailable = runtime::unavailable;
        let identity_sha256 =
            crate::serve::prompt_receipts::native_identity(record).ok_or_else(unavailable)?;
        let provider = record
            .provider_resume
            .as_ref()
            .filter(|provider| provider.provider == "codex")
            .ok_or_else(unavailable)?;
        let op = uuid::Uuid::new_v4().to_string();
        let client_message_id = crate::serve::prompt_receipts::client_message_id(record, &op)
            .ok_or_else(unavailable)?;
        let binding = Binding {
            launch: super::super::Launch::from_record(record)?,
            provider: "codex".into(),
            provider_session_id: provider.session_id.clone(),
            provider_version: None,
            module_sha256: None,
        };
        let mut claim = Self::new(op, binding, text, None, baseline, budget);
        claim.codex = Some(Claim {
            identity_sha256,
            client_message_id,
        });
        claim.recovery = recovery;
        Ok(claim)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scheduler_and_other_notice_budget_caps_refuse_before_consumption() {
        for scheduler in [false, true] {
            let ceiling = if scheduler { 30_000 } else { 15_000 };
            let text =
                "<!-- agent-notice:v1 kind=runtime.resume source=agent-session -->\n\nContinue";
            let mut value = serde_json::json!({"schema_version":SCHEMA,
                "op":"33333333-3333-4333-8333-333333333333",
                "binding":{"launch":{"session_id":"modeled","incarnation":"runtime-one","generation":1,"profile":null},
                    "provider":"codex","provider_session_id":"thread-one"},
                "codex":{"identity_sha256":"a".repeat(64),"client_message_id":"agent-session-fixture"},
                "history_id":crate::provider_history::stable_history_id("codex",None,"thread-one"),
                "envelope_bytes":text.len(),"envelope_sha256":sha256(text.as_bytes()),"original_budget_ms":ceiling,
                "baseline":{"device":0,"inode":0,"eof":0,"continuity_sha256":sha256(b"")}});
            if scheduler {
                value["recovery"] = serde_json::json!({"generation":"b".repeat(64),
                "cause":"provider_capacity","predecessor":"failed-turn","revision":1});
            }
            let accepted: super::super::Claim = serde_json::from_value(value.clone()).unwrap();
            assert!(accepted.valid(), "the exact ceiling remains admitted");
            value["original_budget_ms"] = serde_json::json!(ceiling + 1);
            let claim = serde_json::from_value(value).unwrap();
            let temp = tempfile::tempdir().unwrap();
            let context = CliContext {
                state_dir: temp.path().into(),
                host: None,
            };
            let mut consumed = false;
            assert!(
                consume(
                    &context,
                    claim,
                    Instant::now() + Duration::from_secs(1),
                    || {
                        consumed = true;
                        Ok(())
                    }
                )
                .is_err()
            );
            assert!(!consumed);
            assert!(!temp.path().join("history/claude-notices").exists());
        }
    }
}
