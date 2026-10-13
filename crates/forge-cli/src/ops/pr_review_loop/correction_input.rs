use super::*;
use serde::Deserialize;

pub(super) struct ObservationInput {
    pub observations: Vec<review_state::ReviewFindingObservation>,
    correction: Option<review_state::correction::CorrectionRequest>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CorrectionEnvelope {
    schema_version: String,
    correction: review_state::correction::CorrectionRequest,
    findings: serde_json::Value,
}

impl ObservationInput {
    pub fn read(path: &str) -> Result<Self, ForgeError> {
        Self::parse(&read_findings_value(path)?)
    }

    pub fn parse(value: &serde_json::Value) -> Result<Self, ForgeError> {
        if value.get("correction").is_some()
            || value.get("schema_version").and_then(|v| v.as_str())
                == Some(review_state::correction::INPUT_SCHEMA)
        {
            let envelope: CorrectionEnvelope =
                serde_json::from_value(value.clone()).map_err(|_| {
                    review_state::correction::invalid("invalid correction envelope fields")
                })?;
            if envelope.schema_version != review_state::correction::INPUT_SCHEMA {
                return Err(review_state::correction::invalid(
                    "unsupported correction input schema",
                ));
            }
            envelope.correction.validate()?;
            Ok(Self {
                observations: observations_from_value(&envelope.findings)?,
                correction: Some(envelope.correction),
            })
        } else {
            Ok(Self {
                observations: observations_from_value(value)?,
                correction: None,
            })
        }
    }

    pub fn transition(
        &self,
        chain: &review_state::ReviewStateChain,
        args: &PrReviewLoopObserveArgs,
    ) -> Result<review_state::ReviewLoopTransition, ForgeError> {
        match &self.correction {
            Some(request) => review_state::correction::apply(
                chain,
                &args.expected_head,
                args.expected_state.as_deref(),
                request,
                &self.observations,
            ),
            None => review_state::observe_review_loop(
                review_state::latest_review_loop_state(chain),
                &args.expected_head,
                &self.observations,
            ),
        }
    }
}
