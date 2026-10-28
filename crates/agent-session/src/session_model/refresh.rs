//! On-demand re-detection of the provider-owned facts a session displays.
//!
//! The model and reasoning effort recorded at launch go stale after a manual
//! change inside the provider (for example `/model`). This reads the provider's
//! own transcript once, updates the record under its lock, and reports what
//! changed. It is operator-triggered; nothing polls.
//!
//! The provider account cannot be re-detected here: the account brokers are
//! token-free and expose no signed-in identity, so `account` is always listed
//! in `unavailable` and the response carries the current account views.
use serde_json::{Value, json};

use super::{KEY, ModelSettings};
use crate::cli::SessionRefArgs;
use crate::{
    CliContext, CliError, SessionRecord, display_metadata, lineage::SessionWork, provider_prompt,
    render_error, render_single_success,
};

const COMMAND: &str = "refresh";
const MAX_TITLE_REFERENCES: usize = 2;

/// The `command` subcommand. It lives here so `lib.rs` stays within its
/// file-size baseline.
pub(crate) fn run_show(context: &CliContext, args: SessionRefArgs) -> i32 {
    match crate::load_session_view(context, &args.id, None) {
        Ok(view) => render_single_success(
            crate::COMMAND_COMMAND,
            args.format,
            &view,
            crate::render_command_text,
        ),
        Err(err) => render_error(crate::COMMAND_COMMAND, args.format, err),
    }
}

pub(crate) fn run(context: &CliContext, args: SessionRefArgs) -> i32 {
    match refresh(context, &args.id) {
        Ok(result) => render_single_success(COMMAND, args.format, &result, render_text),
        Err(err) => render_error(COMMAND, args.format, err),
    }
}

/// Live facts read from the provider. `None` means the provider gave no usable
/// evidence, so the recorded value stays as it is.
struct Evidence {
    model: Option<String>,
    effort: Option<String>,
    launch_id: Option<String>,
    provider_session_id: Option<String>,
}

impl Evidence {
    fn read(record: &SessionRecord) -> Self {
        let (model, effort) = if record.agent == "claude" {
            // Claude transcripts record the model but never the effort.
            (provider_prompt::latest_claude_model(record).flatten(), None)
        } else {
            provider_prompt::latest_codex_turn_settings(record).unwrap_or_default()
        };
        Self {
            model,
            effort,
            launch_id: record
                .runtime
                .as_ref()
                .map(|runtime| runtime.launch_id.clone()),
            provider_session_id: record
                .provider_resume
                .as_ref()
                .map(|resume| resume.session_id.clone()),
        }
    }

    /// The record still belongs to the runtime and provider session that was read.
    fn describes(&self, record: &SessionRecord) -> bool {
        self.launch_id.as_deref()
            == record
                .runtime
                .as_ref()
                .map(|runtime| runtime.launch_id.as_str())
            && self.provider_session_id.as_deref()
                == record
                    .provider_resume
                    .as_ref()
                    .map(|resume| resume.session_id.as_str())
    }
}

pub(crate) fn refresh(context: &CliContext, id: &str) -> Result<Value, CliError> {
    crate::validate_id(id)?;
    let observed = crate::load_session_record(context, id)?;
    if !matches!(observed.agent.as_str(), "claude" | "codex") {
        return Err(CliError::usage(
            "refresh-unsupported-agent",
            "only claude and codex sessions have a provider transcript to re-read",
            None,
        ));
    }
    let evidence = Evidence::read(&observed);
    let changed = {
        let _lock = crate::acquire_session_record_lock(context, &observed.id)?;
        let mut record = crate::load_session_record(context, &observed.id)?;
        crate::ensure_same_session_identity(&observed, &record)?;
        let changed = apply(&mut record, &evidence)?;
        if !changed.is_empty() {
            crate::write_session_record(context, &record)?;
        }
        changed
    };
    let mut unavailable = Vec::new();
    if evidence.model.is_none() {
        unavailable.push("model");
    }
    if evidence.effort.is_none() {
        unavailable.push("reasoning_effort");
    }
    unavailable.push("account");
    let view = crate::load_session_view(context, &observed.id, None)?;
    Ok(json!({
        "session_id": observed.id,
        "changed": changed,
        "unavailable": unavailable,
        "session": view,
    }))
}

fn apply(record: &mut SessionRecord, evidence: &Evidence) -> Result<Vec<&'static str>, CliError> {
    if !evidence.describes(record) {
        return Ok(Vec::new());
    }
    let mut changed = Vec::new();
    let before = ModelSettings::for_record(record);
    let mut settings = before.clone();
    if let Some(model) = &evidence.model
        && settings.model.as_ref() != Some(model)
    {
        settings.model = Some(model.clone());
        // An effort belongs to the model it was set for.
        settings.reasoning_effort = None;
    }
    if let Some(effort) = &evidence.effort {
        settings.reasoning_effort = Some(effort.clone());
    }
    if settings.model != before.model {
        changed.push("model");
    }
    if settings.reasoning_effort != before.reasoning_effort {
        changed.push("reasoning_effort");
    }
    if !changed.is_empty() {
        record.extra.insert(
            KEY.to_string(),
            json!({
                "model": settings.model,
                "reasoning_effort": settings.reasoning_effort,
                "runtime_id": evidence.launch_id,
                "provider_session_id": evidence.provider_session_id,
            }),
        );
    }
    if regenerate_title(record)? {
        changed.push("title");
    }
    if !changed.is_empty() {
        record.updated_at = jiff::Timestamp::now().to_string();
    }
    Ok(changed)
}

/// Re-render a structured, unpinned title from the session's current work
/// issues. A title that was typed or pinned, or whose stored state no longer
/// matches it, is not ours to rewrite.
fn regenerate_title(record: &mut SessionRecord) -> Result<bool, CliError> {
    if display_metadata::title_mode(record) == display_metadata::TitleMode::Pinned
        || crate::effective_session_title_state(record).is_none()
    {
        return Ok(false);
    }
    let (Some(work), Some(state)) = (&record.work, &record.title_state) else {
        return Ok(false);
    };
    let references = title_references(work);
    if references == state.references {
        return Ok(false);
    }
    let mut next = state.clone();
    next.references = references;
    let Ok(Some(title)) = crate::render_session_title_state(&next) else {
        return Ok(false);
    };
    let text_changed = record.title.as_deref() != Some(title.as_str());
    record.title = Some(title);
    record.title_state = Some(next);
    record.title_revision = record.title_revision.checked_add(1).ok_or_else(|| {
        CliError::data("title-revision-overflow", "title revision overflow", None)
    })?;
    Ok(text_changed)
}

/// Title references name an issue as `repo #N`, without the owner.
fn title_references(work: &SessionWork) -> Vec<String> {
    work.issues
        .iter()
        .take(MAX_TITLE_REFERENCES)
        .map(|issue| {
            let repo = issue.repository.rsplit('/').next().unwrap_or_default();
            format!("{} #{}", repo.to_ascii_lowercase(), issue.number)
        })
        .collect()
}

fn render_text(result: &Value) -> String {
    let id = result["session_id"].as_str().unwrap_or_default();
    let names = |key: &str| {
        result[key]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(", ")
    };
    let changed = names("changed");
    let mut text = if changed.is_empty() {
        format!("{id}: nothing changed\n")
    } else {
        format!("{id}: refreshed {changed}\n")
    };
    text.push_str(&format!("not re-detected: {}\n", names("unavailable")));
    text
}

#[cfg(test)]
#[path = "refresh_tests.rs"]
mod tests;
