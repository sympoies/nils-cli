//! Session previews shared by `serve` list enrichment and `agent-session previews`.
//!
//! The list `last_prompt` projection and the History START/LATEST rows come from the
//! same functions the daemon uses, so the CLI read cannot drift from the Console data.
//! The CLI read is local and read-only: it needs state access, not the daemon or its token.
use super::*;
use nils_common::cli_contract::{Envelope, OutputFormat};

const PREVIEW_WAIT: Duration = Duration::from_secs(15);
const PREVIEW_POLL: Duration = Duration::from_millis(50);

/// Best-effort enrichment: attach each running Codex/Claude session's most recent
/// user prompt to the list projection.
///
/// Cold recovery and bounded backlog catch-up run in the background. Eligible
/// sessions report `last_prompt_state` so a consumer can distinguish pending
/// catch-up from an authoritative empty or unavailable projection. The latest
/// caught-up value remains process-local and is returned in the response only;
/// it is never persisted.
pub(super) async fn enrich_last_prompts(
    context: &CliContext,
    registry: &ProviderPromptDiscoveryRegistry,
    sessions: &mut [SessionView],
) {
    for session in sessions.iter_mut() {
        if session.status == "stopped" || !matches!(session.agent.as_str(), "codex" | "claude") {
            continue;
        }
        let owned = context.clone();
        let id = session.id.clone();
        let Ok(Ok(record)) =
            tokio::task::spawn_blocking(move || load_session_record(&owned, &id)).await
        else {
            continue;
        };
        if let Some(projection) = registry
            .last_prompt_projection(&record, Some(&context.state_dir))
            .await
        {
            session.previews = crate::provider_prompt::SessionPreviews {
                last_prompt_state: Some(projection.state),
                last_prompt_continuity: projection.continuity,
                last_prompt: projection.prompt,
                last_message: projection.message,
            };
        }
    }
}

pub(super) fn history_catalog(
    context: &CliContext,
    launch_profiles: &AgentLaunchProfiles,
) -> HistoryCatalog {
    HistoryCatalog::new(
        launch_profiles.history_sources(),
        provider_history::archive_root(&context.state_dir),
        provider_history::star_root(&context.state_dir),
    )
    .with_session_state_dir(context.state_dir.clone())
    .with_dsh_sources(launch_profiles.dsh_history_sources())
}

#[derive(Debug, Serialize)]
struct PreviewsResult {
    id: String,
    #[serde(flatten)]
    previews: crate::provider_prompt::SessionPreviews,
    #[serde(skip_serializing_if = "Option::is_none")]
    history_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    history_error: Option<&'static str>,
    start: Vec<Value>,
    latest: Vec<Value>,
}

pub(crate) fn run_previews(context: &CliContext, args: cli::PreviewsArgs) -> i32 {
    const COMMAND: &str = "previews";
    let format = args.format;
    match collect(context, &args) {
        Ok(result) => match format {
            OutputFormat::Json => crate::print_json(&Envelope::success(
                schema_version_for(SCHEMA, COMMAND, 1),
                &result,
            )),
            OutputFormat::Text => {
                print!("{}", render_text(&result));
                exit::SUCCESS
            }
        },
        Err(err) => crate::render_error(COMMAND, format, err),
    }
}

fn collect(context: &CliContext, args: &cli::PreviewsArgs) -> Result<PreviewsResult, CliError> {
    let record = load_session_record(context, &args.id)?;
    let launch_profiles = AgentLaunchProfiles::from_environment()?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|err| {
            CliError::runtime(
                "previews-runtime-failed",
                format!("failed to build runtime: {err}"),
                None,
            )
        })?;
    // One record, no all-session traversal: list reconciliation and resume backfill
    // persist session.json, which a read-only command must not do.
    let mut views = vec![crate::session_view(context, &record, None, None)];
    let registry = ProviderPromptDiscoveryRegistry::default();
    let deadline = Instant::now() + PREVIEW_WAIT;
    runtime.block_on(async {
        loop {
            enrich_last_prompts(context, &registry, &mut views).await;
            let pending = views
                .iter()
                .any(|view| view.previews.last_prompt_state == Some(LastPromptState::Pending));
            if !pending || Instant::now() >= deadline {
                break;
            }
            tokio::time::sleep(PREVIEW_POLL).await;
        }
    });
    let previews = views.pop().map(|view| view.previews).unwrap_or_default();
    let mut result = PreviewsResult {
        id: record.id.clone(),
        previews,
        history_id: None,
        history_error: None,
        start: Vec::new(),
        latest: Vec::new(),
    };
    let Some(resume) = record.provider_resume.as_ref() else {
        return Ok(result);
    };
    let history_id = provider_history::stable_history_id(
        &resume.provider,
        crate::session_agent_profile(&record),
        &resume.session_id,
    );
    let catalog = history_catalog(context, &launch_profiles);
    match history_rows(&catalog, &history_id, args.rows, args.parts.is_some()) {
        Ok((start, latest)) => {
            result.start = start;
            result.latest = latest;
        }
        Err(HistoryError::NotFound) => result.history_error = Some("history-not-found"),
        Err(_) => result.history_error = Some("history-read-failed"),
    }
    result.history_id = Some(history_id);
    Ok(result)
}

/// The History START (oldest forward page) and LATEST (newest page) rows, read through
/// the catalog call the serve `/history/sessions/{id}/messages` route uses.
fn history_rows(
    catalog: &HistoryCatalog,
    history_id: &str,
    rows: usize,
    include_parts: bool,
) -> Result<(Vec<Value>, Vec<Value>), HistoryError> {
    let page = |direction| {
        catalog
            .messages_with_mailbox_events(history_id, None, rows, direction, include_parts)
            .map(|page| page.messages)
    };
    Ok((
        page(HistoryMessageDirection::Forward)?,
        page(HistoryMessageDirection::Latest)?,
    ))
}

fn render_text(result: &PreviewsResult) -> String {
    let mut out = format!("session {}\n", result.id);
    if let Some(state) = result.previews.last_prompt_state {
        out.push_str(&format!("last_prompt_state: {state:?}\n").to_lowercase());
    }
    if let Some(prompt) = &result.previews.last_prompt {
        out.push_str(&format!("last_prompt: {}\n", prompt.text));
    }
    if let Some(error) = result.history_error {
        out.push_str(&format!("history: {error}\n"));
    }
    for (label, rows) in [("START", &result.start), ("LATEST", &result.latest)] {
        for row in rows {
            let field = |key| row[key].as_str().unwrap_or("-");
            out.push_str(&format!(
                "{label} {} {} {}\n",
                field("role"),
                field("row_class"),
                field("text").replace('\n', " ")
            ));
        }
    }
    out
}

#[cfg(test)]
#[path = "serve_session_previews_tests.rs"]
mod tests;
