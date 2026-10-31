//! Anthropic Messages 2023-06-01 with basic search/fetch server tools.
//! web_search_20250305 and web_fetch_20250910 are supported external versions;
//! they avoid dynamic filtering/code execution that this engine does not request.
use super::{INSTRUCTIONS, add_source, finish};
use crate::{Operation, Request, ResearchError, ResearchResult};
use serde_json::{Value, json};

pub fn request(request: &Request, prompt: &str) -> Value {
    let mut search = json!({"type":"web_search_20250305","name":"web_search","max_uses":request.limits.max_tool_uses});
    if !request.domains.is_empty() {
        search["allowed_domains"] = json!(request.domains);
    }
    let mut tools = vec![search];
    if request.operation == Operation::Brief && !request.urls.is_empty() {
        let domains: Vec<_> = request
            .urls
            .iter()
            .filter_map(|s| reqwest::Url::parse(s).ok())
            .filter_map(|u| u.host_str().map(str::to_owned))
            .collect();
        tools.push(json!({"type":"web_fetch_20250910","name":"web_fetch",
            "max_uses":request.limits.max_tool_uses,"allowed_domains":domains,
            "max_content_tokens":request.limits.max_output_tokens,"citations":{"enabled":true}}));
    }
    json!({"model":request.provider.model(),"max_tokens":request.limits.max_output_tokens,
        "system":INSTRUCTIONS,"messages":[{"role":"user","content":prompt}],
        "tools":tools,"tool_choice":{"type":"auto"}})
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
    result.usage.search_calls = wire
        .pointer("/usage/server_tool_use/web_search_requests")
        .and_then(Value::as_u64);
    let content = wire
        .get("content")
        .and_then(Value::as_array)
        .ok_or_else(|| ResearchError::new("provider-response", "Messages content is missing"))?;
    let mut text = String::new();
    let mut tool_errors = Vec::new();
    for block in content {
        match block.get("type").and_then(Value::as_str) {
            Some("text") => {
                if let Some(part) = block.get("text").and_then(Value::as_str) {
                    text.push_str(part);
                }
                if let Some(citations) = block.get("citations").and_then(Value::as_array) {
                    for citation in citations {
                        if citation.get("type").and_then(Value::as_str)
                            == Some("web_search_result_location")
                        {
                            add_source(
                                result,
                                citation,
                                citation.get("cited_text").and_then(Value::as_str),
                            );
                        }
                    }
                }
            }
            Some("server_tool_use") => {
                text.clear();
            }
            Some("web_search_tool_result") => {
                text.clear();
                if let Some(sources) = block.get("content").and_then(Value::as_array) {
                    for source in sources {
                        if source.get("type").and_then(Value::as_str) == Some("web_search_result") {
                            // encrypted_content and page_age are not readable excerpts/publication dates.
                            add_source(result, source, None);
                        }
                    }
                } else {
                    if let Some(code) = block.pointer("/content/error_code").and_then(Value::as_str)
                    {
                        tool_errors.push(code);
                    }
                }
            }
            Some("web_fetch_tool_result") => {
                text.clear();
                let fetched = &block["content"];
                if fetched.get("type").and_then(Value::as_str) == Some("web_fetch_result") {
                    let mut source =
                        json!({"url":fetched["url"],"title":fetched.pointer("/content/title")});
                    if source["title"].is_null() {
                        source["title"] = Value::Null;
                    }
                    add_source(
                        result,
                        &source,
                        fetched
                            .pointer("/content/source/data")
                            .and_then(Value::as_str),
                    );
                } else {
                    if let Some(code) = fetched.get("error_code").and_then(Value::as_str) {
                        tool_errors.push(code);
                    }
                }
            }
            _ => {}
        }
    }
    let stop = wire.get("stop_reason").and_then(Value::as_str);
    if stop != Some("end_turn") {
        result.truncated |= matches!(stop, Some("max_tokens" | "pause_turn"));
        result
            .warnings
            .push("Messages turn incomplete; no paid continuation was attempted".into());
    }
    finish(request, result, &text);
    if !tool_errors.is_empty() {
        result.usage.cost_usd_estimate = None;
        result
            .warnings
            .push("Server tool errors make total billable cost unknown".into());
    }
    if tool_errors.contains(&"too_many_requests") {
        return Err(ResearchError::new(
            "rate-limited",
            "Provider server tool was rate limited",
        ));
    }
    if tool_errors.iter().any(|code| *code != "max_uses_exceeded") {
        return Err(ResearchError::new(
            "provider-tool-error",
            "Provider server tool failed",
        ));
    }
    if !tool_errors.is_empty() {
        result.truncated = true;
        result.warnings.push("Server tool use limit reached".into());
    }
    Ok(())
}
