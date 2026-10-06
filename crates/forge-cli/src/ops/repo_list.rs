//! GitHub organization repository inventory, bounded after client filters.
use std::ffi::OsString;

use nils_common::cli_contract::{OutputFormat, schema_version_for};
use serde::Serialize;

use crate::backend::{BackendCall, BackendProgram, BackendRunner, DryRunPayload};
use crate::cli::{BINARY, GlobalFlags, RepoListArgs};
use crate::envelope::emit_success;
use crate::error::ForgeError;
use crate::provider::{Provider, ProviderContext, detect_unscoped, git_remote_url};
use crate::rate_limit::default_runner;

#[derive(Debug, Serialize)]
struct Repository {
    name: String,
    full_name: String,
    url: String,
    private: bool,
    fork: bool,
    archived: bool,
    default_branch: Option<String>,
    updated_at: Option<String>,
}

#[derive(Serialize)]
struct Payload {
    provider: &'static str,
    host: String,
    org: String,
    limit: u32,
    limited: bool,
    items: Vec<Repository>,
}

pub fn run(
    global: &GlobalFlags,
    args: RepoListArgs,
    format: OutputFormat,
) -> Result<i32, ForgeError> {
    if global.repo.is_some() {
        return Err(ForgeError::validation(
            schema(),
            "repo_conflict",
            "repo list targets --org; omit --repo",
            None,
        ));
    }
    if args.org.is_empty()
        || !args
            .org
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return Err(ForgeError::validation(
            schema(),
            "org_invalid",
            "--org must be a GitHub organization login",
            None,
        ));
    }
    let ctx = detect_unscoped(global.provider_hint(), &global.remote, None, git_remote_url)?;
    if ctx.provider != Provider::GitHub {
        return Err(ForgeError::provider_unsupported(
            schema(),
            "repo list is GitHub-only",
            None,
        ));
    }
    let version = schema_version_for(BINARY, "repo.list", 1);
    if global.dry_run {
        let payload = DryRunPayload::new(ctx.provider, &call(&ctx, &args, 1));
        return Ok(emit_success(version, payload, format, |p| {
            println!("would run: {}", p.plan.join(" "))
        }));
    }
    let runner = default_runner();
    let mut items = Vec::new();
    let mut page = 1u32;
    loop {
        let output = runner.run(&call(&ctx, &args, page))?;
        let raw: serde_json::Value = serde_json::from_str(output.stdout.trim()).map_err(|e| {
            ForgeError::software(
                schema(),
                "repository list JSON is invalid",
                Some(e.to_string()),
            )
        })?;
        let rows = raw.as_array().ok_or_else(|| {
            ForgeError::software(schema(), "repository list JSON must be an array", None)
        })?;
        for row in rows {
            let archived = boolean(row, "archived")?;
            let fork = boolean(row, "fork")?;
            if (args.no_archived && archived) || (args.source && fork) {
                continue;
            }
            items.push(Repository {
                name: string(row, "name")?,
                full_name: string(row, "full_name")?,
                url: string(row, "html_url")?,
                private: boolean(row, "private")?,
                fork,
                archived,
                default_branch: optional(row, "default_branch"),
                updated_at: optional(row, "updated_at"),
            });
            if items.len() >= args.limit as usize {
                break;
            }
        }
        if items.len() >= args.limit as usize || rows.len() < 100 {
            break;
        }
        page += 1;
    }
    let payload = Payload {
        provider: "github",
        host: ctx.host,
        org: args.org,
        limit: args.limit,
        limited: items.len() >= args.limit as usize,
        items,
    };
    Ok(emit_success(version, payload, format, |p| {
        for item in &p.items {
            println!("{} - {}", item.full_name, item.url);
        }
    }))
}

fn call(ctx: &ProviderContext, args: &RepoListArgs, page: u32) -> BackendCall {
    let mut argv = vec![OsString::from("api")];
    ctx.push_github_api_hostname(&mut argv);
    argv.extend([
        "-X".into(),
        "GET".into(),
        format!("orgs/{}/repos", args.org).into(),
        "-f".into(),
        format!("type={}", if args.source { "sources" } else { "all" }).into(),
        "-f".into(),
        "per_page=100".into(),
        "-f".into(),
        format!("page={page}").into(),
    ]);
    BackendCall::new(BackendProgram::Gh, argv).with_host(ctx.provider, &ctx.host)
}
fn string(row: &serde_json::Value, key: &str) -> Result<String, ForgeError> {
    optional(row, key).ok_or_else(|| {
        ForgeError::software(
            schema(),
            format!("repository list is missing '{key}'"),
            None,
        )
    })
}
fn optional(row: &serde_json::Value, key: &str) -> Option<String> {
    row.get(key).and_then(|v| v.as_str()).map(str::to_string)
}
fn boolean(row: &serde_json::Value, key: &str) -> Result<bool, ForgeError> {
    row.get(key).and_then(|v| v.as_bool()).ok_or_else(|| {
        ForgeError::software(
            schema(),
            format!("repository list is missing boolean '{key}'"),
            None,
        )
    })
}
fn schema() -> String {
    schema_version_for(BINARY, "error", 1)
}
