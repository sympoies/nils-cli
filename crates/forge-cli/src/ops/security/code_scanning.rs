//! Exact-ref code-scanning diagnostics through the ordinary identity-bound runner.
use nils_common::cli_contract::{OutputFormat, schema_version_for};
use serde_json::{Value, json};

use crate::backend::{BackendCall, BackendProgram, BackendRunner};
use crate::cli::{BINARY, GlobalFlags, SecurityAlertViewArgs};
use crate::envelope::emit_success;
use crate::error::ForgeError;
use crate::provider::ProviderContext;
use crate::rate_limit::default_runner;

use super::{api_argv, dry_run, parse, schema, text};

pub(super) fn view(
    ctx: &ProviderContext,
    repo: &str,
    global: &GlobalFlags,
    args: SecurityAlertViewArgs,
    format: OutputFormat,
) -> Result<i32, ForgeError> {
    let version = schema_version_for(BINARY, "security.alerts.view", 1);
    let endpoint = format!("repos/{repo}/code-scanning/alerts/{}", args.number);
    let call = get(ctx, &endpoint);
    if global.dry_run {
        return dry_run(version, ctx, call, format);
    }
    let runner = default_runner();
    let alert = parse(&runner.run(&call)?.stdout)?;
    if alert.get("number").and_then(Value::as_u64) != Some(args.number)
        || text(&alert, "/rule/id").is_none()
    {
        return Err(ForgeError::software(
            schema(),
            "invalid code-scanning alert response",
            None,
        ));
    }
    // The alert's most_recent_instance may belong to the default branch. Never
    // use it as evidence for the explicitly requested ref and commit.
    let instances: Vec<_> = pages(
        &runner,
        ctx,
        &format!("{endpoint}/instances"),
        &args.reference,
    )?
    .into_iter()
    .filter(|row| matches_subject(row, &args))
    .map(|row| {
        select(
            &row,
            &[
                "ref",
                "commit_sha",
                "analysis_key",
                "environment",
                "category",
                "state",
                "message",
                "location",
                "html_url",
                "classifications",
            ],
        )
    })
    .collect();
    if instances.is_empty() {
        return Err(ForgeError::validation(
            schema(),
            "security_instance_not_found",
            "no alert instance matches the requested ref and commit",
            None,
        ));
    }
    let mut analyses = Vec::new();
    if args.include_sarif {
        let candidates = pages(
            &runner,
            ctx,
            &format!("repos/{repo}/code-scanning/analyses"),
            &args.reference,
        )?;
        for row in candidates
            .into_iter()
            .filter(|row| matches_subject(row, &args))
        {
            if !instances
                .iter()
                .any(|instance| same_configuration(instance, &row))
                || row.pointer("/tool/name") != alert.pointer("/tool/name")
            {
                continue;
            }
            let id = row.get("id").and_then(Value::as_u64).ok_or_else(|| {
                ForgeError::software(schema(), "code-scanning analysis omitted its id", None)
            })?;
            let mut call = get(ctx, &format!("repos/{repo}/code-scanning/analyses/{id}"));
            call.argv
                .extend(["-H".into(), "Accept:application/sarif+json".into()]);
            let sarif = parse(&runner.run(&call)?.stdout)?;
            if sarif.get("version").and_then(Value::as_str) != Some("2.1.0")
                || !sarif.get("runs").is_some_and(Value::is_array)
            {
                return Err(ForgeError::software(
                    schema(),
                    "analysis response is not SARIF 2.1.0",
                    None,
                ));
            }
            // Keep the provider's result/alert properties and all codeFlows.
            // This is analysis-level evidence, not an invented alert-specific
            // trace or a claim that missing codeFlows means a safe result.
            analyses.push(
                json!({"metadata": select(&row, &["id", "ref", "commit_sha", "analysis_key",
                "environment", "category", "tool", "created_at", "results_count", "rules_count",
                "sarif_id", "error", "warning"]), "sarif": sarif}),
            );
        }
        if analyses.is_empty() {
            return Err(ForgeError::validation(
                schema(),
                "security_analysis_not_found",
                "no analysis matches the selected instance configuration, ref and commit",
                None,
            ));
        }
    }
    let payload = json!({"provider": "github", "host": ctx.host, "repo": repo,
        "kind": "code-scanning", "reference": args.reference, "commit_sha": args.commit,
        "alert": select(&alert, &["number", "state", "html_url", "rule", "tool", "created_at",
            "updated_at", "dismissed_at", "dismissed_reason", "dismissed_comment", "fixed_at"]),
        "instances": instances, "sarif_requested": args.include_sarif,
        "sarif_scope": "analysis", "analyses": analyses});
    Ok(emit_success(version, payload, format, |p| {
        println!(
            "{} alert #{}: {} matching instances at {} ({})",
            repo,
            args.number,
            p["instances"].as_array().map_or(0, Vec::len),
            p["reference"].as_str().unwrap_or_default(),
            p["commit_sha"].as_str().unwrap_or_default()
        );
        println!("{}", serde_json::to_string_pretty(p).unwrap_or_default());
    }))
}

fn get(ctx: &ProviderContext, endpoint: &str) -> BackendCall {
    let mut argv = api_argv(ctx);
    argv.extend(["-X".into(), "GET".into(), endpoint.into()]);
    BackendCall::new(BackendProgram::Gh, argv).with_host(ctx.provider, &ctx.host)
}

fn pages(
    runner: &impl BackendRunner,
    ctx: &ProviderContext,
    endpoint: &str,
    reference: &str,
) -> Result<Vec<Value>, ForgeError> {
    let mut result = Vec::new();
    // Explicit bounded reads, never provider-supplied continuation URLs.
    for page in 1..=10 {
        let mut call = get(ctx, endpoint);
        call.argv.extend([
            "-f".into(),
            format!("ref={reference}").into(),
            "-f".into(),
            "per_page=100".into(),
            "-f".into(),
            format!("page={page}").into(),
        ]);
        let value = parse(&runner.run(&call)?.stdout)?;
        let rows = value.as_array().ok_or_else(|| {
            ForgeError::software(schema(), "code-scanning page must be an array", None)
        })?;
        result.extend(rows.iter().cloned());
        if rows.len() < 100 {
            return Ok(result);
        }
    }
    Err(ForgeError::unavailable(
        schema(),
        "security_read_incomplete",
        "code-scanning pagination reached the 1000-row bound; no complete result emitted",
        None,
    ))
}

fn matches_subject(row: &Value, args: &SecurityAlertViewArgs) -> bool {
    row.get("ref").and_then(Value::as_str) == Some(args.reference.as_str())
        && row.get("commit_sha").and_then(Value::as_str) == Some(args.commit.as_str())
}

fn same_configuration(instance: &Value, analysis: &Value) -> bool {
    ["analysis_key", "environment", "category"]
        .iter()
        .all(|field| instance.get(field) == analysis.get(field))
        && instance.get("analysis_key").is_some_and(Value::is_string)
}

fn select(row: &Value, fields: &[&str]) -> Value {
    Value::Object(
        fields
            .iter()
            .filter_map(|field| {
                row.get(field)
                    .map(|value| ((*field).to_string(), value.clone()))
            })
            .collect(),
    )
}
