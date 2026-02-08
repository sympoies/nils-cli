use std::time::Instant;

use regex::{Regex, RegexBuilder};
use serde_json::Value;

use crate::backend::applescript;
use crate::backend::process::ProcessRunner;
use crate::backend::AxBackendAdapter;
use crate::cli::{AxSelectorArgs, AxTargetArgs};
use crate::error::CliError;
use crate::model::{
    AxAttrGetRequest, AxGateCheckResult, AxGateResult, AxListRequest, AxMatchStrategy, AxNode,
    AxPostconditionCheckResult, AxPostconditionResult, AxSelector, AxSelectorExplain,
    AxSelectorExplainStage, AxTarget,
};
use crate::targets::{self, TargetSelector};
use crate::wait;

#[derive(Debug, Clone, Default)]
pub struct AxSelectorInput {
    pub node_id: Option<String>,
    pub role: Option<String>,
    pub title_contains: Option<String>,
    pub identifier_contains: Option<String>,
    pub value_contains: Option<String>,
    pub subrole: Option<String>,
    pub focused: Option<bool>,
    pub enabled: Option<bool>,
    pub nth: Option<u32>,
    pub match_strategy: AxMatchStrategy,
    pub explain: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectorSelectionStatus {
    Selected,
    NoMatches,
    NthOutOfRange,
    Ambiguous,
}

#[derive(Debug, Clone)]
pub struct SelectorEvaluation {
    pub matched_count: usize,
    pub selected_node_id: Option<String>,
    pub selection_status: SelectorSelectionStatus,
    pub explain: Option<AxSelectorExplain>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct AxActionGateOptions {
    pub app_active: bool,
    pub window_present: bool,
    pub ax_present: bool,
    pub ax_unique: bool,
    pub timeout_ms: u64,
    pub poll_ms: u64,
}

impl AxActionGateOptions {
    pub fn any_enabled(self) -> bool {
        self.app_active || self.window_present || self.ax_present || self.ax_unique
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum AxPostconditionCheck {
    Focused(bool),
    AttributeValue { name: String, expected: Value },
}

impl AxPostconditionCheck {
    fn name(&self) -> String {
        match self {
            Self::Focused(expected) => format!("focused={expected}"),
            Self::AttributeValue { name, .. } => format!("attribute={name}"),
        }
    }

    fn expected_value(&self) -> Value {
        match self {
            Self::Focused(expected) => Value::Bool(*expected),
            Self::AttributeValue { expected, .. } => expected.clone(),
        }
    }

    fn attribute_name(&self) -> Option<String> {
        match self {
            Self::Focused(_) => None,
            Self::AttributeValue { name, .. } => Some(name.clone()),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct AxPostconditionOptions {
    pub checks: Vec<AxPostconditionCheck>,
    pub timeout_ms: u64,
    pub poll_ms: u64,
}

impl AxPostconditionOptions {
    pub fn any_enabled(&self) -> bool {
        !self.checks.is_empty()
    }
}

pub fn build_target(
    session_id: Option<String>,
    app: Option<String>,
    bundle_id: Option<String>,
    window_title_contains: Option<String>,
) -> Result<AxTarget, CliError> {
    let mut target_count = 0;
    if session_id.is_some() {
        target_count += 1;
    }
    if app.is_some() {
        target_count += 1;
    }
    if bundle_id.is_some() {
        target_count += 1;
    }

    if target_count > 1 {
        return Err(CliError::usage(
            "--session-id cannot be combined with --app/--bundle-id",
        ));
    }

    Ok(AxTarget {
        session_id,
        app,
        bundle_id,
        window_title_contains,
    })
}

pub fn build_target_from_args(args: &AxTargetArgs) -> Result<AxTarget, CliError> {
    build_target(
        args.session_id.clone(),
        args.app.clone(),
        args.bundle_id.clone(),
        args.window_title_contains.clone(),
    )
}

pub fn selector_input_from_args(args: &AxSelectorArgs) -> AxSelectorInput {
    AxSelectorInput {
        node_id: args.node_id.clone(),
        role: args.filters.role.clone(),
        title_contains: args.filters.title_contains.clone(),
        identifier_contains: args.filters.identifier_contains.clone(),
        value_contains: args.filters.value_contains.clone(),
        subrole: args.filters.subrole.clone(),
        focused: args.filters.focused,
        enabled: args.filters.enabled,
        nth: args.nth,
        match_strategy: args.match_strategy,
        explain: args.selector_explain,
    }
}

pub fn build_selector(input: AxSelectorInput) -> Result<AxSelector, CliError> {
    if input.nth == Some(0) {
        return Err(CliError::usage("--nth must be at least 1"));
    }

    let has_primary_filters = input.role.is_some()
        || input.title_contains.is_some()
        || input.identifier_contains.is_some()
        || input.value_contains.is_some()
        || input.subrole.is_some()
        || input.focused.is_some()
        || input.enabled.is_some();
    let has_non_node_filters = has_primary_filters || input.nth.is_some();

    if input.node_id.is_some() && has_non_node_filters {
        return Err(CliError::usage(
            "--node-id cannot be combined with role/title/identifier/value/subrole/focused/enabled/nth selectors",
        ));
    }

    if input.node_id.is_none() && !has_primary_filters {
        if input.nth.is_some() {
            return Err(CliError::usage(
                "--nth requires at least one selector filter when --node-id is not set",
            ));
        }
        return Err(CliError::usage(
            "provide --node-id or at least one selector filter (--role/--title-contains/--identifier-contains/--value-contains/--subrole/--focused/--enabled)",
        ));
    }

    if input.match_strategy == AxMatchStrategy::Regex {
        validate_selector_regex("--title-contains", input.title_contains.as_deref())?;
        validate_selector_regex(
            "--identifier-contains",
            input.identifier_contains.as_deref(),
        )?;
        validate_selector_regex("--value-contains", input.value_contains.as_deref())?;
    }

    Ok(AxSelector {
        node_id: input.node_id,
        role: input.role,
        title_contains: input.title_contains,
        identifier_contains: input.identifier_contains,
        value_contains: input.value_contains,
        subrole: input.subrole,
        focused: input.focused,
        enabled: input.enabled,
        nth: input.nth.map(|value| value as usize),
        match_strategy: input.match_strategy,
        explain: input.explain,
    })
}

pub fn build_selector_from_args(args: &AxSelectorArgs) -> Result<AxSelector, CliError> {
    build_selector(selector_input_from_args(args))
}

pub fn selector_selection_error(
    operation: &str,
    status: SelectorSelectionStatus,
) -> Option<CliError> {
    let error = match status {
        SelectorSelectionStatus::Selected => return None,
        SelectorSelectionStatus::NoMatches => {
            CliError::runtime("selector returned zero AX matches")
        }
        SelectorSelectionStatus::NthOutOfRange => CliError::runtime("selector nth is out of range"),
        SelectorSelectionStatus::Ambiguous => {
            CliError::runtime("selector is ambiguous; add --nth or narrow selector filters")
        }
    };

    Some(
        error
            .with_operation(operation)
            .with_hint("Adjust AX selector filters so exactly one element is targeted."),
    )
}

pub fn evaluate_selector_against_backend(
    runner: &dyn ProcessRunner,
    backend: &dyn AxBackendAdapter,
    target: &AxTarget,
    selector: &AxSelector,
    timeout_ms: u64,
) -> Result<SelectorEvaluation, CliError> {
    let list_result = backend.list(
        runner,
        &AxListRequest {
            target: target.clone(),
            ..AxListRequest::default()
        },
        timeout_ms.max(1),
    )?;
    evaluate_selector_against_nodes(&list_result.nodes, selector)
}

pub fn resolve_selector_node_against_backend(
    runner: &dyn ProcessRunner,
    backend: &dyn AxBackendAdapter,
    target: &AxTarget,
    selector: &AxSelector,
    timeout_ms: u64,
) -> Result<(SelectorEvaluation, AxNode), CliError> {
    let list_result = backend.list(
        runner,
        &AxListRequest {
            target: target.clone(),
            ..AxListRequest::default()
        },
        timeout_ms.max(1),
    )?;
    let evaluation = evaluate_selector_against_nodes(&list_result.nodes, selector)?;
    if let Some(error) = selector_selection_error("selector.resolve", evaluation.selection_status) {
        return Err(error);
    }

    let selected_node_id = evaluation
        .selected_node_id
        .as_ref()
        .ok_or_else(|| CliError::runtime("selector evaluation returned no node"))?;
    let node = list_result
        .nodes
        .into_iter()
        .find(|candidate| candidate.node_id == *selected_node_id)
        .ok_or_else(|| {
            CliError::runtime(format!(
                "selector resolved to `{selected_node_id}` but node details were unavailable"
            ))
        })?;

    Ok((evaluation, node))
}

pub fn selector_args_requested(args: &AxSelectorArgs) -> bool {
    args.node_id.is_some()
        || args.filters.role.is_some()
        || args.filters.title_contains.is_some()
        || args.filters.identifier_contains.is_some()
        || args.filters.value_contains.is_some()
        || args.filters.subrole.is_some()
        || args.filters.focused.is_some()
        || args.filters.enabled.is_some()
        || args.nth.is_some()
}

pub fn parse_postcondition_expected_value(raw: &str) -> Value {
    serde_json::from_str(raw).unwrap_or_else(|_| Value::String(raw.to_string()))
}

pub fn evaluate_selector_against_nodes(
    nodes: &[AxNode],
    selector: &AxSelector,
) -> Result<SelectorEvaluation, CliError> {
    let mut current = nodes.iter().collect::<Vec<_>>();
    let mut stage_results = Vec::new();

    if let Some(node_id) = selector.node_id.as_deref() {
        apply_stage("node_id", &mut current, &mut stage_results, |node| {
            node.node_id == node_id
        });
    } else {
        if let Some(role_filter) = selector.role.as_deref() {
            apply_stage("role", &mut current, &mut stage_results, |node| {
                node.role.eq_ignore_ascii_case(role_filter)
            });
        }

        if let Some(filter) = selector.title_contains.as_deref() {
            let matcher = build_text_matcher(filter, selector.match_strategy).map_err(|err| {
                err.with_hint(
                    "Use a valid pattern for --title-contains under --match-strategy regex.",
                )
            })?;
            apply_stage("title", &mut current, &mut stage_results, |node| {
                matcher.matches(node.title.as_deref().unwrap_or_default())
                    || matcher.matches(node.identifier.as_deref().unwrap_or_default())
            });
        }

        if let Some(filter) = selector.identifier_contains.as_deref() {
            let matcher = build_text_matcher(filter, selector.match_strategy).map_err(|err| {
                err.with_hint(
                    "Use a valid pattern for --identifier-contains under --match-strategy regex.",
                )
            })?;
            apply_stage("identifier", &mut current, &mut stage_results, |node| {
                matcher.matches(node.identifier.as_deref().unwrap_or_default())
            });
        }

        if let Some(filter) = selector.value_contains.as_deref() {
            let matcher = build_text_matcher(filter, selector.match_strategy).map_err(|err| {
                err.with_hint(
                    "Use a valid pattern for --value-contains under --match-strategy regex.",
                )
            })?;
            apply_stage("value", &mut current, &mut stage_results, |node| {
                matcher.matches(node.value_preview.as_deref().unwrap_or_default())
            });
        }

        if let Some(subrole_filter) = selector.subrole.as_deref() {
            apply_stage("subrole", &mut current, &mut stage_results, |node| {
                node.subrole
                    .as_deref()
                    .unwrap_or_default()
                    .eq_ignore_ascii_case(subrole_filter)
            });
        }

        if let Some(focused_filter) = selector.focused {
            apply_stage("focused", &mut current, &mut stage_results, |node| {
                node.focused == focused_filter
            });
        }

        if let Some(enabled_filter) = selector.enabled {
            apply_stage("enabled", &mut current, &mut stage_results, |node| {
                node.enabled == enabled_filter
            });
        }
    }

    let matched_count = current.len();
    let mut selected_node_id = None;
    let selection_status = if selector.node_id.is_some() {
        if matched_count == 0 {
            SelectorSelectionStatus::NoMatches
        } else {
            selected_node_id = current.first().map(|node| node.node_id.clone());
            SelectorSelectionStatus::Selected
        }
    } else if let Some(nth) = selector.nth {
        let before_count = matched_count;
        if nth >= 1 && nth <= matched_count {
            selected_node_id = current.get(nth - 1).map(|node| node.node_id.clone());
            stage_results.push(AxSelectorExplainStage {
                stage: "nth".to_string(),
                before_count,
                after_count: 1,
            });
            SelectorSelectionStatus::Selected
        } else {
            stage_results.push(AxSelectorExplainStage {
                stage: "nth".to_string(),
                before_count,
                after_count: 0,
            });
            SelectorSelectionStatus::NthOutOfRange
        }
    } else if matched_count == 0 {
        SelectorSelectionStatus::NoMatches
    } else if matched_count == 1 {
        selected_node_id = current.first().map(|node| node.node_id.clone());
        SelectorSelectionStatus::Selected
    } else {
        SelectorSelectionStatus::Ambiguous
    };

    let explain = if selector.explain {
        Some(AxSelectorExplain {
            strategy: selector.match_strategy,
            total_candidates: nodes.len(),
            matched_count,
            selected_count: if selected_node_id.is_some() { 1 } else { 0 },
            stage_results,
            selected_node_id: selected_node_id.clone(),
        })
    } else {
        None
    };

    Ok(SelectorEvaluation {
        matched_count,
        selected_node_id,
        selection_status,
        explain,
    })
}

pub fn run_action_gates(
    operation: &str,
    runner: &dyn ProcessRunner,
    backend: &dyn AxBackendAdapter,
    target: &AxTarget,
    selector: &AxSelector,
    options: AxActionGateOptions,
    backend_timeout_ms: u64,
) -> Result<Option<AxGateResult>, CliError> {
    if !options.any_enabled() {
        return Ok(None);
    }

    let timeout_ms = options.timeout_ms.max(1);
    let poll_ms = options.poll_ms.max(1);
    let mut checks = Vec::new();

    if options.app_active {
        checks.push(run_gate_app_active(
            operation, runner, target, timeout_ms, poll_ms,
        )?);
    }
    if options.window_present {
        checks.push(run_gate_window_present(
            operation, target, timeout_ms, poll_ms,
        )?);
    }
    if options.ax_present {
        checks.push(run_gate_ax_selector(
            operation,
            "ax-present",
            runner,
            backend,
            target,
            selector,
            timeout_ms,
            poll_ms,
            backend_timeout_ms,
            |matched| matched >= 1,
        )?);
    }
    if options.ax_unique {
        checks.push(run_gate_ax_selector(
            operation,
            "ax-unique",
            runner,
            backend,
            target,
            selector,
            timeout_ms,
            poll_ms,
            backend_timeout_ms,
            |matched| matched == 1,
        )?);
    }

    Ok(Some(AxGateResult {
        timeout_ms,
        poll_ms,
        checks,
    }))
}

pub fn run_postconditions(
    operation: &str,
    runner: &dyn ProcessRunner,
    backend: &dyn AxBackendAdapter,
    target: &AxTarget,
    node_id: &str,
    options: &AxPostconditionOptions,
    backend_timeout_ms: u64,
) -> Result<Option<AxPostconditionResult>, CliError> {
    if !options.any_enabled() {
        return Ok(None);
    }

    let timeout_ms = options.timeout_ms.max(1);
    let poll_ms = options.poll_ms.max(1);
    let mut results = Vec::new();

    for check in &options.checks {
        let started = Instant::now();
        let mut observed = None;
        let outcome = wait::wait_until(
            &format!("{operation}.postcondition.{}", check.name()),
            timeout_ms,
            poll_ms,
            || {
                let (satisfied, current) = evaluate_postcondition_check(
                    runner,
                    backend,
                    target,
                    node_id,
                    check,
                    backend_timeout_ms,
                )?;
                observed = current;
                Ok(satisfied)
            },
        )
        .map_err(|error| {
            map_postcondition_error(operation, check, timeout_ms, observed.clone(), error)
        })?;

        results.push(AxPostconditionCheckResult {
            check: check.name(),
            terminal_status: "satisfied".to_string(),
            attempts: outcome.attempts,
            elapsed_ms: started.elapsed().as_millis() as u64,
            attribute: check.attribute_name(),
            expected: check.expected_value(),
            observed,
        });
    }

    Ok(Some(AxPostconditionResult {
        timeout_ms,
        poll_ms,
        checks: results,
    }))
}

fn run_gate_app_active(
    operation: &str,
    runner: &dyn ProcessRunner,
    target: &AxTarget,
    timeout_ms: u64,
    poll_ms: u64,
) -> Result<AxGateCheckResult, CliError> {
    let mut check: Box<dyn FnMut() -> Result<bool, CliError>> =
        if let Some(app) = target.app.as_deref() {
            let app = app.to_string();
            Box::new(move || {
                let probe_timeout = timeout_ms.max(2_000);
                applescript::frontmost_app_name(runner, probe_timeout)
                    .map(|frontmost| frontmost.eq_ignore_ascii_case(&app))
            })
        } else if let Some(bundle_id) = target.bundle_id.as_deref() {
            let bundle_id = bundle_id.to_string();
            Box::new(move || {
                let probe_timeout = timeout_ms.max(2_000);
                applescript::frontmost_bundle_id(runner, probe_timeout)
                    .map(|frontmost| frontmost.eq_ignore_ascii_case(&bundle_id))
            })
        } else {
            return Err(CliError::usage(
                "`--gate-app-active` requires target app context (--app or --bundle-id)",
            )
            .with_operation(format!("{operation}.gate.app-active"))
            .with_hint("Provide --app or --bundle-id when enabling app-active gating."));
        };

    let started = Instant::now();
    let outcome = wait::wait_until("gate.app-active", timeout_ms, poll_ms, &mut check)
        .map_err(|error| map_gate_error(operation, "app-active", timeout_ms, None, error))?;
    Ok(AxGateCheckResult {
        gate: "app-active".to_string(),
        terminal_status: "satisfied".to_string(),
        attempts: outcome.attempts,
        elapsed_ms: started.elapsed().as_millis() as u64,
        matched_count: None,
    })
}

fn run_gate_window_present(
    operation: &str,
    target: &AxTarget,
    timeout_ms: u64,
    poll_ms: u64,
) -> Result<AxGateCheckResult, CliError> {
    if target.session_id.is_some() && target.app.is_none() && target.bundle_id.is_none() {
        return Err(CliError::usage(
            "`--gate-window-present` cannot infer app/window from --session-id target alone",
        )
        .with_operation(format!("{operation}.gate.window-present"))
        .with_hint("Add --app or --bundle-id to run window-present gating."));
    }

    let window_name = target.window_title_contains.clone();
    let app = target.app.clone();
    let bundle_id = target.bundle_id.clone();

    let started = Instant::now();
    let outcome = wait::wait_until("gate.window-present", timeout_ms, poll_ms, || {
        if let Some(app) = app.as_deref() {
            return targets::window_present(&TargetSelector {
                window_id: None,
                active_window: false,
                app: Some(app.to_string()),
                window_name: window_name.clone(),
            });
        }

        if let Some(bundle_id) = bundle_id.as_deref() {
            if let Some(mapped_app) = targets::app_name_for_bundle_id(bundle_id)? {
                return targets::window_present(&TargetSelector {
                    window_id: None,
                    active_window: false,
                    app: Some(mapped_app),
                    window_name: window_name.clone(),
                });
            }
            return Ok(false);
        }

        Ok(false)
    })
    .map_err(|error| map_gate_error(operation, "window-present", timeout_ms, None, error))?;

    Ok(AxGateCheckResult {
        gate: "window-present".to_string(),
        terminal_status: "satisfied".to_string(),
        attempts: outcome.attempts,
        elapsed_ms: started.elapsed().as_millis() as u64,
        matched_count: None,
    })
}

#[allow(clippy::too_many_arguments)]
fn run_gate_ax_selector<F>(
    operation: &str,
    gate_name: &str,
    runner: &dyn ProcessRunner,
    backend: &dyn AxBackendAdapter,
    target: &AxTarget,
    selector: &AxSelector,
    timeout_ms: u64,
    poll_ms: u64,
    backend_timeout_ms: u64,
    predicate: F,
) -> Result<AxGateCheckResult, CliError>
where
    F: Fn(usize) -> bool,
{
    let mut last_matched_count = 0usize;
    let started = Instant::now();
    let outcome = wait::wait_until(&format!("gate.{gate_name}"), timeout_ms, poll_ms, || {
        let evaluation = evaluate_selector_against_backend(
            runner,
            backend,
            target,
            selector,
            backend_timeout_ms,
        )?;
        last_matched_count = evaluation.matched_count;
        Ok(predicate(evaluation.matched_count))
    })
    .map_err(|error| {
        map_gate_error(
            operation,
            gate_name,
            timeout_ms,
            Some(last_matched_count),
            error,
        )
    })?;

    Ok(AxGateCheckResult {
        gate: gate_name.to_string(),
        terminal_status: "satisfied".to_string(),
        attempts: outcome.attempts,
        elapsed_ms: started.elapsed().as_millis() as u64,
        matched_count: Some(last_matched_count),
    })
}

fn map_gate_error(
    operation: &str,
    gate_name: &str,
    timeout_ms: u64,
    matched_count: Option<usize>,
    error: CliError,
) -> CliError {
    if error.message().contains("timed out waiting") {
        let mut mapped = CliError::runtime(format!(
            "{operation} pre-action gate `{gate_name}` timed out after {timeout_ms}ms"
        ))
        .with_operation(format!("{operation}.gate.{gate_name}"))
        .with_hint("Increase --gate-timeout-ms or relax gate conditions for slower UIs.");
        if let Some(count) = matched_count {
            mapped = mapped.with_hint(format!(
                "Last AX selector match count before timeout: {count}"
            ));
        }
        return mapped;
    }

    error
        .with_operation(format!("{operation}.gate.{gate_name}"))
        .with_hint("Pre-action gate failed before mutation; fix the gate condition and retry.")
}

fn evaluate_postcondition_check(
    runner: &dyn ProcessRunner,
    backend: &dyn AxBackendAdapter,
    target: &AxTarget,
    node_id: &str,
    check: &AxPostconditionCheck,
    backend_timeout_ms: u64,
) -> Result<(bool, Option<Value>), CliError> {
    match check {
        AxPostconditionCheck::Focused(expected) => {
            let list = backend.list(
                runner,
                &AxListRequest {
                    target: target.clone(),
                    ..AxListRequest::default()
                },
                backend_timeout_ms.max(1),
            )?;
            let observed = list
                .nodes
                .into_iter()
                .find(|node| node.node_id == node_id)
                .map(|node| Value::Bool(node.focused));
            let satisfied = observed.as_ref().and_then(Value::as_bool) == Some(*expected);
            Ok((satisfied, observed))
        }
        AxPostconditionCheck::AttributeValue { name, expected } => {
            let observed = backend
                .attr_get(
                    runner,
                    &AxAttrGetRequest {
                        target: target.clone(),
                        selector: AxSelector {
                            node_id: Some(node_id.to_string()),
                            ..AxSelector::default()
                        },
                        name: name.clone(),
                    },
                    backend_timeout_ms.max(1),
                )?
                .value;
            Ok((observed == *expected, Some(observed)))
        }
    }
}

fn map_postcondition_error(
    operation: &str,
    check: &AxPostconditionCheck,
    timeout_ms: u64,
    observed: Option<Value>,
    error: CliError,
) -> CliError {
    if error.message().contains("timed out waiting") {
        let observed_text = observed
            .map(|value| value.to_string())
            .unwrap_or_else(|| "<none>".to_string());
        return CliError::runtime(format!(
            "{operation} postcondition mismatch for `{}` after {timeout_ms}ms",
            check.name()
        ))
        .with_operation(format!("{operation}.postcondition"))
        .with_hint(format!(
            "Expected={}, observed={observed_text}",
            check.expected_value()
        ))
        .with_hint("Increase --postcondition-timeout-ms or adjust postcondition checks.");
    }

    error
        .with_operation(format!("{operation}.postcondition"))
        .with_hint("Postcondition evaluation failed after action execution.")
}

fn validate_selector_regex(flag: &str, pattern: Option<&str>) -> Result<(), CliError> {
    if let Some(pattern) = pattern {
        RegexBuilder::new(pattern)
            .case_insensitive(true)
            .build()
            .map_err(|err| CliError::usage(format!("{flag} has invalid regex: {err}")))?;
    }
    Ok(())
}

fn apply_stage<F>(
    stage: &str,
    current: &mut Vec<&AxNode>,
    stages: &mut Vec<AxSelectorExplainStage>,
    predicate: F,
) where
    F: Fn(&AxNode) -> bool,
{
    let before_count = current.len();
    current.retain(|node| predicate(node));
    stages.push(AxSelectorExplainStage {
        stage: stage.to_string(),
        before_count,
        after_count: current.len(),
    });
}

enum TextMatcher {
    Contains(String),
    Exact(String),
    Prefix(String),
    Suffix(String),
    Regex(Regex),
}

impl TextMatcher {
    fn matches(&self, raw: &str) -> bool {
        match self {
            Self::Contains(needle) => raw.to_ascii_lowercase().contains(needle),
            Self::Exact(needle) => raw.eq_ignore_ascii_case(needle),
            Self::Prefix(needle) => raw
                .to_ascii_lowercase()
                .starts_with(&needle.to_ascii_lowercase()),
            Self::Suffix(needle) => raw
                .to_ascii_lowercase()
                .ends_with(&needle.to_ascii_lowercase()),
            Self::Regex(regex) => regex.is_match(raw),
        }
    }
}

fn build_text_matcher(raw: &str, strategy: AxMatchStrategy) -> Result<TextMatcher, CliError> {
    let matcher = match strategy {
        AxMatchStrategy::Contains => TextMatcher::Contains(raw.to_ascii_lowercase()),
        AxMatchStrategy::Exact => TextMatcher::Exact(raw.to_string()),
        AxMatchStrategy::Prefix => TextMatcher::Prefix(raw.to_string()),
        AxMatchStrategy::Suffix => TextMatcher::Suffix(raw.to_string()),
        AxMatchStrategy::Regex => TextMatcher::Regex(
            RegexBuilder::new(raw)
                .case_insensitive(true)
                .build()
                .map_err(|err| {
                    CliError::usage(format!("--match-strategy regex pattern is invalid: {err}"))
                })?,
        ),
    };
    Ok(matcher)
}
