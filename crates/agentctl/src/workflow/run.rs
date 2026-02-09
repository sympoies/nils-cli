use super::schema::{
    AutomationStep, ProviderStep, WorkflowDocument, WorkflowOnError, WorkflowSchemaError,
    WorkflowStep,
};
use super::steps::automation::{AutomationCommandProvenance, resolve_automation_invocation};
use crate::provider::registry::ProviderRegistry;
use agent_runtime_core::schema::{ExecuteRequest, ProviderError};
use clap::{Args, ValueEnum};
use serde::Serialize;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const EXIT_OK: i32 = 0;
const EXIT_RUNTIME_ERROR: i32 = 1;
const EXIT_USAGE: i32 = 64;
const TIMEOUT_EXIT_CODE: i32 = 124;
const WORKFLOW_RUN_SCHEMA_VERSION: &str = "agentctl.workflow.run.v1";
const WORKFLOW_ARTIFACT_NAMESPACE: &str = "agentctl-workflow";

#[derive(Debug, Args)]
pub struct RunArgs {
    /// Path to workflow manifest JSON file
    #[arg(long)]
    pub file: PathBuf,

    /// Render format
    #[arg(long, value_enum, default_value_t = RunOutputFormat::Json)]
    pub format: RunOutputFormat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Default)]
pub enum RunOutputFormat {
    Text,
    #[default]
    Json,
}

#[derive(Debug, Clone, Serialize)]
pub struct WorkflowRunReport {
    pub schema_version: &'static str,
    pub command: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workflow_name: Option<String>,
    pub on_error: WorkflowOnError,
    pub summary: WorkflowRunSummary,
    pub ledger: Vec<StepLedgerEntry>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WorkflowRunSummary {
    pub total_steps: usize,
    pub executed_steps: usize,
    pub succeeded_steps: usize,
    pub failed_steps: usize,
    pub skipped_steps: usize,
    pub elapsed_ms: u64,
    pub success: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct StepLedgerEntry {
    pub step_id: String,
    pub step_type: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub automation_tool: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<AutomationCommandProvenance>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifact_paths: Vec<String>,
    pub attempts: u32,
    pub status: StepStatus,
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum StepStatus {
    Succeeded,
    Failed,
}

impl StepStatus {
    fn is_failed(self) -> bool {
        matches!(self, Self::Failed)
    }
}

#[derive(Debug)]
pub enum WorkflowLoadError {
    Read {
        path: PathBuf,
        source: io::Error,
    },
    Parse {
        path: PathBuf,
        source: serde_json::Error,
    },
    Schema(WorkflowSchemaError),
}

impl std::fmt::Display for WorkflowLoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Read { path, source } => {
                write!(
                    f,
                    "failed to read workflow file '{}': {}",
                    path.display(),
                    source
                )
            }
            Self::Parse { path, source } => write!(
                f,
                "failed to parse workflow file '{}' as JSON: {}",
                path.display(),
                source
            ),
            Self::Schema(error) => write!(f, "invalid workflow schema: {error}"),
        }
    }
}

impl std::error::Error for WorkflowLoadError {}

pub fn run(args: RunArgs) -> i32 {
    let workflow = match load_workflow_file(&args.file) {
        Ok(workflow) => workflow,
        Err(error) => {
            eprintln!("agentctl workflow run: {error}");
            return EXIT_USAGE;
        }
    };

    let report = execute_workflow_document(&workflow);
    let render_exit = match args.format {
        RunOutputFormat::Json => emit_json(&report),
        RunOutputFormat::Text => emit_text(&report),
    };
    if render_exit != EXIT_OK {
        return EXIT_RUNTIME_ERROR;
    }

    if report.summary.success {
        EXIT_OK
    } else {
        EXIT_RUNTIME_ERROR
    }
}

pub fn load_workflow_file(path: &Path) -> Result<WorkflowDocument, WorkflowLoadError> {
    let raw = fs::read_to_string(path).map_err(|source| WorkflowLoadError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let workflow: WorkflowDocument =
        serde_json::from_str(raw.as_str()).map_err(|source| WorkflowLoadError::Parse {
            path: path.to_path_buf(),
            source,
        })?;
    workflow.validate().map_err(WorkflowLoadError::Schema)?;
    Ok(workflow)
}

pub fn execute_workflow_document(workflow: &WorkflowDocument) -> WorkflowRunReport {
    let started = Instant::now();
    let registry = ProviderRegistry::with_builtins();
    let mut ledger = Vec::with_capacity(workflow.steps.len());

    for step in &workflow.steps {
        let entry = execute_step(step, &registry);
        let failed = entry.status.is_failed();
        ledger.push(entry);

        if failed && workflow.on_error == WorkflowOnError::FailFast {
            break;
        }
    }

    let succeeded_steps = ledger
        .iter()
        .filter(|entry| entry.status == StepStatus::Succeeded)
        .count();
    let failed_steps = ledger.len().saturating_sub(succeeded_steps);
    let total_steps = workflow.steps.len();
    let executed_steps = ledger.len();
    let skipped_steps = total_steps.saturating_sub(executed_steps);

    WorkflowRunReport {
        schema_version: WORKFLOW_RUN_SCHEMA_VERSION,
        command: "workflow-run",
        workflow_name: workflow.name.clone(),
        on_error: workflow.on_error,
        summary: WorkflowRunSummary {
            total_steps,
            executed_steps,
            succeeded_steps,
            failed_steps,
            skipped_steps,
            elapsed_ms: as_millis(started.elapsed()),
            success: failed_steps == 0,
        },
        ledger,
    }
}

fn execute_step(step: &WorkflowStep, registry: &ProviderRegistry) -> StepLedgerEntry {
    let retry = step.retry();
    let max_attempts = retry.normalized_max_attempts();
    let started = Instant::now();

    let mut attempts = 0;
    let mut last = AttemptOutcome::failed(
        EXIT_RUNTIME_ERROR,
        String::new(),
        "workflow step did not execute".to_string(),
    );
    for attempt in 1..=max_attempts {
        attempts = attempt;
        last = match step {
            WorkflowStep::Provider(provider_step) => execute_provider_step(provider_step, registry),
            WorkflowStep::Automation(automation_step) => {
                execute_automation_step(step.id(), attempt, automation_step)
            }
        };
        if last.status == StepStatus::Succeeded {
            break;
        }

        if attempt < max_attempts && retry.backoff_ms > 0 {
            thread::sleep(Duration::from_millis(retry.backoff_ms));
        }
    }

    let (step_type, provider, automation_tool) = match step {
        WorkflowStep::Provider(_) => ("provider", last.provider.clone(), None),
        WorkflowStep::Automation(_) => ("automation", None, last.automation_tool.clone()),
    };

    StepLedgerEntry {
        step_id: step.id().to_string(),
        step_type,
        provider,
        automation_tool,
        command: last.command,
        artifact_paths: last.artifact_paths,
        attempts,
        status: last.status,
        exit_code: last.exit_code,
        stdout: last.stdout,
        stderr: last.stderr,
        elapsed_ms: as_millis(started.elapsed()),
    }
}

fn execute_provider_step(step: &ProviderStep, registry: &ProviderRegistry) -> AttemptOutcome {
    let selection = match registry.resolve_selection(step.provider.as_deref()) {
        Ok(selection) => selection,
        Err(error) => {
            return AttemptOutcome {
                status: StepStatus::Failed,
                exit_code: EXIT_USAGE,
                stdout: String::new(),
                stderr: error.to_string(),
                provider: step.provider.clone(),
                automation_tool: None,
                command: None,
                artifact_paths: Vec::new(),
            };
        }
    };

    let provider_id = selection.provider_id;
    let Some(adapter) = registry.get(provider_id.as_str()) else {
        return AttemptOutcome {
            status: StepStatus::Failed,
            exit_code: EXIT_RUNTIME_ERROR,
            stdout: String::new(),
            stderr: format!("selected provider '{}' is not registered", provider_id),
            provider: Some(provider_id),
            automation_tool: None,
            command: None,
            artifact_paths: Vec::new(),
        };
    };

    let request = ExecuteRequest {
        task: step.task.clone(),
        input: step.input.clone(),
        timeout_ms: step.timeout_ms,
    };
    match adapter.execute(request) {
        Ok(response) => AttemptOutcome {
            status: if response.exit_code == 0 {
                StepStatus::Succeeded
            } else {
                StepStatus::Failed
            },
            exit_code: response.exit_code,
            stdout: response.stdout,
            stderr: response.stderr,
            provider: Some(provider_id),
            automation_tool: None,
            command: None,
            artifact_paths: Vec::new(),
        },
        Err(error) => provider_error_outcome(provider_id, error.as_ref()),
    }
}

fn provider_error_outcome(provider_id: String, error: &ProviderError) -> AttemptOutcome {
    let mut stderr = error.message.clone();
    if let Some(extra_stderr) = error
        .details
        .as_ref()
        .and_then(|details| details.get("stderr"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        stderr.push('\n');
        stderr.push_str(extra_stderr);
    }

    let stdout = error
        .details
        .as_ref()
        .and_then(|details| details.get("stdout"))
        .and_then(|value| value.as_str())
        .map(ToOwned::to_owned)
        .unwrap_or_default();

    let exit_code = error
        .details
        .as_ref()
        .and_then(|details| details.get("exit_code"))
        .and_then(|value| value.as_i64())
        .and_then(|value| i32::try_from(value).ok())
        .unwrap_or(EXIT_RUNTIME_ERROR);

    AttemptOutcome {
        status: StepStatus::Failed,
        exit_code,
        stdout,
        stderr,
        provider: Some(provider_id),
        automation_tool: None,
        command: None,
        artifact_paths: Vec::new(),
    }
}

fn execute_automation_step(step_id: &str, attempt: u32, step: &AutomationStep) -> AttemptOutcome {
    let invocation = resolve_automation_invocation(step);
    let mut outcome = match run_command(
        invocation.command.as_str(),
        invocation.args.as_slice(),
        invocation.env.as_slice(),
        step.timeout_ms,
    ) {
        Ok(output) => AttemptOutcome {
            status: if output.exit_code == 0 {
                StepStatus::Succeeded
            } else {
                StepStatus::Failed
            },
            exit_code: output.exit_code,
            stdout: output.stdout,
            stderr: output.stderr,
            provider: None,
            automation_tool: Some(step.tool.as_id().to_string()),
            command: Some(invocation.provenance),
            artifact_paths: Vec::new(),
        },
        Err(error) => AttemptOutcome {
            status: StepStatus::Failed,
            exit_code: EXIT_RUNTIME_ERROR,
            stdout: String::new(),
            stderr: format!(
                "failed to run automation tool '{}': {}",
                step.tool.as_id(),
                error
            ),
            provider: None,
            automation_tool: Some(step.tool.as_id().to_string()),
            command: Some(invocation.provenance),
            artifact_paths: Vec::new(),
        },
    };

    match persist_automation_artifacts(step_id, attempt, &outcome.stdout, &outcome.stderr) {
        Ok(artifact_paths) => {
            outcome.artifact_paths = artifact_paths;
        }
        Err(error) => {
            if !outcome.stderr.is_empty() && !outcome.stderr.ends_with('\n') {
                outcome.stderr.push('\n');
            }
            outcome
                .stderr
                .push_str(format!("failed to persist step artifacts: {error}").as_str());
        }
    }

    outcome
}

struct AttemptOutcome {
    status: StepStatus,
    exit_code: i32,
    stdout: String,
    stderr: String,
    provider: Option<String>,
    automation_tool: Option<String>,
    command: Option<AutomationCommandProvenance>,
    artifact_paths: Vec<String>,
}

impl AttemptOutcome {
    fn failed(exit_code: i32, stdout: String, stderr: String) -> Self {
        Self {
            status: StepStatus::Failed,
            exit_code,
            stdout,
            stderr,
            provider: None,
            automation_tool: None,
            command: None,
            artifact_paths: Vec::new(),
        }
    }
}

struct CommandOutput {
    exit_code: i32,
    stdout: String,
    stderr: String,
}

fn run_command(
    command: &str,
    args: &[String],
    env: &[(String, String)],
    timeout_ms: Option<u64>,
) -> io::Result<CommandOutput> {
    let mut cmd = Command::new(command);
    cmd.args(args);
    for (key, value) in env {
        cmd.env(key, value);
    }

    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    let stdout_reader = child.stdout.take().map(spawn_pipe_reader);
    let stderr_reader = child.stderr.take().map(spawn_pipe_reader);

    let started = Instant::now();
    let timeout = timeout_ms.map(Duration::from_millis);
    let mut timed_out = false;

    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }

        if let Some(timeout) = timeout
            && started.elapsed() >= timeout
        {
            timed_out = true;
            let _ = child.kill();
            break child.wait()?;
        }

        thread::sleep(Duration::from_millis(5));
    };

    let stdout = pipe_reader_output(stdout_reader);
    let stderr = pipe_reader_output(stderr_reader);
    let mut stderr_text = String::from_utf8_lossy(stderr.as_slice()).to_string();

    let exit_code = if timed_out {
        if !stderr_text.is_empty() && !stderr_text.ends_with('\n') {
            stderr_text.push('\n');
        }
        stderr_text.push_str("step timed out");
        if let Some(timeout_ms) = timeout_ms {
            stderr_text.push_str(format!(" after {}ms", timeout_ms).as_str());
        }
        TIMEOUT_EXIT_CODE
    } else {
        status.code().unwrap_or(EXIT_RUNTIME_ERROR)
    };

    Ok(CommandOutput {
        exit_code,
        stdout: String::from_utf8_lossy(stdout.as_slice()).to_string(),
        stderr: stderr_text,
    })
}

fn persist_automation_artifacts(
    step_id: &str,
    attempt: u32,
    stdout: &str,
    stderr: &str,
) -> io::Result<Vec<String>> {
    let artifact_dir = workflow_artifact_dir(step_id, attempt);
    fs::create_dir_all(&artifact_dir)?;

    let stdout_path = artifact_dir.join("stdout.log");
    let stderr_path = artifact_dir.join("stderr.log");
    fs::write(&stdout_path, stdout.as_bytes())?;
    fs::write(&stderr_path, stderr.as_bytes())?;

    Ok(vec![
        path_to_string(stdout_path.as_path()),
        path_to_string(stderr_path.as_path()),
    ])
}

fn workflow_artifact_dir(step_id: &str, attempt: u32) -> PathBuf {
    codex_out_dir()
        .join(WORKFLOW_ARTIFACT_NAMESPACE)
        .join(sanitize_component(step_id))
        .join(format!("attempt-{attempt}"))
}

fn sanitize_component(value: &str) -> String {
    let mut sanitized = String::with_capacity(value.len());
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            sanitized.push(ch);
        } else {
            sanitized.push('-');
        }
    }

    let trimmed = sanitized.trim_matches('-');
    if trimmed.is_empty() {
        "step".to_string()
    } else {
        trimmed.to_string()
    }
}

fn path_to_string(path: &Path) -> String {
    path.to_string_lossy().to_string()
}

fn codex_out_dir() -> PathBuf {
    if let Ok(codex_home) = std::env::var("CODEX_HOME") {
        return PathBuf::from(codex_home).join("out");
    }
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home).join(".codex").join("out");
    }
    PathBuf::from(".codex").join("out")
}

fn spawn_pipe_reader<R>(mut reader: R) -> thread::JoinHandle<Vec<u8>>
where
    R: Read + Send + 'static,
{
    thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = reader.read_to_end(&mut bytes);
        bytes
    })
}

fn pipe_reader_output(handle: Option<thread::JoinHandle<Vec<u8>>>) -> Vec<u8> {
    let Some(handle) = handle else {
        return Vec::new();
    };
    handle.join().unwrap_or_default()
}

fn emit_json<T: Serialize>(value: &T) -> i32 {
    match serde_json::to_string_pretty(value) {
        Ok(encoded) => {
            println!("{encoded}");
            EXIT_OK
        }
        Err(error) => {
            eprintln!("agentctl workflow run: failed to render json output: {error}");
            EXIT_RUNTIME_ERROR
        }
    }
}

fn emit_text(report: &WorkflowRunReport) -> i32 {
    println!("schema_version: {}", report.schema_version);
    println!("command: {}", report.command);
    if let Some(workflow_name) = report.workflow_name.as_deref() {
        println!("workflow_name: {workflow_name}");
    }
    println!(
        "on_error: {}",
        match report.on_error {
            WorkflowOnError::FailFast => "fail-fast",
            WorkflowOnError::ContinueOnError => "continue-on-error",
        }
    );
    println!(
        "summary: total={} executed={} succeeded={} failed={} skipped={} elapsed_ms={} success={}",
        report.summary.total_steps,
        report.summary.executed_steps,
        report.summary.succeeded_steps,
        report.summary.failed_steps,
        report.summary.skipped_steps,
        report.summary.elapsed_ms,
        report.summary.success
    );
    println!("ledger:");
    for step in &report.ledger {
        println!(
            "- {} [{}] status={} attempts={} exit_code={} elapsed_ms={}",
            step.step_id,
            step.step_type,
            match step.status {
                StepStatus::Succeeded => "succeeded",
                StepStatus::Failed => "failed",
            },
            step.attempts,
            step.exit_code,
            step.elapsed_ms
        );
    }
    EXIT_OK
}

fn as_millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}
