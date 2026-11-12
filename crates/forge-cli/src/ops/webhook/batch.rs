//! `hook batch` — apply many webhook changes from one file in one process.
//!
//! The file is a JSON array of `{"target","hook","action","from","to"}` rows
//! (`set-url` with string values, `set-active` with boolean values). It is
//! validated completely before the first provider call. Rows are independent
//! and each is all-or-nothing (compare-and-swap, one write, read-back); a
//! failed row never stops the others, and any failed row makes the exit code
//! non-zero. The one exception is a read-back mismatch, which may mean a write
//! changed more than intended: the remaining rows are then skipped. Every row
//! runs under its own identity scope in this one process, so the verified
//! actor is reused instead of probed per row.

use std::collections::HashSet;
use std::path::Path;

use nils_common::cli_contract::{OutputFormat, exit, schema_version_for};
use nils_common::forge_identity::Operation;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::engine::{Change, HookSpec, Kind, RowReport, apply, schema_err};
use super::target::HookTarget;
use crate::backend::BackendRunner;
use crate::cli::{BINARY, GlobalFlags};
use crate::envelope::emit_success;
use crate::error::ForgeError;
use crate::provider::{Provider, ProviderContext};
use crate::rate_limit::default_runner;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRow {
    target: String,
    hook: u64,
    action: String,
    from: Value,
    to: Value,
}

#[derive(Serialize)]
struct BatchPayload {
    provider: &'static str,
    dry_run: bool,
    total: usize,
    failed: usize,
    rows: Vec<RowReport>,
}

fn invalid(message: String) -> ForgeError {
    ForgeError::validation(schema_err(), "batch_invalid", message, None)
}

fn change(row: &RawRow) -> Result<Change, String> {
    match (row.action.as_str(), &row.from, &row.to) {
        ("set-url", Value::String(from), Value::String(to)) => Ok(Change::Url {
            from: from.clone(),
            to: to.clone(),
        }),
        ("set-active", Value::Bool(from), Value::Bool(to)) => Ok(Change::Active {
            from: *from,
            to: *to,
        }),
        ("set-url", ..) => Err("set-url needs string from and to".to_string()),
        ("set-active", ..) => Err("set-active needs boolean from and to".to_string()),
        _ => Err("action must be set-url or set-active".to_string()),
    }
}

/// Read and fully validate the batch file; nothing here touches the provider.
fn load(file: &Path, kind: Kind) -> Result<Vec<HookSpec>, ForgeError> {
    let text = std::fs::read_to_string(file)
        .map_err(|_| invalid("unable to read the batch file".to_string()))?;
    let raw: Vec<RawRow> = serde_json::from_str(&text)
        .map_err(|error| invalid(format!("batch file is not a valid row array: {error}")))?;
    if raw.is_empty() {
        return Err(invalid("batch file has no rows".to_string()));
    }
    let mut seen = HashSet::new();
    let mut specs = Vec::with_capacity(raw.len());
    for (index, row) in raw.into_iter().enumerate() {
        let number = index + 1;
        let change = change(&row).map_err(|message| invalid(format!("row {number}: {message}")))?;
        let target = match kind {
            Kind::Repo => HookTarget::Repo(row.target),
            Kind::Org => HookTarget::Org(row.target),
        };
        let spec = HookSpec {
            target,
            hook: row.hook,
            change,
        };
        spec.check()
            .map_err(|issue| invalid(format!("row {number}: {}: {}", issue.kind, issue.message)))?;
        // One hook may carry several changes, but never the same field twice:
        // the second row's expected value would depend on the first.
        let key = (
            spec.target.name().to_ascii_lowercase(),
            spec.hook,
            spec.change.action(),
        );
        if !seen.insert(key) {
            return Err(invalid(format!(
                "row {number}: {} hook {} {} appears more than once",
                spec.target.name(),
                spec.hook,
                spec.change.action()
            )));
        }
        specs.push(spec);
    }
    Ok(specs)
}

fn run_row<R: BackendRunner>(
    runner: &R,
    ctx: &ProviderContext,
    spec: &HookSpec,
    dry_run: bool,
) -> Result<RowReport, ForgeError> {
    let target = spec.target.identity(&ctx.host).map_err(|error| {
        ForgeError::validation(schema_err(), "hook_target_invalid", error.to_string(), None)
    })?;
    let _scope = crate::identity::enter_scope(Some((target, Operation::ApiWrite)));
    apply(runner, ctx, spec, dry_run)
}

pub(super) fn run(
    global: &GlobalFlags,
    ctx: &ProviderContext,
    kind: Kind,
    file: &Path,
    format: OutputFormat,
) -> Result<i32, ForgeError> {
    let specs = load(file, kind)?;
    let runner = default_runner();
    let mut rows = Vec::with_capacity(specs.len());
    let mut aborted = false;
    for spec in &specs {
        if aborted {
            rows.push(RowReport::skipped(spec));
            continue;
        }
        rows.push(match run_row(&runner, ctx, spec, global.dry_run) {
            Ok(report) => report,
            Err(error) => {
                aborted = error.kind() == "hook_readback_mismatch";
                RowReport::failed(spec, &error)
            }
        });
    }
    let failed = rows
        .iter()
        .filter(|row| matches!(row.status, "failed" | "skipped"))
        .count();
    let code = emit_success(
        schema_version_for(BINARY, &kind.op("batch"), 1),
        BatchPayload {
            provider: Provider::GitHub.as_str(),
            dry_run: global.dry_run,
            total: rows.len(),
            failed,
            rows,
        },
        format,
        |payload| {
            for row in &payload.rows {
                println!("{}", row.line());
            }
        },
    );
    Ok(if code == exit::SUCCESS && failed > 0 {
        exit::RUNTIME
    } else {
        code
    })
}
