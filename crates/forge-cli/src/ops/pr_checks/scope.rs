//! Additive scope metadata leaves the existing aggregate fields intact.
use super::PrChecksPayload;
use serde::{Serialize, Serializer, ser::SerializeStruct};

impl Serialize for PrChecksPayload {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let configured_required_count = self.checks.iter().filter(|c| c.required).count();
        let visible_state = ["failure", "timed_out", "cancelled", "pending"]
            .into_iter()
            .find(|state| self.checks.iter().any(|c| c.state == *state))
            .unwrap_or("success");
        let fallback = self.gate_scope == "visible_fallback";
        let gate_count = if fallback {
            self.checks.len() as u32
        } else {
            self.required_count
        };
        let gate_state = if fallback && !self.checks.is_empty() {
            visible_state
        } else {
            self.state
        };
        let mut data = serializer.serialize_struct("PrChecksPayload", 15)?;
        data.serialize_field("provider", self.provider)?;
        data.serialize_field("state", self.state)?;
        data.serialize_field("required_count", &self.required_count)?;
        data.serialize_field("success_count", &self.success_count)?;
        data.serialize_field("failed", &self.failed)?;
        data.serialize_field("pending", &self.pending)?;
        data.serialize_field("checks", &self.checks)?;
        if let Some(duration_ms) = self.duration_ms {
            data.serialize_field("duration_ms", &duration_ms)?;
        }
        if !self.warnings.is_empty() {
            data.serialize_field("warnings", &self.warnings)?;
        }
        data.serialize_field("configured_required_count", &configured_required_count)?;
        data.serialize_field("gate_scope", self.gate_scope)?;
        data.serialize_field("gate_count", &gate_count)?;
        data.serialize_field("gate_state", gate_state)?;
        data.serialize_field("visible_count", &self.checks.len())?;
        data.serialize_field("visible_state", visible_state)?;
        data.end()
    }
}
