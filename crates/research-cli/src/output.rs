use crate::ResearchResult;
use nils_common::cli_contract::{Envelope, exit};

pub fn exit_code(envelope: &Envelope<ResearchResult>) -> i32 {
    if envelope.ok {
        return exit::SUCCESS;
    }
    match envelope.error.as_ref().map(|e| e.code.as_str()) {
        Some("invalid-request" | "invalid-url" | "non-public-url") => exit::DATA,
        Some("missing-key" | "unsupported-capability" | "provider-auth") => exit::UNAVAILABLE,
        _ => exit::RUNTIME,
    }
}
pub fn render_text(envelope: &Envelope<ResearchResult>) -> String {
    let mut out = String::new();
    if let Some(data) = &envelope.data {
        if let Some(content) = &data.content {
            out.push_str(content);
            out.push('\n');
        }
        out.push_str("Sources\n");
        for source in &data.sources {
            out.push_str(&format!("[{}] {}\n", source.id, source.url));
        }
        out.push_str("Facts\n");
        for fact in &data.facts {
            out.push_str(&format!(
                "- {} [{}]\n",
                fact.claim,
                fact.source_ids.join(", ")
            ));
        }
        if let Some(summary) = &data.summary {
            out.push_str(&format!("Summary\n{summary}\n"));
        }
        out.push_str("Assessment (model inference)\n");
        if data.assessment.generated {
            for point in &data.assessment.key_points {
                out.push_str(&format!("- {point}\n"));
            }
            for direction in &data.assessment.suggested_directions {
                out.push_str(&format!("Suggested direction: {direction}\n"));
            }
            if let Some(judgment) = &data.assessment.preliminary_judgment {
                out.push_str(&format!("{judgment}\n"));
            }
            out.push_str(&format!("Confidence: {:?}\n", data.assessment.confidence));
        } else {
            out.push_str("Unavailable (no model assessment generated)\n");
        }
        for unknown in &data.unknowns {
            out.push_str(&format!("Unknown: {unknown}\n"));
        }
        for conflict in &data.conflicts {
            out.push_str(&format!("Conflict: {conflict}\n"));
        }
    }
    out
}
