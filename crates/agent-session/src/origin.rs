//! Descriptive launch origin and inherited workspace, independent of authority.
use clap::{Args, Subcommand};
use nils_common::cli_contract::OutputFormat;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{CliContext, CliError, SessionRecord};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Parent {
    pub session_id: String,
    pub machine: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Origin {
    pub parent: Option<Parent>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct Fields {
    #[serde(default)]
    pub origin: Origin,
    pub workspace: Option<String>,
}

impl Fields {
    pub(crate) fn of(record: &SessionRecord) -> Self {
        Self {
            origin: record
                .extra
                .get("origin")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or_default(),
            workspace: record
                .extra
                .get("workspace")
                .and_then(Value::as_str)
                .map(str::to_owned),
        }
    }

    pub(crate) fn validate(&self) -> Result<(), CliError> {
        if let Some(parent) = &self.origin.parent {
            crate::validate_id(&parent.session_id).map_err(|_| invalid_origin())?;
            if parent.machine.is_empty()
                || parent.machine.len() > 64
                || !parent
                    .machine
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
            {
                return Err(invalid_origin());
            }
        }
        if let Some(workspace) = &self.workspace {
            validate_workspace(workspace)?;
        }
        Ok(())
    }

    pub(crate) fn store(&self, record: &mut SessionRecord) {
        record.extra.insert("origin".into(), json!(self.origin));
        record
            .extra
            .insert("workspace".into(), json!(self.workspace));
    }
}

pub(crate) fn invalid_origin() -> CliError {
    CliError::usage(
        "origin-invalid",
        "origin.parent requires a valid session_id and a bounded machine identifier",
        Some(
            json!({"recovery":{"action":"correct-request", "hint":"Use only the managed caller's session_id and machine; omit origin when no caller is known."}}),
        ),
    )
}

pub(crate) fn validate_workspace(workspace: &str) -> Result<(), CliError> {
    crate::metadata::validate_value("topic", workspace).map_err(|_| invalid_workspace())
}

pub(crate) fn invalid_workspace() -> CliError {
    CliError::usage(
        "workspace-invalid",
        "workspace must be a public lowercase slug of 1-64 bytes with single hyphen or dot separators and alphanumeric ends",
        Some(
            json!({"recovery":{"action":"correct-request", "command":"agent-session start --agent codex --workspace example-workspace", "hint":"Use a non-secret slug, for example example-workspace; paths are forbidden."}}),
        ),
    )
}

/// The environment identifies the caller, even if its record is on another
/// machine. It describes provenance only; it is never authentication.
pub(crate) fn cli(context: &CliContext) -> Fields {
    let Some(id) =
        crate::non_empty_env("AGENT_SESSION_ID").filter(|id| crate::validate_id(id).is_ok())
    else {
        return Fields::default();
    };
    let parent = crate::load_session_record(context, &id).ok();
    let machine = crate::non_empty_env("AGENT_SESSION_MACHINE")
        .or_else(|| {
            parent
                .as_ref()
                .and_then(|r| r.lineage.as_ref())
                .map(|l| l.machine.clone())
                .filter(|s| !s.is_empty())
        })
        .unwrap_or_else(|| crate::board::machine_identity(None, context));
    let fields = Fields {
        origin: Origin {
            parent: Some(Parent {
                session_id: id,
                machine,
            }),
        },
        workspace: parent.as_ref().and_then(|r| Fields::of(r).workspace),
    };
    if fields.validate().is_ok() {
        fields
    } else {
        Fields::default()
    }
}

pub(crate) fn for_start(
    context: &CliContext,
    mut initial: Option<crate::InitialLineage>,
    workspace: Option<&str>,
) -> Result<Option<crate::InitialLineage>, CliError> {
    if let Some(workspace) = workspace {
        validate_workspace(workspace)?;
        let initial = initial.get_or_insert_with(|| crate::InitialLineage {
            seed: crate::lineage::LineageSeed::root(
                &crate::board::machine_identity(None, context),
                crate::lineage::STARTER_OPERATOR,
                crate::lineage::VIA_CLI,
            ),
            origin: Fields::default(),
            work: None,
            role: None,
        });
        initial.origin.workspace = Some(workspace.to_owned());
    }
    Ok(initial)
}

#[derive(Debug, Args)]
pub struct WorkspaceArgs {
    #[command(subcommand)]
    pub command: WorkspaceCommand,
}

#[derive(Debug, Subcommand)]
pub enum WorkspaceCommand {
    /// Assign a public workspace slug without restarting the session.
    Set(WorkspaceSetArgs),
}

#[derive(Debug, Args)]
pub struct WorkspaceSetArgs {
    pub id: String,
    pub slug: String,
    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub format: OutputFormat,
}

pub(crate) fn format(args: &WorkspaceArgs) -> OutputFormat {
    match &args.command {
        WorkspaceCommand::Set(args) => args.format,
    }
}

pub(crate) fn run(context: &CliContext, args: WorkspaceArgs) -> i32 {
    let WorkspaceCommand::Set(args) = args.command;
    let result = (|| {
        crate::validate_id(&args.id)?;
        validate_workspace(&args.slug)?;
        let _lock = crate::acquire_session_record_lock(context, &args.id)?;
        let mut record = crate::load_session_record(context, &args.id)?;
        let fields = Fields {
            workspace: Some(args.slug),
            ..Fields::of(&record)
        };
        fields.store(&mut record);
        record.updated_at = jiff::Timestamp::now().to_string();
        crate::write_session_record(context, &record)?;
        Ok::<_, CliError>(
            json!({"id":record.id, "origin":fields.origin, "workspace":fields.workspace}),
        )
    })();
    match result {
        Ok(value) => crate::render_single_success("workspace-set", args.format, &value, |v| {
            format!(
                "{}: {}",
                v["id"].as_str().unwrap_or_default(),
                v["workspace"].as_str().unwrap_or_default()
            )
        }),
        Err(error) => crate::render_error("workspace-set", args.format, error),
    }
}

#[cfg(test)]
mod tests {
    use super::validate_workspace;
    use pretty_assertions::assert_eq;

    #[test]
    fn workspace_uses_public_metadata_slug_bounds_and_redacted_fixit_errors() {
        for valid in ["a", "0", "team-1", "project.alpha", &"a".repeat(64)] {
            validate_workspace(valid).unwrap();
        }
        for invalid in [
            "",
            "A",
            "a_b",
            "a..b",
            "a.-b",
            "-a",
            "a.",
            "private/path",
            "sk-example",
            &"a".repeat(65),
        ] {
            let error = validate_workspace(invalid).unwrap_err().into_inner();
            assert_eq!(error.code, "workspace-invalid");
            assert_eq!(error.exit_code, 64);
            assert!(!error.message.contains("private/path"));
        }
    }
}
