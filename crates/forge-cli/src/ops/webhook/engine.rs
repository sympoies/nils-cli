//! Target-independent webhook reads, compare-and-swap writes and reporting.
use std::ffi::OsString;
use std::path::PathBuf;

use nils_common::cli_contract::{OutputFormat, schema_version_for};
use serde::Serialize;
use serde_json::Value;

use super::target::HookTarget;
use crate::backend::{BackendCall, BackendProgram, BackendRunner, BackendSuccess};
use crate::cli::{BINARY, GlobalFlags};
use crate::envelope::emit_success;
use crate::error::ForgeError;
use crate::provider::{Provider, ProviderContext};
use crate::rate_limit::default_runner;

/// Printed in place of a configured secret; the value itself is never kept.
pub(super) const REDACTED: &str = "[redacted]";

/// Which command family is running (names schemas).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Repo,
    Org,
}

impl Kind {
    pub(super) fn op(self, action: &str) -> String {
        let family = if self == Self::Repo { "repo" } else { "org" };
        format!("{family}.hook.{action}")
    }
}

pub(crate) enum Action {
    List,
    SetUrl { hook: u64, from: String, to: String },
    SetActive { hook: u64, to: bool },
    Batch(PathBuf),
}

pub(super) fn schema_err() -> String {
    schema_version_for(BINARY, "error", 1)
}

fn refuse(kind: &'static str, message: &'static str) -> ForgeError {
    ForgeError::validation(schema_err(), kind, message, None)
}

/// One requested change; `from` is what the hook must currently have.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Change {
    Url { from: String, to: String },
    Active { from: bool, to: bool },
}

impl Change {
    pub(super) fn action(&self) -> &'static str {
        match self {
            Self::Url { .. } => "set-url",
            Self::Active { .. } => "set-active",
        }
    }
    fn expected_value(&self) -> Value {
        match self {
            Self::Url { from, .. } => Value::String(from.clone()),
            Self::Active { from, .. } => Value::Bool(*from),
        }
    }
    fn wanted_value(&self) -> Value {
        match self {
            Self::Url { to, .. } => Value::String(to.clone()),
            Self::Active { to, .. } => Value::Bool(*to),
        }
    }
}

/// One requested change to one hook.
#[derive(Debug, Clone)]
pub(super) struct HookSpec {
    pub target: HookTarget,
    pub hook: u64,
    pub change: Change,
}

/// Why a [`HookSpec`] is refused before any provider call.
pub(super) struct SpecIssue {
    pub kind: &'static str,
    pub message: &'static str,
}

impl HookSpec {
    pub(super) fn check(&self) -> Result<(), SpecIssue> {
        let issue = |kind, message| Err(SpecIssue { kind, message });
        if !self.target.valid() {
            return issue(
                "target_invalid",
                "target must be OWNER/NAME or an organization login",
            );
        }
        if self.hook == 0 {
            return issue("hook_invalid", "hook id must be a positive integer");
        }
        match &self.change {
            Change::Url { from, to } => {
                if !valid_url(from) || !valid_url(to) {
                    return issue(
                        "hook_url_invalid",
                        "from and to must be absolute http(s) URLs without whitespace",
                    );
                }
                if from == to {
                    return issue("hook_url_unchanged", "from and to are identical");
                }
            }
            Change::Active { from, to } => {
                if from == to {
                    return issue("hook_active_unchanged", "from and to are identical");
                }
            }
        }
        Ok(())
    }

    fn checked(self) -> Result<Self, ForgeError> {
        self.check()
            .map_err(|issue| refuse(issue.kind, issue.message))?;
        Ok(self)
    }
}

pub(super) fn valid_url(value: &str) -> bool {
    value.len() <= 2048
        && !value.chars().any(|c| c.is_control() || c.is_whitespace())
        && url::Url::parse(value)
            .is_ok_and(|url| matches!(url.scheme(), "http" | "https") && url.host_str().is_some())
}

/// Outcome of one row (and the payload of a single change).
#[derive(Debug, Clone, Serialize)]
pub(super) struct RowReport {
    pub target: String,
    pub hook: u64,
    pub action: &'static str,
    /// `updated`, `would_update` (dry run), `already_set`, `failed`, or
    /// `skipped` (a batch stopped after a read-back mismatch).
    pub status: &'static str,
    pub from: Value,
    pub to: Value,
    /// The URL or active flag as last observed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observed: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RowError>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct RowError {
    pub kind: &'static str,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl RowReport {
    fn new(spec: &HookSpec, status: &'static str, observed: Option<Value>) -> Self {
        Self {
            target: spec.target.name().to_string(),
            hook: spec.hook,
            action: spec.change.action(),
            status,
            from: spec.change.expected_value(),
            to: spec.change.wanted_value(),
            observed,
            error: None,
        }
    }

    pub(super) fn failed(spec: &HookSpec, error: &ForgeError) -> Self {
        let mut report = Self::new(spec, "failed", None);
        report.error = Some(RowError {
            kind: error.kind(),
            message: error.message().to_string(),
            detail: error.detail().map(str::to_string),
        });
        report
    }

    pub(super) fn skipped(spec: &HookSpec) -> Self {
        let mut report = Self::new(spec, "skipped", None);
        report.error = Some(RowError {
            kind: "batch_aborted",
            message: "not attempted: an earlier row did not read back as written".to_string(),
            detail: None,
        });
        report
    }

    pub(super) fn line(&self) -> String {
        let head = format!("{} hook {}: {}", self.target, self.hook, self.action);
        match (&self.error, &self.observed) {
            (Some(error), _) => format!("{head} {} {}: {}", self.status, error.kind, error.message),
            (None, Some(Value::String(text))) => format!("{head} {} {text}", self.status),
            (None, Some(other)) => format!("{head} {} {other}", self.status),
            (None, None) => format!("{head} {}", self.status),
        }
    }
}

/// The comparable state of a hook; the secret is reduced to whether one is set.
#[derive(Debug, Clone, PartialEq, Eq)]
struct HookState {
    active: bool,
    events: Vec<String>,
    url: String,
    content_type: Option<String>,
    insecure_ssl: Option<String>,
    has_secret: bool,
}

pub(super) fn text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

pub(super) fn has_secret(config: &Value) -> bool {
    config
        .get("secret")
        .and_then(Value::as_str)
        .is_some_and(|secret| !secret.is_empty())
}

fn events(value: &Value) -> Vec<String> {
    value
        .get("events")
        .and_then(Value::as_array)
        .map(|events| {
            events
                .iter()
                .filter_map(|event| event.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

pub(super) fn invalid_response(what: &str) -> ForgeError {
    ForgeError::software(
        schema_err(),
        format!("provider returned an unexpected {what}"),
        None,
    )
}

fn state(value: &Value) -> Result<HookState, ForgeError> {
    let config = value.get("config").unwrap_or(&Value::Null);
    Ok(HookState {
        active: value
            .get("active")
            .and_then(Value::as_bool)
            .ok_or_else(|| invalid_response("webhook"))?,
        events: events(value),
        url: config
            .get("url")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid_response("webhook"))?
            .to_string(),
        content_type: config.get("content_type").and_then(text),
        insecure_ssl: config.get("insecure_ssl").and_then(text),
        has_secret: has_secret(config),
    })
}

pub(super) fn api_call(ctx: &ProviderContext, path: String, write: &[&str]) -> BackendCall {
    let mut argv: Vec<OsString> = vec!["api".into()];
    ctx.push_github_api_hostname(&mut argv);
    argv.push(path.into());
    argv.extend(write.iter().map(OsString::from));
    BackendCall::new(BackendProgram::Gh, argv)
}

/// Run a call, naming the missing token scope when GitHub refuses an
/// organization webhook call for lack of it (`gh` prints the needed scope).
pub(super) fn run_call<R: BackendRunner>(
    runner: &R,
    call: &BackendCall,
) -> Result<BackendSuccess, ForgeError> {
    runner.run(call).map_err(|error| {
        let mentions = |text: &str| text.contains("admin:org_hook");
        if mentions(error.message()) || error.detail().is_some_and(mentions) {
            ForgeError::validation(
                schema_err(),
                "org_hook_scope_missing",
                "the token lacks the admin:org_hook scope (organization webhooks need an \
                 organization owner token with it)",
                None,
            )
        } else {
            error
        }
    })
}

fn read<R: BackendRunner>(
    runner: &R,
    ctx: &ProviderContext,
    spec: &HookSpec,
) -> Result<HookState, ForgeError> {
    let path = spec.target.path(&format!("/{}", spec.hook));
    let output = run_call(runner, &api_call(ctx, path, &[]))?;
    let value: Value =
        serde_json::from_str(output.stdout.trim()).map_err(|_| invalid_response("webhook"))?;
    state(&value)
}

/// Change one hook: compare-and-swap, write only the changed field, read back.
pub(super) fn apply<R: BackendRunner>(
    runner: &R,
    ctx: &ProviderContext,
    spec: &HookSpec,
    dry_run: bool,
) -> Result<RowReport, ForgeError> {
    let before = read(runner, ctx, spec)?;
    let (what, observed, expected, wanted) = match &spec.change {
        Change::Url { from, to } => (
            "URL",
            Value::String(before.url.clone()),
            Value::String(from.clone()),
            Value::String(to.clone()),
        ),
        Change::Active { from, to } => (
            "active flag",
            Value::Bool(before.active),
            Value::Bool(*from),
            Value::Bool(*to),
        ),
    };
    if observed == wanted {
        return Ok(RowReport::new(spec, "already_set", Some(observed)));
    }
    if observed != expected {
        return Err(ForgeError::validation(
            schema_err(),
            "hook_value_mismatch",
            format!("the hook's current {what} is not the expected one; nothing was changed"),
            Some(format!("current {what}: {observed}")),
        ));
    }
    if dry_run {
        return Ok(RowReport::new(spec, "would_update", Some(observed)));
    }
    let hook_path = spec.target.path(&format!("/{}", spec.hook));
    let write = match &spec.change {
        Change::Url { to, .. } => api_call(
            ctx,
            format!("{hook_path}/config"),
            &["--method", "PATCH", "-f", &format!("url={to}")],
        ),
        Change::Active { to, .. } => api_call(
            ctx,
            hook_path,
            &["--method", "PATCH", "-F", &format!("active={to}")],
        ),
    };
    run_call(runner, &write)?;
    let after = read(runner, ctx, spec)?;
    let mut expected_after = before;
    match &spec.change {
        Change::Url { to, .. } => expected_after.url = to.clone(),
        Change::Active { to, .. } => expected_after.active = *to,
    }
    if after != expected_after {
        return Err(ForgeError::runtime_failure(
            schema_err(),
            "hook_readback_mismatch",
            "the hook read back differently after the write (changed field, other settings, or \
             secret presence); the change may have been applied",
            None,
        ));
    }
    Ok(RowReport::new(spec, "updated", Some(wanted)))
}

#[derive(Serialize)]
pub(super) struct HookView {
    pub id: u64,
    pub active: bool,
    pub events: Vec<String>,
    pub content_type: Option<String>,
    pub url: Option<String>,
    pub secret: Option<&'static str>,
}

#[derive(Serialize)]
struct ListPayload {
    provider: &'static str,
    target: String,
    hooks: Vec<HookView>,
}

pub(super) fn hook_view(value: &Value) -> Result<HookView, ForgeError> {
    let config = value.get("config").unwrap_or(&Value::Null);
    Ok(HookView {
        id: value
            .get("id")
            .and_then(Value::as_u64)
            .ok_or_else(|| invalid_response("webhook"))?,
        active: value
            .get("active")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        events: events(value),
        content_type: config.get("content_type").and_then(text),
        url: config.get("url").and_then(text),
        secret: has_secret(config).then_some(REDACTED),
    })
}

pub(super) fn list_hooks<R: BackendRunner>(
    runner: &R,
    ctx: &ProviderContext,
    target: &HookTarget,
) -> Result<Vec<HookView>, ForgeError> {
    // GitHub allows at most 20 webhooks per repository or organization, so
    // one page of 100 is complete.
    let output = run_call(runner, &api_call(ctx, target.path("?per_page=100"), &[]))?;
    let value: Value =
        serde_json::from_str(output.stdout.trim()).map_err(|_| invalid_response("webhook list"))?;
    value
        .as_array()
        .ok_or_else(|| invalid_response("webhook list"))?
        .iter()
        .map(hook_view)
        .collect()
}

#[derive(Serialize)]
struct SinglePayload {
    provider: &'static str,
    dry_run: bool,
    #[serde(flatten)]
    row: RowReport,
}

/// Run one command. `target` is `None` only for a batch, whose rows name
/// their own targets.
pub(crate) fn execute(
    global: &GlobalFlags,
    ctx: &ProviderContext,
    kind: Kind,
    target: Option<HookTarget>,
    action: Action,
    format: OutputFormat,
) -> Result<i32, ForgeError> {
    let single = |change: Change, hook: u64| {
        let target = target
            .clone()
            .ok_or_else(|| refuse("target_required", "a target is required"))?;
        HookSpec {
            target,
            hook,
            change,
        }
        .checked()
    };
    let (op, spec) = match action {
        Action::Batch(file) => return super::batch::run(global, ctx, kind, &file, format),
        Action::List => {
            let target = target.ok_or_else(|| refuse("target_required", "a target is required"))?;
            if !target.valid() {
                return Err(refuse(
                    "target_invalid",
                    "target must be OWNER/NAME or an organization login",
                ));
            }
            let hooks = list_hooks(&default_runner(), ctx, &target)?;
            return Ok(emit_success(
                schema_version_for(BINARY, &kind.op("list"), 1),
                ListPayload {
                    provider: Provider::GitHub.as_str(),
                    target: target.name().to_string(),
                    hooks,
                },
                format,
                |payload| {
                    for hook in &payload.hooks {
                        // Literal markers only: the secret value is never in scope here.
                        let marker = if hook.secret.is_some() {
                            "secret=[redacted]"
                        } else {
                            "no-secret"
                        };
                        println!(
                            "{}\t{}\t{}\t{}\t{}\t{marker}",
                            hook.id,
                            if hook.active { "active" } else { "inactive" },
                            hook.events.join(","),
                            hook.content_type.as_deref().unwrap_or("-"),
                            hook.url.as_deref().unwrap_or("-"),
                        );
                    }
                },
            ));
        }
        Action::SetUrl { hook, from, to } => {
            (kind.op("set-url"), single(Change::Url { from, to }, hook)?)
        }
        Action::SetActive { hook, to } => (
            kind.op("set-active"),
            single(Change::Active { from: !to, to }, hook)?,
        ),
    };
    let row = apply(&default_runner(), ctx, &spec, global.dry_run)?;
    Ok(emit_success(
        schema_version_for(BINARY, &op, 1),
        SinglePayload {
            provider: Provider::GitHub.as_str(),
            dry_run: global.dry_run,
            row,
        },
        format,
        |payload| println!("{}", payload.row.line()),
    ))
}
