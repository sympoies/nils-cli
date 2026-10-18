use crate::{CliContext, CliError, cli, lineage};
use serde_json::{Value, json};
use std::path::Path;

#[derive(clap::Args, Debug, Clone, Default)]
pub struct OperationArgs {
    /// Time-bearing UUIDv7 for a Console start; replay never creates another session.
    #[arg(long, value_name = "UUID", requires = "via_console")]
    pub operation_id: Option<String>,
    /// Inspect a prior Console start operation without creating a session.
    #[arg(long, requires_all = ["via_console", "operation_id"], conflicts_with_all = ["prompt", "prompt_file", "prompt_stdin", "launch_env"])]
    pub reconcile: bool,
}

pub(crate) fn start_args(
    context: &CliContext,
    mut args: cli::StartArgs,
    work: &lineage::WorkRequest,
) -> Result<Value, CliError> {
    let operation_id = args
        .console_operation
        .operation_id
        .clone()
        .unwrap_or_else(|| uuid::Uuid::now_v7().to_string());
    if !super::operation::valid_id(&operation_id) {
        return Err(super::invalid());
    }
    args.console_operation.operation_id = Some(operation_id.clone());
    super::diagnostics::run(&operation_id, || build_args(context, args, work))
}

fn build_args(
    context: &CliContext,
    args: cli::StartArgs,
    work: &lineage::WorkRequest,
) -> Result<Value, CliError> {
    if args.console_operation.reconcile {
        return super::cli_operation(
            context,
            None,
            None,
            json!({}),
            &super::ChildLineage::default(),
            &args.console_operation,
        );
    }
    let cwd = crate::absolute_path(args.cwd.as_deref().unwrap_or(Path::new(".")))?;
    let mut session = json!({ "agent": args.agent.as_str(), "cwd": cwd.to_string_lossy() });
    if let Some(workspace) = &args.workspace {
        crate::origin::validate_workspace(workspace)?;
        session["workspace"] = json!(workspace);
    }
    let launch_env = crate::launch_env::from_flags(&args.launch_env)?;
    if !launch_env.is_empty() {
        session["launch_env"] = json!(launch_env);
    }
    if args.title_mode == "pinned" {
        session["title_mode"] = json!(args.title_mode);
    }
    if let Some(title) = &args.title {
        session["title"] = json!(title);
    }
    if let Some(prompt) =
        crate::read_prompt(&args.prompt, args.prompt_file.as_deref(), args.prompt_stdin)?
    {
        session["prompt"] = json!(prompt);
    }
    if !args.agent_args.is_empty() {
        session["agent_args"] = json!(args.agent_args);
    }
    if let Some(profile) = &args.agent_profile {
        session["agent_profile"] = json!(profile);
    }
    if let Some(account) = &args.account {
        let field = match args.agent {
            cli::AgentKind::Codex => "codex_account",
            cli::AgentKind::Claude => "claude_account",
            _ => {
                return Err(CliError::usage(
                    "console-start-account-unsupported",
                    "--account selects a Codex or Claude account and needs --agent codex or --agent claude",
                    None,
                ));
            }
        };
        session[field] = json!(account);
    }
    let lineage = super::ChildLineage {
        no_parent: args.no_parent,
        work: work.clone(),
        role: args.role.clone(),
    };
    super::cli_operation(
        context,
        None,
        args.machine.as_deref(),
        session,
        &lineage,
        &args.console_operation,
    )
}

pub(crate) fn render_text(result: &Value) -> String {
    if let Some(state) = result["reconciliation"]["state"].as_str() {
        let mut text = format!(
            "console start operation {}: {state}; safe_to_retry={}\n",
            result["operation_id"].as_str().unwrap_or_default(),
            result["reconciliation"]["safe_to_retry"]
        );
        if let Some(id) = result["session"]["id"].as_str() {
            text.push_str(&format!("existing session: {id}\n"));
        }
        if let Some(candidates) = result["reconciliation"]["candidates"].as_array() {
            for candidate in candidates {
                text.push_str(&format!(
                    "candidate: {}/{}\n",
                    candidate["machine"].as_str().unwrap_or_default(),
                    candidate["session_id"].as_str().unwrap_or_default()
                ));
            }
        }
        return text;
    }
    let session = &result["session"];
    let id = session["id"].as_str().unwrap_or_default();
    let mut text = format!(
        "started {} session {id} on {} through Agent Console\n",
        session["agent"].as_str().unwrap_or_default(),
        result["machine"].as_str().unwrap_or_default(),
    );
    if let Some(command) = session["attach_command"].as_str() {
        text.push_str(&format!("attach: {command}\n"));
    }
    text
}
