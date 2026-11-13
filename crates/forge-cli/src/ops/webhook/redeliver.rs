//! Delivery selection and one attempt per selected id. GitHub REST redelivery
//! is asynchronous (202): correlate a new attempt by GUID, never report the
//! original failed delivery as the new attempt. No polling or POST retries.
use std::collections::HashSet;

use jiff::Timestamp;
use nils_common::cli_contract::OutputFormat;
use serde::{Deserialize, Serialize};

use super::engine::{Kind, RowError, api_call, invalid_response, run_call, schema_err};
use super::target::HookTarget;
use crate::backend::BackendRunner;
use crate::cli::{GlobalFlags, HookDeliveryStatus, HookRedeliverArgs};
use crate::error::ForgeError;
use crate::provider::ProviderContext;
use crate::rate_limit::default_runner;

#[derive(Debug, Clone)]
pub(super) struct Selection {
    status: HookDeliveryStatus,
    since: Timestamp,
    until: Timestamp,
}

impl Selection {
    pub(super) fn new(
        status: HookDeliveryStatus,
        since: &str,
        until: &str,
    ) -> Result<Self, ForgeError> {
        let invalid = || {
            ForgeError::validation(
                schema_err(),
                "hook_window_invalid",
                "since and until must be RFC 3339 timestamps with since before until",
                None,
            )
        };
        let since = since.parse::<Timestamp>().map_err(|_| invalid())?;
        let until = until.parse::<Timestamp>().map_err(|_| invalid())?;
        if since >= until {
            return Err(invalid());
        }
        Ok(Self {
            status,
            since,
            until,
        })
    }

    fn contains(&self, delivery: &Delivery) -> Result<bool, ForgeError> {
        let at = delivery
            .delivered_at
            .parse::<Timestamp>()
            .map_err(|_| invalid_response("delivery timestamp"))?;
        let selected = match self.status {
            HookDeliveryStatus::All => true,
            HookDeliveryStatus::Success => outcome(delivery.status_code) == "success",
            HookDeliveryStatus::Failed => outcome(delivery.status_code) == "failed",
        };
        Ok(selected && at >= self.since && at < self.until)
    }
}

#[derive(Debug, Clone)]
pub(super) struct RedeliverSpec {
    pub target: HookTarget,
    pub hook: u64,
    pub selection: Selection,
}

impl RedeliverSpec {
    pub(super) fn check(&self) -> Result<(), ForgeError> {
        if !self.target.valid() || self.hook == 0 {
            return Err(ForgeError::validation(
                schema_err(),
                "hook_target_invalid",
                "a valid target and positive hook id are required",
                None,
            ));
        }
        Ok(())
    }
}

/// Only allowlisted metadata enters output. In particular, the provider's
/// free-form status, request, response, headers and payloads are discarded.
#[derive(Deserialize)]
struct Delivery {
    id: u64,
    guid: String,
    redelivery: bool,
    delivered_at: String,
    status_code: Option<u16>,
}

#[derive(Deserialize)]
struct Observed {
    id: u64,
    status_code: Option<u16>,
}

fn outcome(code: Option<u16>) -> &'static str {
    match code {
        Some(200..=399) => "success",
        Some(_) => "failed",
        None => "pending",
    }
}

#[derive(Serialize)]
pub(super) struct DeliveryReport {
    pub target: String,
    pub hook: u64,
    pub action: &'static str,
    pub delivery: Option<u64>,
    pub status: &'static str,
    pub observed_delivery: Option<u64>,
    pub delivery_status: &'static str,
    pub status_code: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RowError>,
}

impl DeliveryReport {
    pub(super) fn failed(spec: &RedeliverSpec, delivery: Option<u64>, error: &ForgeError) -> Self {
        Self {
            target: spec.target.name().to_string(),
            hook: spec.hook,
            action: "redeliver",
            delivery,
            status: "failed",
            observed_delivery: None,
            delivery_status: "unknown",
            status_code: None,
            error: Some(RowError {
                kind: error.kind(),
                message: if error.kind() == "org_hook_scope_missing" {
                    "the token lacks the admin:org_hook scope"
                } else {
                    "redelivery operation failed; provider details omitted"
                }
                .to_string(),
                detail: None,
            }),
        }
    }

    pub(super) fn skipped(spec: &RedeliverSpec) -> Self {
        let error = ForgeError::runtime_failure(
            schema_err(),
            "batch_aborted",
            "not attempted after an earlier hook write/read-back mismatch",
            None,
        );
        let mut row = Self::failed(spec, None, &error);
        row.status = "skipped";
        row
    }

    pub(super) fn line(&self) -> String {
        let delivery = self
            .delivery
            .map(|id| id.to_string())
            .unwrap_or_else(|| "-".to_string());
        let code = self
            .status_code
            .map(|code| code.to_string())
            .unwrap_or_else(|| "-".to_string());
        format!(
            "{} hook {} delivery {}: {} {} {}{}",
            self.target,
            self.hook,
            delivery,
            self.status,
            self.delivery_status,
            code,
            self.error
                .as_ref()
                .map(|error| format!(" {}: {}", error.kind, error.message))
                .unwrap_or_default()
        )
    }
}

fn list<R: BackendRunner>(
    runner: &R,
    ctx: &ProviderContext,
    spec: &RedeliverSpec,
) -> Result<Vec<Delivery>, ForgeError> {
    // GitHub uses Link-header cursors, not numbered pages. gh follows those
    // links; slurp retains page boundaries as one JSON array of page arrays.
    let path = spec
        .target
        .path(&format!("/{}/deliveries?per_page=100", spec.hook));
    let output = run_call(runner, &api_call(ctx, path, &["--paginate", "--slurp"]))?;
    let pages: Vec<Vec<Delivery>> = serde_json::from_str(output.stdout.trim())
        .map_err(|_| invalid_response("delivery list"))?;
    let deliveries: Vec<_> = pages.into_iter().flatten().collect();
    if deliveries
        .iter()
        .any(|delivery| delivery.id == 0 || delivery.guid.is_empty())
    {
        return Err(invalid_response("delivery list"));
    }
    Ok(deliveries)
}

fn attempt<R: BackendRunner>(
    runner: &R,
    ctx: &ProviderContext,
    spec: &RedeliverSpec,
    delivery: &Delivery,
    known: &mut HashSet<u64>,
    ambiguous: bool,
) -> Result<Option<Observed>, ForgeError> {
    let path = spec.target.path(&format!(
        "/{}/deliveries/{}/attempts",
        spec.hook, delivery.id
    ));
    run_call(runner, &api_call(ctx, path, &["--method", "POST"]))
        .map_err(|error| safe_error(error, "hook_redelivery_failed"))?;
    let after = list(runner, ctx, spec)?;
    let mut candidates = after
        .iter()
        .filter(|row| row.redelivery && row.guid == delivery.guid && !known.contains(&row.id));
    let new = candidates.next().filter(|_| candidates.next().is_none());
    // A later selected id can have the same GUID. Once observed, an attempt
    // must not be attributed to another POST, even if that next one is pending.
    known.extend(after.iter().map(|row| row.id));
    // The API returns no attempt id with its 202. If an earlier POST for this
    // GUID is unresolved, a newly visible attempt could belong to that POST.
    if ambiguous {
        return Ok(None);
    }
    let Some(new) = new else {
        return Ok(None);
    };
    let path = spec
        .target
        .path(&format!("/{}/deliveries/{}", spec.hook, new.id));
    let output = run_call(runner, &api_call(ctx, path, &[]))?;
    let observed: Observed = serde_json::from_str(output.stdout.trim())
        .map_err(|_| invalid_response("delivery read-back"))?;
    if observed.id != new.id {
        return Err(invalid_response("delivery read-back"));
    }
    Ok(Some(observed))
}

fn safe_error(error: ForgeError, kind: &'static str) -> ForgeError {
    if matches!(
        error.kind(),
        "org_hook_scope_missing" | "hook_redelivery_failed"
    ) {
        return error;
    }
    ForgeError::runtime_failure(
        schema_err(),
        kind,
        "webhook delivery operation failed; provider details omitted",
        None,
    )
}

pub(super) fn apply<R: BackendRunner>(
    runner: &R,
    ctx: &ProviderContext,
    spec: &RedeliverSpec,
    dry_run: bool,
) -> Result<Vec<DeliveryReport>, ForgeError> {
    let deliveries = list(runner, ctx, spec)?;
    let mut known: HashSet<_> = deliveries.iter().map(|row| row.id).collect();
    let mut seen = HashSet::new();
    let mut unresolved = HashSet::new();
    // Validate the complete selection before the first POST. Freeze it so a
    // read-back cannot add redeliveries to the work queue.
    let selected: Vec<_> = deliveries
        .iter()
        .map(|row| spec.selection.contains(row))
        .collect::<Result<Vec<_>, _>>()?;
    let mut rows = Vec::new();
    for (delivery, selected) in deliveries.iter().zip(selected) {
        if !selected || !seen.insert(delivery.id) {
            continue;
        }
        let mut row = DeliveryReport {
            target: spec.target.name().to_string(),
            hook: spec.hook,
            action: "redeliver",
            delivery: Some(delivery.id),
            status: "would_redeliver",
            observed_delivery: None,
            delivery_status: outcome(delivery.status_code),
            status_code: delivery.status_code,
            error: None,
        };
        if !dry_run {
            match attempt(
                runner,
                ctx,
                spec,
                delivery,
                &mut known,
                unresolved.contains(&delivery.guid),
            ) {
                Ok(observed) => {
                    if observed.is_none() {
                        unresolved.insert(delivery.guid.clone());
                    }
                    row.status = "redelivered";
                    row.observed_delivery = observed.as_ref().map(|d| d.id);
                    row.status_code = observed.and_then(|d| d.status_code);
                    row.delivery_status = outcome(row.status_code);
                    if row.delivery_status == "failed" {
                        row.status = "failed";
                    }
                }
                Err(error) => {
                    // A failed POST or read-back may leave an unseen attempt.
                    unresolved.insert(delivery.guid.clone());
                    row = DeliveryReport::failed(
                        spec,
                        Some(delivery.id),
                        &safe_error(error, "hook_redelivery_readback_failed"),
                    )
                }
            }
        }
        rows.push(row);
    }
    Ok(rows)
}

pub(super) fn run(
    global: &GlobalFlags,
    ctx: &ProviderContext,
    kind: Kind,
    target: Option<HookTarget>,
    args: HookRedeliverArgs,
    format: OutputFormat,
) -> Result<i32, ForgeError> {
    let target = target.ok_or_else(|| {
        ForgeError::validation(
            schema_err(),
            "target_required",
            "a target is required",
            None,
        )
    })?;
    let spec = RedeliverSpec {
        target,
        hook: args.hook,
        selection: Selection::new(args.status, &args.since, &args.until)?,
    };
    spec.check()?;
    let rows = match apply(&default_runner(), ctx, &spec, global.dry_run) {
        Ok(rows) => rows,
        Err(error) => vec![DeliveryReport::failed(
            &spec,
            None,
            &safe_error(error, "hook_deliveries_failed"),
        )],
    };
    super::batch::emit(
        kind,
        "redeliver",
        global.dry_run,
        rows.into_iter()
            .map(super::batch::BatchReport::Delivery)
            .collect(),
        format,
    )
}
