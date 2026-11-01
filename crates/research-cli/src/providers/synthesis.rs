use crate::{Assessment, Confidence, Fact, ResearchResult};
use serde::Deserialize;

#[derive(Deserialize)]
struct Synthesis {
    summary: String,
    facts: Vec<CitedClaim>,
    assessment: ModelAssessment,
    unknowns: Vec<String>,
    conflicts: Vec<String>,
}
#[derive(Deserialize)]
struct CitedClaim {
    claim: String,
    source_urls: Vec<String>,
}
#[derive(Deserialize)]
struct ModelAssessment {
    key_points: Vec<String>,
    suggested_directions: Vec<String>,
    preliminary_judgment: String,
    confidence: Confidence,
}
pub fn apply(result: &mut ResearchResult, text: &str) {
    let Ok(synthesis) = serde_json::from_str::<Synthesis>(text.trim()) else {
        result.warnings.push("Model synthesis is unavailable or incomplete; retained only returned source metadata and usage".into());
        result
            .unknowns
            .push("Model summary and assessment unavailable".into());
        return;
    };
    result.summary = Some(synthesis.summary);
    result.assessment = Assessment {
        label: "model_inference".into(),
        generated: true,
        key_points: synthesis.assessment.key_points,
        suggested_directions: synthesis.assessment.suggested_directions,
        preliminary_judgment: Some(synthesis.assessment.preliminary_judgment),
        confidence: synthesis.assessment.confidence,
    };
    result.unknowns.extend(synthesis.unknowns);
    result.conflicts.extend(synthesis.conflicts);
    for fact in synthesis.facts {
        let source_ids: Vec<_> = fact
            .source_urls
            .iter()
            .filter_map(|url| {
                result
                    .sources
                    .iter()
                    .find(|s| s.url == *url || s.final_url.as_ref() == Some(url))
                    .map(|s| s.id.clone())
            })
            .collect();
        if fact.claim.trim().is_empty() || source_ids.is_empty() {
            result.warnings.push(
                "Discarded an uncited fact or a fact referencing an unavailable source".into(),
            );
        } else {
            result.facts.push(Fact {
                claim: fact.claim,
                source_ids,
            });
        }
    }
}
