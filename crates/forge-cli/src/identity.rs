//! CLI adapter and explicit invocation scope for the shared identity resolver.
use crate::backend::{BackendCall, BackendProgram};
use crate::cli::{
    ActivityArgs, ActivityCommand, BINARY, Cli, Command, GlobalFlags, IdentityCommand, RepoArgs,
    RepoCommand, SearchArgs,
};
use crate::envelope::emit_success;
use crate::error::ForgeError;
use crate::provider::{Provider, detect, detect_unscoped, git_remote_url};
use nils_common::cli_contract::{OutputFormat, schema_version_for};
use nils_common::forge_identity::{self as identity, Authorization, Operation, Target};
use serde::Serialize;
use std::cell::RefCell;

thread_local! { static SCOPE: RefCell<Option<(Target, Operation)>> = const { RefCell::new(None) }; }
pub struct Scope(Option<(Target, Operation)>);
pub(crate) fn current_scope() -> Option<(Target, Operation)> {
    SCOPE.with(|s| s.borrow().clone())
}
pub(crate) fn enter_scope(scope: Option<(Target, Operation)>) -> Scope {
    Scope(SCOPE.with(|s| s.replace(scope)))
}
impl Drop for Scope {
    fn drop(&mut self) {
        SCOPE.with(|s| *s.borrow_mut() = self.0.take());
    }
}
fn error(e: identity::Error) -> ForgeError {
    if matches!(
        e.code,
        "identity_probe_timeout" | "identity_probe_output_limit"
    ) {
        return ForgeError::unavailable(
            schema_version_for(BINARY, "identity", 1),
            e.code,
            e.to_string(),
            None,
        );
    }
    ForgeError::validation(
        schema_version_for(BINARY, "identity", 1),
        e.code,
        e.to_string(),
        None,
    )
}
fn target_for_context(
    provider: Provider,
    host: &str,
    repo: &str,
    policy: &identity::LoadedPolicy,
) -> identity::Result<Target> {
    match provider {
        Provider::GitHub => Target::new(host, repo),
        Provider::GitLab
            if host.eq_ignore_ascii_case("gitlab.com")
                || policy
                    .policy
                    .gitlab_hosts
                    .iter()
                    .any(|configured| configured.eq_ignore_ascii_case(host)) =>
        {
            Target::new_gitlab(host, repo)
        }
        Provider::GitLab => Err(identity::Error::new(
            "identity_gitlab_host_not_configured_add_gitlab_hosts",
        )),
        Provider::Local => Err(identity::Error::new("identity_provider_unsupported")),
    }
}
pub fn scope(cli: &Cli, global: &GlobalFlags) -> Result<Scope, ForgeError> {
    let none = || Scope(None);
    if matches!(
        cli.command,
        Some(
            Command::Completion(_)
                | Command::Provider(_)
                | Command::Identity(_)
                | Command::OperationEffect(_)
        ) | None
    ) || global.is_local()
    {
        return Ok(none());
    }
    let (_, _, effect, _) = crate::operation_effect::classify(cli);
    if !matches!(
        effect,
        nils_common::execution_effect::ProviderEffect::NetworkRead
            | nils_common::execution_effect::ProviderEffect::NetworkWrite
    ) {
        return Ok(none());
    }
    let Some(policy) = identity::load().map_err(error)? else {
        return Ok(none());
    };
    let cross_repository = matches!(
        &cli.command,
        Some(Command::Inbox(_))
            | Some(Command::Activity(ActivityArgs {
                command: Some(
                    ActivityCommand::Commits(_)
                        | ActivityCommand::Events(_)
                        | ActivityCommand::Summary(_)
                )
            }))
    ) && global.repo.is_none()
        || matches!(&cli.command, Some(Command::Search(SearchArgs { command: Some(command) })) if global.repo.is_none() && crate::ops::search::query_scoped(command));
    let target = if cross_repository {
        let host = if matches!(&cli.command, Some(Command::Inbox(_))) {
            if global
                .provider
                .as_ref()
                .is_some_and(|p| !matches!(p, crate::cli::ProviderFlag::Github))
            {
                return Err(error(identity::Error::new("identity_provider_unsupported")));
            }
            crate::ops::inbox::github_host(global)
        } else {
            let ctx =
                detect_unscoped(global.provider_hint(), &global.remote, None, git_remote_url)?;
            if ctx.provider != Provider::GitHub {
                return Err(error(identity::Error::new("identity_provider_unsupported")));
            }
            ctx.host
        };
        let target = Target::cross_repository(&host).map_err(error)?;
        // Resolve metadata before inbox fan-out so ambiguity remains a typed
        // invocation error, rather than a provider-local query failure.
        policy
            .select(&target, None, Operation::ApiRead)
            .map_err(error)?;
        target
    } else {
        let ctx = detect(
            global.provider_hint(),
            &global.remote,
            global.repo.as_deref(),
            git_remote_url,
        )?;
        if ctx.provider != Provider::GitHub {
            return Err(error(identity::Error::new("identity_provider_unsupported")));
        }
        Target::new(
            &ctx.host,
            ctx.repo
                .as_deref()
                .ok_or_else(|| error(identity::Error::new("identity_target_unknown")))?,
        )
        .map_err(error)?
    };
    let op = if effect == nils_common::execution_effect::ProviderEffect::NetworkWrite {
        Operation::ApiWrite
    } else {
        Operation::ApiRead
    };
    if matches!(
        &cli.command,
        Some(Command::Repo(RepoArgs {
            command: Some(RepoCommand::Bootstrap(_))
        }))
    ) {
        policy
            .select(&target, None, Operation::GitPush)
            .map_err(error)?;
        let commit = policy
            .select(&target, None, Operation::Commit)
            .map_err(error)?;
        identity::verify_key(&commit.profile).map_err(error)?;
    }
    Ok(enter_scope(Some((target, op))))
}
fn target_from_call(call: &BackendCall) -> identity::Result<Option<Target>> {
    let args: Vec<_> = call.argv.iter().filter_map(|s| s.to_str()).collect();
    let host = call.resolved_host().unwrap_or("github.com");
    let mut candidates = Vec::new();
    for pair in args.windows(2) {
        if matches!(pair[0], "--repo" | "-R") {
            let parts: Vec<_> = pair[1].split('/').collect();
            let (host, repo) = match parts.len() {
                2 => (host, pair[1].to_string()),
                3 => (parts[0], format!("{}/{}", parts[1], parts[2])),
                _ => return Err(identity::Error::new("identity_target_invalid")),
            };
            candidates.push(Target::new(host, &repo)?);
        }
    }
    for arg in args.iter().filter(|_| args.first() == Some(&"api")) {
        if let Some(path) = arg.strip_prefix("repos/") {
            let parts: Vec<_> = path.split('/').collect();
            if parts.len() >= 2 {
                candidates.push(Target::new(
                    host,
                    &format!("{}/{}", parts[0], parts[1].split('?').next().unwrap()),
                )?);
            }
        }
    }
    let owner = args.iter().find_map(|a| a.strip_prefix("owner="));
    let name = args.iter().find_map(|a| a.strip_prefix("name="));
    if let (Some(owner), Some(name)) = (owner, name) {
        candidates.push(Target::new(host, &format!("{owner}/{name}"))?);
    }
    if candidates.windows(2).any(|p| p[0] != p[1]) {
        return Err(identity::Error::new("identity_target_ambiguous"));
    }
    Ok(candidates.pop())
}
pub fn prepare_api(
    call: &BackendCall,
    cmd: &mut std::process::Command,
    deadline: Option<std::time::Instant>,
) -> Result<Option<Authorization>, ForgeError> {
    identity::with_probe_deadline(deadline, || prepare_api_inner(call, cmd))
}
fn prepare_api_inner(
    call: &BackendCall,
    cmd: &mut std::process::Command,
) -> Result<Option<Authorization>, ForgeError> {
    let Some(policy) = identity::load().map_err(error)? else {
        return Ok(None);
    };
    if call.program != BackendProgram::Gh {
        return Err(error(identity::Error::new("identity_provider_unsupported")));
    }
    let scope = current_scope();
    let own = target_from_call(call).map_err(error)?;
    if let (Some(own), Some((scoped, _))) = (&own, &scope)
        && own != scoped
    {
        return Err(error(identity::Error::new("identity_target_ambiguous")));
    }
    let target = own
        .or_else(|| scope.as_ref().map(|(t, _)| t.clone()))
        .ok_or_else(|| error(identity::Error::new("identity_target_unknown")))?;
    if target.host != call.resolved_host().unwrap_or("github.com") {
        return Err(error(identity::Error::new("identity_target_ambiguous")));
    }
    let operation = scope.map(|(_, o)| o).unwrap_or(Operation::ApiWrite);
    let path = identity::managed_path_optional(None).map_err(error)?;
    let auth = policy
        .authorize(
            &target,
            path.as_deref(),
            operation,
            &call.program.executable(),
        )
        .map_err(error)?;
    auth.apply_api(cmd);
    Ok(Some(auth))
}
#[derive(Serialize)]
struct Explanation {
    enforced: bool,
    selection: Option<identity::Selection>,
    credential_verified: bool,
    signing_key_verified: bool,
}
pub fn run(
    global: &GlobalFlags,
    command: IdentityCommand,
    format: OutputFormat,
    remote_explicit: bool,
) -> Result<i32, ForgeError> {
    identity::with_probe_deadline(None, || run_inner(global, command, format, remote_explicit))
}
fn run_inner(
    global: &GlobalFlags,
    command: IdentityCommand,
    format: OutputFormat,
    remote_explicit: bool,
) -> Result<i32, ForgeError> {
    let (doctor, operation) = match command {
        IdentityCommand::Explain { operation } => (false, operation.operation()),
        IdentityCommand::Doctor { operation } => (true, operation.operation()),
    };
    let policy = identity::load().map_err(error)?;
    let mut payload = Explanation {
        enforced: policy.is_some(),
        selection: None,
        credential_verified: false,
        signing_key_verified: false,
    };
    if let Some(policy) = policy {
        let (target, provider) = if global.repo.is_none()
            && matches!(
                operation,
                Operation::GitRead | Operation::GitPush | Operation::Commit
            ) {
            let remote = if operation == Operation::Commit && !remote_explicit {
                identity::authoring_remote(None).map_err(error)?
            } else {
                global.remote.clone()
            };
            let (target, url) =
                identity::target_for_remote(None, &remote, operation != Operation::GitRead)
                    .map_err(error)?;
            let ctx = detect(global.provider_hint(), &remote, None, |_| Some(url.clone()))?;
            if ctx.provider == Provider::Local {
                return Err(error(identity::Error::new("identity_provider_unsupported")));
            }
            if ctx.host != target.host {
                return Err(error(identity::Error::new("identity_target_ambiguous")));
            }
            (target, ctx.provider)
        } else {
            let ctx = detect(
                global.provider_hint(),
                &global.remote,
                global.repo.as_deref(),
                git_remote_url,
            )?;
            let target = target_for_context(
                ctx.provider,
                &ctx.host,
                ctx.repo
                    .as_deref()
                    .ok_or_else(|| error(identity::Error::new("identity_target_unknown")))?,
                &policy,
            )
            .map_err(error)?;
            (target, ctx.provider)
        };
        if doctor && provider != Provider::GitHub {
            return Err(error(identity::Error::new("identity_provider_unsupported")));
        }
        let path = identity::managed_path_optional(None).map_err(error)?;
        let selection = policy
            .select(&target, path.as_deref(), operation)
            .map_err(error)?;
        if doctor {
            let _ = policy
                .authorize(
                    &target,
                    path.as_deref(),
                    operation,
                    &BackendProgram::Gh.executable(),
                )
                .map_err(error)?;
            if let Err(e) = identity::verify_key(&selection.profile) {
                policy
                    .audit(Some(&selection), &target, operation, e.code, None)
                    .map_err(error)?;
                return Err(error(e));
            }
            payload.credential_verified = true;
            payload.signing_key_verified = true;
        }
        payload.selection = Some(selection);
    }
    Ok(emit_success(
        schema_version_for(
            BINARY,
            if doctor {
                "identity.doctor"
            } else {
                "identity.explain"
            },
            1,
        ),
        payload,
        format,
        |p| {
            if let Some(s) = &p.selection {
                println!(
                    "principal={} rule={} profile={} target={}",
                    s.principal,
                    s.matched_rule,
                    s.profile_id,
                    s.target.key()
                );
            } else {
                println!("identity policy absent; existing behavior applies");
            }
            if doctor && p.enforced {
                println!("credential and signing key verified");
            }
        },
    ))
}
