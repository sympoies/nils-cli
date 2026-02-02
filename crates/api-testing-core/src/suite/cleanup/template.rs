use std::collections::BTreeMap;

use anyhow::Context;

use crate::Result;

fn jq_extract_first_string(value: &serde_json::Value, expr: &str) -> Option<String> {
    let lines = crate::jq::query_raw(value, expr).ok()?;
    let first = lines.into_iter().next()?.trim().to_string();
    if first.is_empty() || first == "null" {
        None
    } else {
        Some(first)
    }
}

pub(super) fn parse_vars_map(vars: Option<&serde_json::Value>) -> Result<BTreeMap<String, String>> {
    let Some(vars) = vars else {
        return Ok(BTreeMap::new());
    };
    if vars.is_null() {
        return Ok(BTreeMap::new());
    }
    let obj = vars.as_object().context("cleanup.vars must be an object")?;
    let mut out = BTreeMap::new();
    for (k, v) in obj {
        let Some(expr) = v.as_str() else {
            anyhow::bail!("cleanup.vars values must be strings");
        };
        out.insert(k.clone(), expr.to_string());
    }
    Ok(out)
}

pub(super) fn render_template(
    template: &str,
    response_json: &serde_json::Value,
    vars: &BTreeMap<String, String>,
) -> Result<String> {
    let mut out = template.to_string();
    for (key, expr) in vars {
        let Some(value) = jq_extract_first_string(response_json, expr) else {
            anyhow::bail!("template var '{key}' failed to resolve");
        };
        out = out.replace(&format!("{{{{{key}}}}}"), &value);
    }
    Ok(out)
}
