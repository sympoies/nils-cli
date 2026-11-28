//! Glance projects one session and its terminal tail without changing the runtime.
use super::*;
use crate::auth_incident::recovery;

const GLANCE_COMMAND: &str = "glance";

pub(crate) fn run(context: &CliContext, args: cli::GlanceArgs) -> i32 {
    let format = args.format;
    match glance_session(context, args) {
        Ok(result) => render_single_success(GLANCE_COMMAND, format, &result, render_glance_text),
        Err(err) => render_error(GLANCE_COMMAND, format, err),
    }
}

#[derive(Debug, Serialize)]
pub(super) struct GlanceResult {
    #[serde(skip_serializing_if = "Option::is_none")]
    auth_recovery: Option<recovery::report::Outcome>,
    #[serde(skip_serializing_if = "Option::is_none")]
    native_notice: Option<send::claude_notice::attach::Diagnostic>,
    id: String,
    agent: String,
    title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    title_state: Option<SessionTitleStateView>,
    title_state_supported: bool,
    title_revision: u64,
    tmux_session: String,
    status: String,
    resumable: bool,
    repo_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    provider_resume: Option<ProviderResumeView>,
    tail: String,
    created_at: String,
    updated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_terminal_activity_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    runtime_started_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    turn_state: Option<activity::TurnState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    startup: Option<StartupProjection>,
    auto_resume: auto_resume::AutoResumeView,
    #[serde(flatten)]
    coordination: coordination::CoordinationSummary,
}

pub(super) fn glance_session(
    context: &CliContext,
    args: cli::GlanceArgs,
) -> Result<GlanceResult, CliError> {
    let mut record = load_session_record(context, &args.id)?;
    let tmux_bin = resolve_tmux_bin(args.tmux_bin.as_deref());
    let status = session_status(context, &tmux_bin, &record);
    let (reconciled, status) = reconcile_startup_projection(context, record, status, None);
    record = reconciled;
    if status != "running" {
        record = backfill_provider_resume(context, record);
    }
    let tail = if status == "running" {
        capture_pane_tail(&record, args.tail, &tmux_bin)?
    } else {
        String::new()
    };
    let last_terminal_activity_at = last_terminal_activity_at(&tmux_bin, &record, &status);
    Ok(GlanceResult {
        auth_recovery: recovery::report::for_record(context, &record),
        native_notice: send::claude_notice::attach::diagnostic(context, &record),
        id: record.id.clone(),
        agent: record.agent.clone(),
        title: record.title.clone(),
        title_state: effective_session_title_state(&record),
        title_state_supported: true,
        title_revision: record.title_revision,
        tmux_session: record.tmux_session.clone(),
        status,
        resumable: is_resumable(&record),
        repo_name: repo_name_from_cwd(&record.cwd),
        provider_resume: record
            .provider_resume
            .as_ref()
            .map(ProviderResumeView::from),
        tail,
        created_at: record.created_at.clone(),
        updated_at: record.updated_at.clone(),
        last_terminal_activity_at,
        runtime_started_at: record
            .runtime
            .as_ref()
            .map(|runtime| runtime.started_at.clone()),
        turn_state: activity::state_for_view(context, &record),
        startup: startup_projection_for_view(&record),
        auto_resume: auto_resume::view_for_record(context, &record),
        coordination: coordination::public_summary(context, &record.id),
    })
}

fn render_glance_text(result: &GlanceResult) -> String {
    let mut text = format!("{} {} [{}]\n", result.id, result.agent, result.status);
    if let Some(notice) = &result.native_notice {
        text.push_str(&notice.text());
    }
    if let Some(outcome) = &result.auth_recovery {
        text.push_str(&outcome.text());
    }
    text.push_str(&result.tail);
    if !result.tail.is_empty() && !result.tail.ends_with('\n') {
        text.push('\n');
    }
    text
}
