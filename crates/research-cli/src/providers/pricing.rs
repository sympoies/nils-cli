//! Informational Standard/global Haiku 5.5 rate snapshot, verified 2026-11-01.
//! Source: https://platform.claude.com/docs/en/about-claude/pricing
//! Unknown categories, cache write duration, model, or usage leave cost unknown.
use crate::{Provider, ResearchResult};
use serde_json::Value;
pub fn estimate(result: &mut ResearchResult) {
    if result.provider != Some(Provider::Anthropic)
        || result.model.as_deref() != Some("claude-haiku-5-5")
    {
        return;
    }
    let Some(raw) = &result.usage.raw else {
        return;
    };
    result.usage.cost_usd_estimate = standard_cost(raw);
}

fn standard_cost(raw: &Value) -> Option<f64> {
    let input = raw.get("input_tokens")?.as_u64()?;
    let output = raw.get("output_tokens")?.as_u64()?;
    let reads = raw.get("cache_read_input_tokens")?.as_u64()?;
    let writes = raw.get("cache_creation_input_tokens")?.as_u64()?;
    let searches = raw
        .pointer("/server_tool_use/web_search_requests")?
        .as_u64()?;
    // Requests never configure caching; nonzero writes require a TTL-specific price.
    if writes != 0 {
        return None;
    }
    if let Some(tools) = raw.get("server_tool_use").and_then(Value::as_object)
        && tools
            .keys()
            .any(|k| !matches!(k.as_str(), "web_search_requests" | "web_fetch_requests"))
    {
        return None;
    }
    // Tool iterations aggregate usage; over the threshold, the per-pass tier is
    // unknown. Do not apply a guessed long-context multiplier to that aggregate.
    if input.checked_add(reads)? > 100_000 {
        return None;
    }
    let (input_rate, output_rate, read_rate) = (0.1, 0.5, 0.01);
    Some(
        (input as f64 * input_rate + output as f64 * output_rate + reads as f64 * read_rate)
            / 1_000_000.0
            + searches as f64 * 0.01,
    )
}
