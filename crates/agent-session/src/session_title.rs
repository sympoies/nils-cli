//! Authenticated, title-only writes using existing lineage and adoption records.
use std::path::PathBuf;

use clap::{Args, Subcommand};
use nils_common::cli_contract::OutputFormat;
use serde_json::{Value, json};

use crate::{CliContext, CliError, SessionRecord, coordination, display_metadata};

#[derive(Debug, Args)]
pub struct TitleArgs {
    #[command(subcommand)]
    pub command: TitleCommand,
}

#[derive(Debug, Subcommand)]
pub enum TitleCommand {
    /// Store a manual title and pin it against generated topics.
    Set(TitleSetArgs),
    /// Resume generated topics for a session.
    Auto(TitleTargetArgs),
}

#[derive(Debug, Args)]
pub struct TitleSetArgs {
    #[command(flatten)]
    pub target: TitleTargetArgs,
    /// Nonempty single-line title, at most 120 characters.
    pub title: String,
}

#[derive(Debug, Args)]
pub struct TitleTargetArgs {
    /// Exact target session ID.
    pub id: String,
    /// Expected title_revision from the current session view.
    #[arg(long)]
    pub if_revision: u64,
    /// Ordinary runtime capability; defaults to AGENT_SESSION_CAPABILITY_FILE.
    #[arg(long, value_name = "PATH", value_hint = clap::ValueHint::FilePath)]
    pub capability_file: Option<PathBuf>,
    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub format: OutputFormat,
}

pub(crate) fn target(args: &TitleArgs) -> &TitleTargetArgs {
    match &args.command {
        TitleCommand::Set(args) => &args.target,
        TitleCommand::Auto(args) => args,
    }
}

pub(crate) fn name(args: &TitleArgs) -> &'static str {
    match args.command {
        TitleCommand::Set(_) => "title-set",
        TitleCommand::Auto(_) => "title-auto",
    }
}

pub(crate) fn run(context: &CliContext, args: TitleArgs) -> i32 {
    let command = name(&args);
    let format = target(&args).format;
    let title = match &args.command {
        TitleCommand::Set(args) => Some(args.title.as_str()),
        TitleCommand::Auto(_) => None,
    };
    match update(context, target(&args), title) {
        Ok(value) => crate::render_single_success(command, format, &value, |value| {
            format!(
                "{}: {} (title revision {})\n",
                value["id"].as_str().unwrap_or_default(),
                value["title"].as_str().unwrap_or_default(),
                value["title_revision"]
            )
        }),
        Err(error) => crate::render_error(command, format, error),
    }
}

fn failure(code: &str, message: &str, next_action: &str) -> CliError {
    CliError::data(
        code,
        message,
        Some(json!({
            "retryable": false,
            "next_action": next_action,
            "recovery": {"action": next_action}
        })),
    )
}

pub(crate) fn update(
    context: &CliContext,
    args: &TitleTargetArgs,
    title: Option<&str>,
) -> Result<Value, CliError> {
    crate::validate_id(&args.id)?;
    let title = title
        .map(|title| normalize_manual_title(Some(title.to_owned()))?.ok_or_else(invalid_title))
        .transpose()?;
    let capability_file = args
        .capability_file
        .clone()
        .or_else(|| std::env::var_os("AGENT_SESSION_CAPABILITY_FILE").map(PathBuf::from));
    let (caller, incarnation) =
        coordination::authenticate_any_from_file(context, capability_file.as_deref())?;
    let mut authority =
        crate::lock_exact_session_authority(context, &args.id)?.ok_or_else(|| {
            failure(
                "session-not-found",
                "session does not exist",
                "inspect-session",
            )
        })?;
    // Recheck capability rotation/revocation under the registry lock before writing.
    let locked = coordination::lock_registry(context)?;
    let current_caller = if caller.id == authority.record.id {
        authority.record.clone()
    } else {
        crate::load_session_record(context, &caller.id)?
    };
    crate::ensure_same_session_identity(&caller, &current_caller)?;
    coordination::revalidate_capability_file(
        context,
        &locked.registry,
        &current_caller,
        &incarnation,
        capability_file.as_deref().ok_or_else(|| {
            failure(
                "title-forbidden",
                "a runtime capability is required",
                "authenticate-session",
            )
        })?,
    )?;
    let record = &mut authority.record;
    if !may_write(&current_caller, record) {
        return Err(failure(
            "title-forbidden",
            "only the session or its recorded supervisors may change its title",
            "ask-recorded-supervisor",
        ));
    }
    if record.title_revision != args.if_revision {
        let mut error = failure(
            "title-revision-conflict",
            "session title changed; read its revision and retry",
            "read-title-revision",
        );
        if let Some(details) = error.details_mut().as_mut() {
            details["current_revision"] = json!(record.title_revision);
            details["expected_revision"] = json!(args.if_revision);
        }
        return Err(error);
    }
    let next_title = record.title_revision.checked_add(1).ok_or_else(|| {
        failure(
            "title-revision-overflow",
            "title revision cannot advance",
            "inspect-session",
        )
    })?;
    let next_display = display_metadata::revision(record)
        .checked_add(1)
        .ok_or_else(|| {
            failure(
                "display-revision-overflow",
                "display revision cannot advance",
                "inspect-session",
            )
        })?;
    let mode = if let Some(title) = title {
        record.title = Some(title);
        display_metadata::TitleMode::Pinned
    } else {
        display_metadata::TitleMode::Auto
    };
    // Releasing a pin also releases a structured user-owned topic, which the
    // inference engine would otherwise preserve even in automatic mode.
    record.title_state = None;
    record.extra.insert("title_mode".into(), json!(mode));
    record
        .extra
        .insert("display_revision".into(), json!(next_display));
    record.title_revision = next_title;
    record.updated_at = jiff::Timestamp::now().to_string();
    crate::write_session_record(context, record)?;
    Ok(
        json!({"id": record.id, "title": record.title, "title_mode": mode,
        "title_revision": next_title, "display_revision": next_display}),
    )
}

fn may_write(caller: &SessionRecord, target: &SessionRecord) -> bool {
    if caller.id == target.id && caller.created_at == target.created_at {
        return true;
    }
    let Some(machine) = caller
        .lineage
        .as_ref()
        .map(|l| l.machine.as_str())
        .filter(|m| !m.is_empty())
    else {
        return false;
    };
    let matches = |reference: &crate::lineage::SessionRef| {
        reference.session_id == caller.id
            && reference.session_created_at == caller.created_at
            && reference.machine == machine
    };
    target
        .lineage
        .as_ref()
        .is_some_and(|l| l.parent.as_ref().is_some_and(matches) || matches(&l.root))
        || target
            .lineage_adoption
            .as_ref()
            .and_then(|a| a.adopted_by.as_ref())
            .is_some_and(matches)
}

fn invalid_title() -> CliError {
    CliError::usage(
        "title-invalid",
        "session title must be a nonempty single line without control characters",
        None,
    )
}

/// Keep the existing operator title API's normalization unchanged.
pub(crate) fn normalize_title(title: Option<String>) -> Result<Option<String>, CliError> {
    let Some(title) = title else {
        return Ok(None);
    };
    let title = title.trim();
    if title.is_empty() {
        return Ok(None);
    }
    if title.chars().count() > 120 {
        return Err(CliError::usage(
            "title-too-long",
            "session title must be 120 characters or fewer",
            Some(json!({"max_chars":120})),
        ));
    }
    Ok(Some(title.to_owned()))
}

/// Start-time plain titles and managed title writes share the same validation.
pub(crate) fn normalize_manual_title(title: Option<String>) -> Result<Option<String>, CliError> {
    if title.as_deref().is_some_and(|title| {
        title.trim().is_empty()
            || title
                .chars()
                .any(|c| c.is_control() || matches!(c, '\u{2028}' | '\u{2029}'))
    }) {
        return Err(invalid_title());
    }
    normalize_title(title)
}
