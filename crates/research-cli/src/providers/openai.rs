//! OpenAI Responses API, gpt-6-luna, hosted web_search (official current contract).
use super::{INSTRUCTIONS, add_source, finish};
use crate::{Request, ResearchError, ResearchResult};
use serde_json::{Value, json};

pub fn request(request: &Request, prompt: &str) -> Value {
    let mut tool = json!({"type":"web_search","search_context_size":"low"});
    if !request.domains.is_empty() {
        tool["filters"] = json!({"allowed_domains":request.domains});
    }
    json!({"model":request.provider.model(),"instructions":INSTRUCTIONS,"input":prompt,
        "reasoning":{"effort":"low"},"tools":[tool],"tool_choice":"required",
        "max_tool_calls":request.limits.max_tool_uses,"max_output_tokens":request.limits.max_output_tokens,
        "include":["web_search_call.action.sources"],"store":false})
}

pub fn parse(
    request: &Request,
    wire: &Value,
    result: &mut ResearchResult,
) -> Result<(), ResearchError> {
    if let Some(model) = wire.get("model").and_then(Value::as_str) {
        result.model = Some(model.into());
    }
    result.usage.raw = wire.get("usage").filter(|v| !v.is_null()).cloned();
    result.usage.input_tokens = wire.pointer("/usage/input_tokens").and_then(Value::as_u64);
    result.usage.output_tokens = wire.pointer("/usage/output_tokens").and_then(Value::as_u64);
    let output = wire
        .get("output")
        .and_then(Value::as_array)
        .ok_or_else(|| ResearchError::new("provider-response", "Responses output is missing"))?;
    // Responses does not promise a raw search-call counter. Do not manufacture it
    // from output item counts: an agentic call may encapsulate multiple actions.
    let mut text = String::new();
    for item in output {
        match item.get("type").and_then(Value::as_str) {
            Some("web_search_call") => {
                // Discard any assistant planning preamble before this tool result.
                text.clear();
                if let Some(sources) = item.pointer("/action/sources").and_then(Value::as_array) {
                    for source in sources {
                        add_source(result, source, None);
                    }
                }
            }
            Some("message") => {
                if let Some(content) = item.get("content").and_then(Value::as_array) {
                    for block in content {
                        if block.get("type").and_then(Value::as_str) != Some("output_text") {
                            continue;
                        }
                        if let Some(part) = block.get("text").and_then(Value::as_str) {
                            text.push_str(part);
                        }
                        if let Some(annotations) =
                            block.get("annotations").and_then(Value::as_array)
                        {
                            for annotation in annotations {
                                if annotation.get("type").and_then(Value::as_str)
                                    == Some("url_citation")
                                {
                                    add_source(result, annotation, None);
                                }
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
    let status = wire.get("status").and_then(Value::as_str);
    if status != Some("completed") {
        result.truncated |= status == Some("incomplete");
        result
            .warnings
            .push("Responses generation did not complete; retained partial response".into());
    }
    finish(request, result, &text);
    if matches!(status, Some("failed" | "cancelled")) {
        Err(ResearchError::new(
            "provider-error",
            "Responses generation failed or was cancelled",
        ))
    } else {
        Ok(())
    }
}
