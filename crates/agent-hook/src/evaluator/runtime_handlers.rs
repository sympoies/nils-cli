//! Trusted runtime-kit handler selection and execution.

use std::collections::BTreeSet;
use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::contract::runtime_handler_filename;
use crate::error::HookError;
use crate::handler_timeout::handler_timeout;
use crate::model::{Capability, LoadedPolicy, Product};

use super::python_runner::{PythonRunner, validate_runner};
use super::{
    ACTIVITY_HELPER_UNRESOLVABLE, CoordinationHandlerOutcome, ExecutionBudget, HELPER_BIN_ENV,
    MAX_HANDLER_OUTPUT, RuleOutcome, SESSION_COORDINATION_HANDLER, handler_outcome,
    locate_activity_helper, run_bounded, session_coordination_outcome,
};

pub(super) fn run_runtime_handler(
    product: Product,
    handler_id: &str,
    raw: &[u8],
    budget: &mut ExecutionBudget,
    runner: Option<&mut PythonRunner>,
) -> Result<RuleOutcome, HookError> {
    let filename = runtime_handler_filename(handler_id).ok_or_else(|| {
        HookError::data(
            "handler-id-unsupported",
            "runtime-kit handler is not in the compiled v1 allowlist",
        )
    })?;
    let stdout = run_installed_handler(product, filename, raw, Some(budget), runner)?;
    handler_outcome(handler_id, &stdout)
}

pub(super) fn run_session_coordination(
    product: Product,
    raw: &[u8],
    runner: Option<&mut PythonRunner>,
) -> Result<CoordinationHandlerOutcome, HookError> {
    let stdout = run_installed_handler(product, SESSION_COORDINATION_HANDLER, raw, None, runner)?;
    session_coordination_outcome(&stdout)
}

fn run_installed_handler(
    product: Product,
    filename: &str,
    raw: &[u8],
    mut budget: Option<&mut ExecutionBudget>,
    runner: Option<&mut PythonRunner>,
) -> Result<Vec<u8>, HookError> {
    let root = runtime_hook_root(product)?;
    let path = root.join(filename);
    validate_handler(&path)?;
    let (timeout, output_limit) = match budget.as_deref_mut() {
        Some(budget) => budget.reserve_child()?,
        None => (handler_timeout(), MAX_HANDLER_OUTPUT),
    };
    let (success, stdout) = match runner.filter(|_| filename.ends_with(".py")) {
        Some(runner) => runner.request(&root, product, raw, filename, timeout, output_limit)?,
        None => {
            let mut command = Command::new(path);
            command
                .env("AGENT_RUNTIME_PRODUCT", product.as_str())
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null());
            let output = run_bounded(command, raw, timeout, output_limit, false)?;
            (output.status.success(), output.stdout)
        }
    };
    if let Some(budget) = budget {
        budget.retain_output(stdout.len())?;
    }
    if stdout.len() > MAX_HANDLER_OUTPUT {
        return Err(HookError::data(
            "handler-output-too-large",
            "runtime-kit handler output exceeds 256 KiB",
        ));
    }
    if !success {
        return Err(HookError::runtime(
            "handler-failed",
            "runtime-kit handler returned failure",
        ));
    }
    Ok(stdout)
}

fn runtime_hook_root(product: Product) -> Result<PathBuf, HookError> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .ok_or_else(|| HookError::runtime("home-unavailable", "HOME is required"))?;
    Ok(match product {
        Product::Codex => std::env::var_os("CODEX_HOME")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .unwrap_or_else(|| home.join(".codex"))
            .join("hooks"),
        Product::Claude => home.join(".claude/hooks"),
        Product::Dsh => home.join(".dsh/hooks"),
        Product::Hermes => home.join(".hermes/hooks"),
    })
}

pub(crate) fn validate_policy_handlers(
    loaded: &LoadedPolicy,
    product: Product,
) -> Result<(), HookError> {
    let mut handlers = BTreeSet::new();
    let mut needs_activity_helper = false;
    for rule in &loaded.bundle.rules {
        if !rule.products.contains(&product) {
            continue;
        }
        match &rule.capability {
            Capability::RuntimeKitHandler { handler_id } => {
                if let Some(filename) = runtime_handler_filename(handler_id) {
                    handlers.insert((handler_id.clone(), filename));
                }
            }
            Capability::SessionCoordination { .. } => {
                handlers.insert((
                    "session-coordination".to_string(),
                    SESSION_COORDINATION_HANDLER,
                ));
            }
            Capability::SessionActivity { .. } => needs_activity_helper = true,
            _ => {}
        }
    }

    // Ingress registration is not capability health. A provider can be fully
    // converged while the activity helper is unexecutable, which is how a stale
    // `AGENT_SESSION_BIN` deadlocked sessions with every health surface reporting
    // green (`sympoies/nils-cli#1414`).
    if needs_activity_helper && locate_activity_helper().is_none() {
        return Err(HookError::data(
            ACTIVITY_HELPER_UNRESOLVABLE,
            format!(
                "the {} activity capability helper could not be resolved to an \
                 executable file through {HELPER_BIN_ENV}, approved roots, or the same-release sibling",
                nils_common::session_binary::NAME
            ),
        ));
    }

    let root = runtime_hook_root(product)?;
    let mut failures = handlers
        .iter()
        .filter_map(|(handler_id, filename)| {
            validate_handler(&root.join(filename))
                .err()
                .map(|error| (handler_id.clone(), error.code))
        })
        .collect::<Vec<_>>();
    if loaded.bundle.python_runner
        && handlers
            .iter()
            .any(|(_, filename)| filename.ends_with(".py"))
        && let Err(error) = validate_runner(&root)
    {
        failures.push(("event-runner".to_string(), error.code));
    }
    if failures.is_empty() {
        return Ok(());
    }
    let summary = failures
        .iter()
        .map(|(handler_id, code)| format!("{handler_id} ({code})"))
        .collect::<Vec<_>>()
        .join(", ");
    if failures.iter().any(|(_, code)| code == "handler-untrusted") {
        Err(HookError::data(
            "handler-untrusted",
            format!(
                "runtime-kit handlers are untrusted for {}: {summary}",
                product.as_str()
            ),
        ))
    } else {
        Err(HookError::runtime(
            "handler-unavailable",
            format!(
                "runtime-kit handlers are unavailable for {}: {summary}",
                product.as_str()
            ),
        ))
    }
}

pub(super) fn validate_handler(path: &Path) -> Result<(), HookError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| {
        HookError::runtime("handler-unavailable", "runtime-kit handler is unavailable")
    })?;
    let mode = metadata.permissions().mode();
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.uid() != unsafe { libc::geteuid() }
        || mode & 0o100 == 0
        || mode & 0o022 != 0
    {
        return Err(HookError::data(
            "handler-untrusted",
            "runtime-kit handler type, executable bit, owner, or mode is untrusted",
        ));
    }
    Ok(())
}
