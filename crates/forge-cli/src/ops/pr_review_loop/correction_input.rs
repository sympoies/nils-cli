use super::*;
use serde::Deserialize;

pub(super) struct ObservationInput {
    pub observations: Vec<review_state::ReviewFindingObservation>,
    correction: Option<review_state::correction::CorrectionRequest>,
    supersession: Option<review_state::supersession::SupersessionRequest>,
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
        if value.get("supersession").is_some()
            || value.get("schema_version").and_then(|v| v.as_str())
                == Some(review_state::supersession::INPUT_SCHEMA)
        {
            let envelope: super::supersession_input::SupersessionEnvelope =
                serde_json::from_value(value.clone()).map_err(|_| {
                    review_state::supersession::invalid("invalid supersession envelope fields")
                })?;
            if envelope.schema_version != review_state::supersession::INPUT_SCHEMA {
                return Err(review_state::supersession::invalid(
                    "unsupported supersession input schema",
                ));
            }
            envelope.supersession.validate()?;
            return Ok(Self {
                observations: observations_from_value(&envelope.findings)?,
                correction: None,
                supersession: Some(envelope.supersession),
            });
        }
        if value.get("correction").is_some()
            || value.get("schema_version").and_then(|v| v.as_str())
                == Some(review_state::correction::INPUT_SCHEMA)
        {
            // Parse optional proof rows through the same ordinary row parser,
            // retaining aliases while normalizing input key names for the receipt.
            let mut value = value.clone();
            if let Some(proof) = value
                .get_mut("correction")
                .and_then(|c| c.get_mut("attempted_findings"))
                && !proof.is_null()
            {
                *proof = serde_json::to_value(observations_from_value(proof)?).map_err(|_| {
                    review_state::correction::invalid("cannot encode attempted findings proof")
                })?;
            }
            let envelope: CorrectionEnvelope = serde_json::from_value(value).map_err(|_| {
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
                supersession: None,
            })
        } else {
            Ok(Self {
                observations: observations_from_value(value)?,
                correction: None,
                supersession: None,
            })
        }
    }

    pub fn transition<R: BackendRunner>(
        &self,
        runner: &R,
        ctx: &ProviderContext,
        repository: &str,
        chain: &review_state::ReviewStateChain,
        args: &PrReviewLoopObserveArgs,
    ) -> Result<review_state::ReviewLoopTransition, ForgeError> {
        if let Some(request) = &self.supersession {
            if args.body.is_some() || args.body_file.is_some() {
                return Err(review_state::supersession::invalid(
                    "supersession is a stop receipt and cannot carry a delivery outcome",
                ));
            }
            return super::supersession_input::transition(
                runner,
                ctx,
                repository,
                chain,
                args,
                request,
                &self.observations,
            );
        }
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
