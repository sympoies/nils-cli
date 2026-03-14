use std::collections::BTreeMap;

use anyhow::Context;
use jaq_core::load::{Arena, File, Loader};
use jaq_core::{Ctx, Vars, data, unwrap_valr};
use jaq_json::{Val, write};

use crate::Result;

fn to_jaq_val(value: &serde_json::Value) -> Result<Val> {
    let v: Val = serde_json::from_value(value.clone())?;
    Ok(v)
}

fn from_jaq_val(value: &Val) -> Result<serde_json::Value> {
    let mut buf = Vec::new();
    write::write(&mut buf, &write::Pp::default(), 0, value).context("write jaq value as JSON")?;
    let v: serde_json::Value = serde_json::from_slice(&buf).context("parse jaq output as JSON")?;
    Ok(v)
}

fn compile_filter<'s>(
    expr: &'s str,
    global_vars: impl IntoIterator<Item = &'s str>,
) -> Result<jaq_core::compile::Filter<jaq_core::Native<data::JustLut<Val>>>> {
    let defs = jaq_core::defs()
        .chain(jaq_std::defs())
        .chain(jaq_json::defs());
    let loader = Loader::new(defs);
    let arena = Arena::default();
    let modules = loader
        .load(
            &arena,
            File {
                code: expr,
                path: (),
            },
        )
        .map_err(|errs| anyhow::anyhow!("{errs:?}"))
        .with_context(|| format!("jq parse failed: {expr:?}"))?;

    let funs = jaq_core::funs()
        .chain(jaq_std::funs())
        .chain(jaq_json::funs());
    let compiler = jaq_core::Compiler::default()
        .with_funs(funs)
        .with_global_vars(global_vars);

    compiler
        .compile(modules)
        .map_err(|errs| anyhow::anyhow!("{errs:?}"))
        .with_context(|| format!("jq compile failed: {expr:?}"))
}

pub fn query(value: &serde_json::Value, expr: &str) -> Result<Vec<serde_json::Value>> {
    query_with_vars(value, expr, &BTreeMap::new())
}

pub fn query_with_vars(
    value: &serde_json::Value,
    expr: &str,
    vars: &BTreeMap<String, serde_json::Value>,
) -> Result<Vec<serde_json::Value>> {
    let input = to_jaq_val(value).context("convert input JSON to jq value")?;

    let global_var_names: Vec<String> = vars.keys().map(|k| format!("${k}")).collect();
    let global_var_slices: Vec<&str> = global_var_names.iter().map(String::as_str).collect();
    let filter = compile_filter(expr, global_var_slices)?;

    let mut global_var_values: Vec<Val> = Vec::with_capacity(vars.len());
    for v in vars.values() {
        global_var_values.push(to_jaq_val(v)?);
    }

    let ctx = Ctx::<data::JustLut<Val>>::new(&filter.lut, Vars::new(global_var_values));

    let mut out = Vec::new();
    for y in filter
        .id
        .run((ctx, input))
        .map(unwrap_valr)
        .collect::<Vec<_>>()
    {
        let y = y
            .map_err(|e| anyhow::anyhow!("{e:?}"))
            .context("jq runtime error")?;
        out.push(from_jaq_val(&y).context("convert jq output to JSON")?);
    }

    Ok(out)
}

/// Evaluate a jq expression like `jq -e`: returns `true` if the last output value is truthy.
///
/// Truthiness matches jq:
/// - `false` and `null` are falsey
/// - all other values are truthy
/// - no output is treated as falsey
pub fn eval_exit_status(value: &serde_json::Value, expr: &str) -> Result<bool> {
    let out = query(value, expr)?;
    let Some(last) = out.last() else {
        return Ok(false);
    };
    Ok(!matches!(
        last,
        serde_json::Value::Null | serde_json::Value::Bool(false)
    ))
}

/// Evaluate a jq expression and return raw output lines similar to `jq -r`.
///
/// - strings are emitted without JSON quotes
/// - all other types are emitted as compact JSON per line
pub fn query_raw(value: &serde_json::Value, expr: &str) -> Result<Vec<String>> {
    let out = query(value, expr)?;
    let mut lines = Vec::with_capacity(out.len());
    for v in out {
        match v {
            serde_json::Value::String(s) => lines.push(s),
            other => lines.push(serde_json::to_string(&other)?),
        }
    }
    Ok(lines)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn jq_basic_query_outputs_multiple_values() {
        let input = serde_json::json!(["a", "b"]);
        let out = query(&input, ".[]").unwrap();
        assert_eq!(out, vec![serde_json::json!("a"), serde_json::json!("b")]);
    }

    #[test]
    fn jq_eval_exit_status_matches_truthiness() {
        let input = serde_json::json!({"a": 1});
        assert!(eval_exit_status(&input, ".a == 1").unwrap());
        assert!(!eval_exit_status(&input, ".a == 2").unwrap());
        assert!(!eval_exit_status(&input, "empty").unwrap());
    }

    #[test]
    fn jq_vars_support_arg_like_usage() {
        let input = serde_json::json!({"data": {"login": {"accessToken": "t"}}});
        let mut vars = BTreeMap::new();
        vars.insert("field".to_string(), serde_json::json!("login"));

        let out = query_with_vars(&input, ".data[$field].accessToken", &vars).unwrap();
        assert_eq!(out, vec![serde_json::json!("t")]);
    }

    #[test]
    fn jq_query_raw_unwraps_strings() {
        let input = serde_json::json!({"token": "abc"});
        let out = query_raw(&input, ".token").unwrap();
        assert_eq!(out, vec!["abc".to_string()]);
    }

    #[test]
    fn jq_parse_errors_include_expression() {
        let input = serde_json::json!({});
        let err = query(&input, ".[").unwrap_err();
        assert!(format!("{err:#}").contains(".["));
    }
}
