//! Additive scope metadata leaves the existing aggregate fields intact.
use super::{CheckItem, PrChecksPayload};
use serde::{Serialize, Serializer, ser::SerializeStruct};

/// Gate row that satisfied the rule without running (`skipped` / `neutral`).
#[derive(Serialize)]
struct SkippedRequired<'a> {
    name: &'a str,
    state: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    conclusion: Option<&'a str>,
}

struct ScopeFields {
    configured_required_count: usize,
    gate_count: u32,
    gate_state: &'static str,
    visible_state: &'static str,
}

impl PrChecksPayload {
    /// Rows of the effective gate (the `gate_count` set) that are `skipped` or
    /// `neutral`. They keep GitHub's pass semantics but are not real passes.
    fn skipped_required(&self) -> Vec<SkippedRequired<'_>> {
        let in_gate = |c: &&CheckItem| self.gate_scope != "required" || c.required;
        self.checks
            .iter()
            .filter(in_gate)
            .filter(|c| matches!(c.state, "skipped" | "neutral"))
            .map(|c| SkippedRequired {
                name: &c.name,
                state: c.state,
                url: c.url.as_deref(),
                conclusion: c.conclusion.as_deref(),
            })
            .collect()
    }

    fn scope_fields(&self) -> ScopeFields {
        let configured_required_count = self.checks.iter().filter(|c| c.required).count();
        let visible_state = ["failure", "timed_out", "cancelled", "pending"]
            .into_iter()
            .find(|state| self.checks.iter().any(|c| c.state == *state))
            .unwrap_or("success");
        let fallback = self.gate_scope == "visible_fallback";
        ScopeFields {
            configured_required_count,
            gate_count: if fallback {
                self.checks.len() as u32
            } else {
                self.required_count
            },
            gate_state: if fallback && !self.checks.is_empty() && self.state == "success" {
                visible_state
            } else {
                self.state
            },
            visible_state,
        }
    }

    pub(crate) fn render_scope_text(&self) {
        let scope = self.scope_fields();
        println!(
            "gate_scope={} configured_required_count={} gate_count={} gate_state={} visible_count={} visible_state={}",
            self.gate_scope,
            scope.configured_required_count,
            scope.gate_count,
            scope.gate_state,
            self.checks.len(),
            scope.visible_state,
        );
        let skipped = self.skipped_required();
        if skipped.is_empty() {
            return;
        }
        let count = |state| skipped.iter().filter(|c| c.state == state).count();
        println!(
            "required_skipped={} required_neutral={} (concluded without running; gate_state={} is not proof the work ran)",
            count("skipped"),
            count("neutral"),
            scope.gate_state,
        );
        for c in &skipped {
            println!("  skipped-required: {} ({})", c.name, c.state);
        }
    }
}

impl Serialize for PrChecksPayload {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let scope = self.scope_fields();
        let skipped = self.skipped_required();
        let count = |state| skipped.iter().filter(|c| c.state == state).count();
        let mut data = serializer.serialize_struct("PrChecksPayload", 18)?;
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
        data.serialize_field(
            "configured_required_count",
            &scope.configured_required_count,
        )?;
        data.serialize_field("gate_scope", self.gate_scope)?;
        data.serialize_field("gate_count", &scope.gate_count)?;
        data.serialize_field("gate_state", scope.gate_state)?;
        data.serialize_field("visible_count", &self.checks.len())?;
        data.serialize_field("visible_state", scope.visible_state)?;
        data.serialize_field("required_skipped", &count("skipped"))?;
        data.serialize_field("required_neutral", &count("neutral"))?;
        data.serialize_field("skipped_required", &skipped)?;
        data.end()
    }
}
