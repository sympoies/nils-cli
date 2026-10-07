//! GitHub security metadata reads. Alert output selects fields explicitly so
//! secret-scanning findings never serialize scanned secret values.
use nils_common::cli_contract::{OutputFormat, schema_version_for};
use serde::Serialize;
use std::ffi::OsString;

use crate::backend::{BackendCall, BackendProgram, BackendRunner, DryRunPayload};
use crate::cli::{
    BINARY, GlobalFlags, SecurityAlertKind, SecurityAlertListArgs, SecurityAlertsCommand,
    SecurityCommand, SecuritySettingsCommand,
};
use crate::envelope::emit_success;
use crate::error::ForgeError;
use crate::provider::{Provider, ProviderContext, detect, git_remote_url};
use crate::rate_limit::default_runner;

#[derive(Serialize)]
struct Alert {
    number: u64,
    state: String,
    url: String,
    title: Option<String>,
    severity: Option<String>,
    created_at: Option<String>,
    updated_at: Option<String>,
    dismissed_at: Option<String>,
    fixed_at: Option<String>,
    resolved_at: Option<String>,
}
#[derive(Serialize)]
struct Alerts {
    provider: &'static str,
    host: String,
    repo: String,
    kind: &'static str,
    state: String,
    limit: u32,
    limited: bool,
    items: Vec<Alert>,
}
#[derive(Serialize)]
struct Settings {
    provider: &'static str,
    host: String,
    repo: String,
    security_and_analysis: Option<serde_json::Value>,
}

pub fn run(
    global: &GlobalFlags,
    command: SecurityCommand,
    format: OutputFormat,
) -> Result<i32, ForgeError> {
    let ctx = detect(
        global.provider_hint(),
        &global.remote,
        global.repo.as_deref(),
        git_remote_url,
    )?;
    if ctx.provider != Provider::GitHub {
        return Err(ForgeError::provider_unsupported(
            schema(),
            "security reads are GitHub-only",
            None,
        ));
    }
    let repo = ctx.repo.as_deref().ok_or_else(|| {
        ForgeError::validation(
            schema(),
            "repo_required",
            "security reads require --repo owner/repo or a detected remote",
            None,
        )
    })?;
    match command {
        SecurityCommand::Alerts(args) => match args.command {
            SecurityAlertsCommand::List(args) => alerts(&ctx, repo, global, args, format),
        },
        SecurityCommand::Settings(args) => match args.command {
            SecuritySettingsCommand::View => settings(&ctx, repo, global, format),
        },
    }
}

fn alerts(
    ctx: &ProviderContext,
    repo: &str,
    global: &GlobalFlags,
    args: SecurityAlertListArgs,
    format: OutputFormat,
) -> Result<i32, ForgeError> {
    let allowed: &[&str] = match args.kind {
        SecurityAlertKind::Dependabot => &["open", "dismissed", "fixed", "auto_dismissed", "all"],
        SecurityAlertKind::CodeScanning => &["open", "dismissed", "fixed", "all"],
        SecurityAlertKind::SecretScanning => &["open", "resolved", "all"],
    };
    if !allowed.contains(&args.state.as_str()) {
        return Err(ForgeError::validation(
            schema(),
            "alert_state_invalid",
            format!(
                "state is not valid for {}; expected {}",
                args.kind.as_str(),
                allowed.join(", ")
            ),
            None,
        ));
    }
    let version = schema_version_for(BINARY, "security.alerts.list", 1);
    if global.dry_run {
        return dry_run(version, ctx, alert_call(ctx, repo, &args, 1, None), format);
    }
    let runner = default_runner();
    let mut items = Vec::new();
    let mut page = 1u32;
    let mut after = None;
    loop {
        let output = runner.run(&alert_call(ctx, repo, &args, page, after.as_deref()))?;
        let (value, next) = if matches!(args.kind, SecurityAlertKind::Dependabot) {
            dependabot_page(&output.stdout)?
        } else {
            (parse(&output.stdout)?, None)
        };
        let rows = value.as_array().ok_or_else(|| {
            ForgeError::software(schema(), "security alert JSON must be an array", None)
        })?;
        for row in rows
            .iter()
            .take((args.limit as usize).saturating_sub(items.len()))
        {
            items.push(Alert {
                number: row
                    .get("number")
                    .and_then(|v| v.as_u64())
                    .ok_or_else(|| missing("number"))?,
                state: text(row, "/state").ok_or_else(|| missing("state"))?,
                url: text(row, "/html_url").ok_or_else(|| missing("html_url"))?,
                title: match args.kind {
                    SecurityAlertKind::Dependabot => text(row, "/security_advisory/summary"),
                    SecurityAlertKind::CodeScanning => text(row, "/rule/description"),
                    SecurityAlertKind::SecretScanning => text(row, "/secret_type_display_name"),
                },
                severity: match args.kind {
                    SecurityAlertKind::Dependabot => text(row, "/security_advisory/severity"),
                    SecurityAlertKind::CodeScanning => text(row, "/rule/security_severity_level")
                        .or_else(|| text(row, "/rule/severity")),
                    SecurityAlertKind::SecretScanning => None,
                },
                created_at: text(row, "/created_at"),
                updated_at: text(row, "/updated_at"),
                dismissed_at: text(row, "/dismissed_at"),
                fixed_at: text(row, "/fixed_at"),
                resolved_at: text(row, "/resolved_at"),
            });
        }
        if items.len() >= args.limit as usize {
            break;
        }
        if matches!(args.kind, SecurityAlertKind::Dependabot) {
            let Some(next) = next else { break };
            after = Some(next);
        } else {
            if rows.len() < 100 {
                break;
            }
            page += 1;
        }
    }
    let payload = Alerts {
        provider: "github",
        host: ctx.host.clone(),
        repo: repo.to_string(),
        kind: args.kind.as_str(),
        state: args.state,
        limit: args.limit,
        limited: items.len() >= args.limit as usize,
        items,
    };
    Ok(emit_success(version, payload, format, |p| {
        for alert in &p.items {
            println!(
                "#{} [{}] {} - {}",
                alert.number,
                alert.state,
                alert.title.as_deref().unwrap_or(""),
                alert.url
            );
        }
    }))
}
fn alert_call(
    ctx: &ProviderContext,
    repo: &str,
    args: &SecurityAlertListArgs,
    page: u32,
    after: Option<&str>,
) -> BackendCall {
    let mut argv = api_argv(ctx);
    argv.extend([
        "-X".into(),
        "GET".into(),
        format!("repos/{repo}/{}/alerts", args.kind.as_str()).into(),
    ]);
    if args.state != "all" {
        argv.extend(["-f".into(), format!("state={}", args.state).into()]);
    }
    argv.extend(["-f".into(), "per_page=100".into()]);
    if matches!(args.kind, SecurityAlertKind::Dependabot) {
        argv.push("--include".into());
        if let Some(after) = after {
            argv.extend(["-f".into(), format!("after={after}").into()]);
        }
    } else {
        argv.extend(["-f".into(), format!("page={page}").into()]);
    }
    BackendCall::new(BackendProgram::Gh, argv).with_host(ctx.provider, &ctx.host)
}

fn dependabot_page(stdout: &str) -> Result<(serde_json::Value, Option<String>), ForgeError> {
    let (headers, body) = stdout
        .split_once("\r\n\r\n")
        .or_else(|| stdout.split_once("\n\n"))
        .ok_or_else(|| {
            ForgeError::software(schema(), "Dependabot response omitted HTTP headers", None)
        })?;
    let mut next = None;
    for line in headers.lines() {
        let Some((name, links)) = line.split_once(':') else {
            continue;
        };
        if !name.eq_ignore_ascii_case("link") {
            continue;
        }
        for link in links.split(',') {
            let mut parts = link.trim().split(';');
            let target = parts.next().unwrap_or_default().trim();
            if !parts.any(|part| part.trim() == "rel=\"next\"") {
                continue;
            }
            // Read only the opaque cursor; the next request keeps the selected
            // repository and authority instead of following the provider URL.
            next = target
                .strip_prefix('<')
                .and_then(|target| target.strip_suffix('>'))
                .and_then(|target| url::Url::parse(target).ok())
                .and_then(|target| {
                    target.query_pairs().find_map(|(key, value)| {
                        (key == "after" && !value.is_empty()).then(|| value.into_owned())
                    })
                });
            if next.is_none() {
                return Err(ForgeError::software(
                    schema(),
                    "Dependabot next-page link omitted a valid after cursor",
                    None,
                ));
            }
        }
    }
    Ok((parse(body)?, next))
}
fn settings(
    ctx: &ProviderContext,
    repo: &str,
    global: &GlobalFlags,
    format: OutputFormat,
) -> Result<i32, ForgeError> {
    let mut argv = api_argv(ctx);
    argv.push(format!("repos/{repo}").into());
    let call = BackendCall::new(BackendProgram::Gh, argv).with_host(ctx.provider, &ctx.host);
    let version = schema_version_for(BINARY, "security.settings.view", 1);
    if global.dry_run {
        return dry_run(version, ctx, call, format);
    }
    let output = default_runner().run(&call)?;
    let value = parse(&output.stdout)?;
    if !value.is_object() {
        return Err(ForgeError::software(
            schema(),
            "security settings JSON must be an object",
            None,
        ));
    }
    let block = value.get("security_and_analysis").filter(|v| !v.is_null());
    if block.is_some_and(|v| !v.is_object()) {
        return Err(ForgeError::software(
            schema(),
            "security_and_analysis must be an object or null",
            None,
        ));
    }
    let payload = Settings {
        provider: "github",
        host: ctx.host.clone(),
        repo: repo.to_string(),
        security_and_analysis: block.cloned(),
    };
    Ok(emit_success(version, payload, format, |p| {
        println!(
            "{} security settings: {}",
            p.repo,
            p.security_and_analysis
                .as_ref()
                .map(serde_json::Value::to_string)
                .unwrap_or_else(|| "unavailable".to_string())
        );
    }))
}
fn api_argv(ctx: &ProviderContext) -> Vec<OsString> {
    let mut argv = vec!["api".into()];
    ctx.push_github_api_hostname(&mut argv);
    argv
}
fn dry_run(
    version: String,
    ctx: &ProviderContext,
    call: BackendCall,
    format: OutputFormat,
) -> Result<i32, ForgeError> {
    Ok(emit_success(
        version,
        DryRunPayload::new(ctx.provider, &call),
        format,
        |p| println!("would run: {}", p.plan.join(" ")),
    ))
}
fn text(row: &serde_json::Value, pointer: &str) -> Option<String> {
    row.pointer(pointer)
        .and_then(|v| v.as_str())
        .map(str::to_string)
}
fn parse(stdout: &str) -> Result<serde_json::Value, ForgeError> {
    serde_json::from_str(stdout.trim()).map_err(|e| {
        ForgeError::software(
            schema(),
            "security read JSON is invalid",
            Some(e.to_string()),
        )
    })
}
fn missing(field: &str) -> ForgeError {
    ForgeError::software(
        schema(),
        format!("security alert is missing '{field}'"),
        None,
    )
}
fn schema() -> String {
    schema_version_for(BINARY, "error", 1)
}
