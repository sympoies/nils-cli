//! Source-side identity and portable history discovery.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use nils_provider_resume::{
    ClaudeResumeScanBudget, CodexResumeScanBudget, ProviderHistoryMatch,
    collect_claude_named_resume_matches, collect_codex_provider_resume_matches,
};
use serde::{Deserialize, Serialize};

use crate::{CliContext, CliError, SessionRecord, claude_account, codex_account};

use super::{HistoryRole, Provider, Refusal, canonical_uuid, host_identity, sha256};

#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Plan {
    pub(super) source_host: String,
    pub(super) destination_host: String,
    pub(super) source_session_id: String,
    pub(super) source_incarnation: String,
    pub(super) source_generation: u64,
    pub(super) provider: Provider,
    pub(super) provider_session_id: String,
    pub(super) account: String,
    pub(super) account_binding_revision: u64,
    pub(super) source_checkout_sha256: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) git_checkout: Option<GitCheckout>,
    pub(super) history: HistoryPlacement,
}

#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct GitCheckout {
    pub(super) repository_sha256: String,
    pub(super) head: String,
    pub(super) relative_cwd: String,
}

#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct HistoryPlacement {
    pub(super) role: HistoryRole,
    pub(super) relative_path: String,
    pub(super) source_relative_path: String,
    pub(super) bytes: u64,
}

pub(super) struct Source {
    pub(super) plan: Plan,
    pub(super) history_path: PathBuf,
    pub(super) provider_root: PathBuf,
}

pub(crate) fn plan(
    context: &CliContext,
    id: &str,
    incarnation: &str,
    destination: &str,
) -> Result<Plan, CliError> {
    inspect(context, id, incarnation, &local_host()?, destination).map(|source| source.plan)
}

pub(super) fn inspect(
    context: &CliContext,
    id: &str,
    incarnation: &str,
    source_host: &str,
    destination: &str,
) -> Result<Source, CliError> {
    if id.len() > 128 || crate::validate_id(id).is_err() || !canonical_uuid(incarnation) {
        return Err(super::cli_refusal(Refusal::SessionMismatch));
    }
    if !host_identity(source_host) || !host_identity(destination) || destination == source_host {
        return Err(super::cli_refusal(Refusal::TargetMismatch));
    }
    let record = crate::load_session_record(context, id)
        .map_err(|_| super::cli_refusal(Refusal::SessionMismatch))?;
    let runtime = record
        .runtime
        .as_ref()
        .filter(|runtime| {
            record.id == id && runtime.launch_id == incarnation && runtime.generation > 0
        })
        .ok_or_else(|| super::cli_refusal(Refusal::SessionMismatch))?;
    let (provider, account, revision, root) = binding(&record)?;
    let conversation = record
        .provider_resume
        .as_ref()
        .filter(|identity| {
            identity.provider == record.agent && canonical_uuid(&identity.session_id)
        })
        .ok_or_else(|| super::cli_refusal(Refusal::SessionMismatch))?;
    let root = fs::canonicalize(root).map_err(|_| history_missing())?;
    let found = find_history(provider, &root, &conversation.session_id)?;
    let cwd = crate::resolve_cwd(Some(Path::new(&record.cwd)))
        .map_err(|_| super::cli_refusal(Refusal::PathMismatch))?;
    if found.cwd != cwd {
        return Err(super::cli_refusal(Refusal::PathMismatch));
    }
    let cwd_text = cwd
        .to_str()
        .ok_or_else(|| super::cli_refusal(Refusal::PathMismatch))?;
    let (history, history_path) = describe_history(provider, &root, &found.path)?;
    let plan = Plan {
        source_host: source_host.to_owned(),
        destination_host: destination.to_owned(),
        source_session_id: record.id,
        source_incarnation: runtime.launch_id.clone(),
        source_generation: runtime.generation,
        provider,
        provider_session_id: conversation.session_id.clone(),
        account,
        account_binding_revision: revision,
        source_checkout_sha256: sha256(cwd_text.as_bytes()),
        git_checkout: git_checkout(&cwd),
        history,
    };
    Ok(Source {
        plan,
        history_path,
        provider_root: root,
    })
}

pub(super) fn local_host() -> Result<String, CliError> {
    crate::non_empty_env("AGENT_SESSION_RESUME_MOVE_HOST")
        .filter(|host| host_identity(host))
        .ok_or_else(|| super::cli_refusal(Refusal::TrustUnavailable))
}

pub(super) fn binding(
    record: &SessionRecord,
) -> Result<(Provider, String, u64, PathBuf), CliError> {
    let unknown = || crate::start_resume::refusal("resume-source-unknown");
    match record.agent.as_str() {
        "codex" => {
            if !matches!(
                codex_account::next_transition_state(record),
                codex_account::NextTransitionState::Absent
            ) {
                return Err(unknown());
            }
            let codex_account::BindingSnapshot::Bound { account, revision } =
                codex_account::binding_snapshot(record)
            else {
                return Err(unknown());
            };
            let root = crate::session_provider_config_dir(record)
                .or_else(|| {
                    nils_provider_resume::codex_sessions_root()
                        .and_then(|root| root.parent().map(Path::to_path_buf))
                })
                .ok_or_else(history_missing)?;
            Ok((Provider::Codex, account, revision, root))
        }
        "claude" => {
            if claude_account::abandon_next_for_resume(&mut record.clone())
                .map_err(|_| unknown())?
            {
                return Err(unknown());
            }
            let view = claude_account::view_for_record(record)
                .filter(|view| view.state == "bound")
                .ok_or_else(unknown)?;
            let account = view.selected_account.ok_or_else(unknown)?;
            let root = claude_account::config_dir_for_runtime(record).ok_or_else(unknown)?;
            Ok((Provider::Claude, account, view.revision, root))
        }
        _ => Err(crate::start_resume::refusal("resume-unsupported")),
    }
}

pub(super) fn find_history(
    provider: Provider,
    root: &Path,
    thread: &str,
) -> Result<ProviderHistoryMatch, CliError> {
    let mut matches = BTreeSet::new();
    let truncated = match provider {
        Provider::Codex => {
            let mut budget = CodexResumeScanBudget::from_env();
            collect_codex_provider_resume_matches(
                &root.join("sessions"),
                0,
                thread,
                &mut matches,
                &mut budget,
            );
            budget.truncated
        }
        Provider::Claude => {
            let mut budget = ClaudeResumeScanBudget::from_env();
            collect_claude_named_resume_matches(
                &root.join("projects"),
                thread,
                &mut matches,
                &mut budget,
            );
            budget.truncated
        }
    };
    if truncated || matches.len() > 1 {
        return Err(crate::start_resume::refusal("resume-source-unknown"));
    }
    matches.pop_first().ok_or_else(history_missing)
}

fn describe_history(
    provider: Provider,
    root: &Path,
    path: &Path,
) -> Result<(HistoryPlacement, PathBuf), CliError> {
    let unsupported = || super::cli_refusal(Refusal::HistoryUnsupported);
    let path_on_disk = fs::canonicalize(path).map_err(|_| history_missing())?;
    let expected = match provider {
        Provider::Codex => path.to_path_buf(),
        Provider::Claude => {
            let projects = root.join("projects");
            let relative = path.strip_prefix(&projects).map_err(|_| unsupported())?;
            let projects_on_disk = fs::canonicalize(&projects).map_err(|_| history_missing())?;
            projects_on_disk.join(relative)
        }
    };
    if path_on_disk != expected {
        return Err(unsupported());
    }
    let source_relative_path = path
        .strip_prefix(root)
        .ok()
        .and_then(Path::to_str)
        .ok_or_else(unsupported)?;
    let (role, relative_path) = match provider {
        Provider::Codex => (HistoryRole::CodexRollout, source_relative_path),
        Provider::Claude => (
            HistoryRole::ClaudeTranscript,
            path.file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(unsupported)?,
        ),
    };
    let metadata = fs::symlink_metadata(&path_on_disk).map_err(|_| history_missing())?;
    if !metadata.is_file() || metadata.len() == 0 {
        return Err(unsupported());
    }
    Ok((
        HistoryPlacement {
            role,
            relative_path: relative_path.to_owned(),
            source_relative_path: source_relative_path.to_owned(),
            bytes: metadata.len(),
        },
        path_on_disk,
    ))
}

pub(super) fn git_checkout(cwd: &Path) -> Option<GitCheckout> {
    use nils_common::git;
    let root = fs::canonicalize(git::repo_root_in(cwd).ok()??).ok()?;
    let relative_cwd = cwd.strip_prefix(root).ok()?.to_str()?.to_owned();
    let head = git::rev_parse_in(cwd, &["--verify", "HEAD"]).ok()??;
    let origin = git::run_output_in(cwd, &["config", "--get", "remote.origin.url"]).ok()?;
    if !origin.status.success() {
        return None;
    }
    let origin = String::from_utf8(origin.stdout).ok()?;
    let repository =
        git::repository_identity_from_remote_url(origin.trim().split(['?', '#']).next()?)?;
    Some(GitCheckout {
        repository_sha256: sha256(repository.as_bytes()),
        head,
        relative_cwd,
    })
}

fn history_missing() -> CliError {
    crate::start_resume::refusal("resume-history-missing")
}

#[cfg(test)]
mod claude_layout_tests {
    use std::fs;
    use std::os::unix::fs::symlink;

    use pretty_assertions::assert_eq;

    use super::{Provider, describe_history};

    #[test]
    fn claude_source_manifest_keeps_the_account_project_route() {
        let fixture = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(fixture.path()).unwrap();
        let account = root.join("account");
        let shared = root.join("shared");
        fs::create_dir(&account).unwrap();
        fs::create_dir_all(shared.join("project")).unwrap();
        symlink(&shared, account.join("projects")).unwrap();
        let transcript = shared.join("project/session.jsonl");
        fs::write(&transcript, b"synthetic history").unwrap();

        let route = account.join("projects/project/session.jsonl");
        let (placement, bound_path) = describe_history(Provider::Claude, &account, &route).unwrap();
        assert_eq!(
            placement.source_relative_path,
            "projects/project/session.jsonl"
        );
        assert_eq!(placement.relative_path, "session.jsonl");
        assert_eq!(bound_path, transcript);

        let redirected = shared.join("project/redirected.jsonl");
        symlink(&transcript, &redirected).unwrap();
        let route = account.join("projects/project/redirected.jsonl");
        assert!(describe_history(Provider::Claude, &account, &route).is_err());
    }
}
