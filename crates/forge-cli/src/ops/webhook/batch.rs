//! `hook batch` — apply many webhook changes from one file in one process.
//!
//! The JSON array selects `set-url`, `set-active`, or `redeliver`, with the
//! fields of that action. It is validated before the first provider call.
//! Change rows compare-and-swap one field and read it back; redeliver rows emit
//! one result per selected delivery. A failed row makes the exit code non-zero
//! and does not stop the others. The exception is a hook read-back mismatch, which may mean a write
//! changed more than intended: the remaining rows are then skipped. Every row
//! runs under its own identity scope in this one process, so the verified
//! actor is reused instead of probed per row.

use std::collections::HashSet;
use std::path::Path;

use nils_common::cli_contract::{OutputFormat, exit, schema_version_for};
use nils_common::forge_identity::Operation;
use serde::{Deserialize, Serialize};

use super::engine::{Change, HookSpec, Kind, RowReport, apply, schema_err};
use super::redeliver::{DeliveryReport, RedeliverSpec};
use super::selection::Selection;
use super::target::HookTarget;
use crate::backend::BackendRunner;
use crate::cli::{BINARY, GlobalFlags, HookDeliveryStatus};
use crate::envelope::emit_success;
use crate::error::ForgeError;
use crate::provider::{Provider, ProviderContext};
use crate::rate_limit::default_runner;

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "kebab-case", deny_unknown_fields)]
enum RawRow {
    SetUrl {
        target: String,
        hook: u64,
        from: String,
        to: String,
    },
    SetActive {
        target: String,
        hook: u64,
        from: bool,
        to: bool,
    },
    Redeliver {
        target: String,
        hook: u64,
        #[serde(default)]
        status: Option<HookDeliveryStatus>,
        since: Option<String>,
        until: Option<String>,
        delivery_ids: Option<Vec<u64>>,
    },
}

enum BatchSpec {
    Change(HookSpec),
    Delivery(RedeliverSpec),
}

impl BatchSpec {
    fn target(&self) -> &HookTarget {
        match self {
            Self::Change(spec) => &spec.target,
            Self::Delivery(spec) => &spec.target,
        }
    }

    fn key(&self) -> (String, u64, &'static str) {
        let (hook, action) = match self {
            Self::Change(spec) => (spec.hook, spec.change.action()),
            Self::Delivery(spec) => (spec.hook, "redeliver"),
        };
        (self.target().name().to_ascii_lowercase(), hook, action)
    }

    fn failed(&self, error: &ForgeError) -> BatchReport {
        match self {
            Self::Change(spec) => BatchReport::Change(RowReport::failed(spec, error)),
            Self::Delivery(spec) => {
                BatchReport::Delivery(DeliveryReport::failed(spec, None, error))
            }
        }
    }

    fn skipped(&self) -> BatchReport {
        match self {
            Self::Change(spec) => BatchReport::Change(RowReport::skipped(spec)),
            Self::Delivery(spec) => BatchReport::Delivery(DeliveryReport::skipped(spec)),
        }
    }
}

#[derive(Serialize)]
#[serde(untagged)]
pub(super) enum BatchReport {
    Change(RowReport),
    Delivery(DeliveryReport),
}

impl BatchReport {
    fn status(&self) -> &'static str {
        match self {
            Self::Change(row) => row.status,
            Self::Delivery(row) => row.status,
        }
    }

    fn line(&self) -> String {
        match self {
            Self::Change(row) => row.line(),
            Self::Delivery(row) => row.line(),
        }
    }
}

#[derive(Serialize)]
struct BatchPayload {
    provider: &'static str,
    dry_run: bool,
    total: usize,
    failed: usize,
    rows: Vec<BatchReport>,
}

fn invalid(message: String) -> ForgeError {
    ForgeError::validation(schema_err(), "batch_invalid", message, None)
}

/// Read and fully validate the batch file; nothing here touches the provider.
fn load(file: &Path, kind: Kind) -> Result<Vec<BatchSpec>, ForgeError> {
    let text = std::fs::read_to_string(file)
        .map_err(|_| invalid("unable to read the batch file".to_string()))?;
    let raw: Vec<RawRow> = serde_json::from_str(&text)
        .map_err(|_| invalid("batch file is not a valid row array".to_string()))?;
    if raw.is_empty() {
        return Err(invalid("batch file has no rows".to_string()));
    }
    let mut seen = HashSet::new();
    let mut specs = Vec::with_capacity(raw.len());
    for (index, row) in raw.into_iter().enumerate() {
        let number = index + 1;
        let target = |name| match kind {
            Kind::Repo => HookTarget::Repo(name),
            Kind::Org => HookTarget::Org(name),
        };
        let spec = match row {
            RawRow::SetUrl {
                target: name,
                hook,
                from,
                to,
            } => BatchSpec::Change(HookSpec {
                target: target(name),
                hook,
                change: Change::Url { from, to },
            }),
            RawRow::SetActive {
                target: name,
                hook,
                from,
                to,
            } => BatchSpec::Change(HookSpec {
                target: target(name),
                hook,
                change: Change::Active { from, to },
            }),
            RawRow::Redeliver {
                target: name,
                hook,
                status,
                since,
                until,
                delivery_ids,
            } => {
                let selection =
                    Selection::new(delivery_ids, status, since.as_deref(), until.as_deref())
                        .map_err(|error| invalid(format!("row {number}: {}", error.message())))?;
                BatchSpec::Delivery(RedeliverSpec {
                    target: target(name),
                    hook,
                    selection,
                })
            }
        };
        match &spec {
            BatchSpec::Change(spec) => spec.check().map_err(|issue| {
                invalid(format!("row {number}: {}: {}", issue.kind, issue.message))
            })?,
            BatchSpec::Delivery(spec) => spec
                .check()
                .map_err(|error| invalid(format!("row {number}: {}", error.message())))?,
        }
        // One hook may carry several changes, but never the same field twice:
        // the second row's expected value would depend on the first.
        if !seen.insert(spec.key()) {
            return Err(invalid(format!(
                "row {number}: target, hook and action appear more than once"
            )));
        }
        specs.push(spec);
    }
    Ok(specs)
}

fn run_row<R: BackendRunner>(
    runner: &R,
    ctx: &ProviderContext,
    spec: &BatchSpec,
    dry_run: bool,
) -> Result<Vec<BatchReport>, ForgeError> {
    let target = spec.target().identity(&ctx.host).map_err(|error| {
        ForgeError::validation(schema_err(), "hook_target_invalid", error.to_string(), None)
    })?;
    let _scope = crate::identity::enter_scope(Some((target, Operation::ApiWrite)));
    match spec {
        BatchSpec::Change(spec) => {
            apply(runner, ctx, spec, dry_run).map(|row| vec![BatchReport::Change(row)])
        }
        BatchSpec::Delivery(spec) => super::redeliver::apply(runner, ctx, spec, dry_run)
            .map(|rows| rows.into_iter().map(BatchReport::Delivery).collect()),
    }
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
            rows.push(spec.skipped());
            continue;
        }
        match run_row(&runner, ctx, spec, global.dry_run) {
            Ok(reports) => rows.extend(reports),
            Err(error) => {
                aborted = error.kind() == "hook_readback_mismatch";
                rows.push(spec.failed(&error));
            }
        }
    }
    emit(kind, "batch", global.dry_run, rows, format)
}

pub(super) fn emit(
    kind: Kind,
    action: &str,
    dry_run: bool,
    rows: Vec<BatchReport>,
    format: OutputFormat,
) -> Result<i32, ForgeError> {
    let failed = rows
        .iter()
        .filter(|row| matches!(row.status(), "failed" | "skipped"))
        .count();
    let code = emit_success(
        schema_version_for(BINARY, &kind.op(action), 1),
        BatchPayload {
            provider: Provider::GitHub.as_str(),
            dry_run,
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
