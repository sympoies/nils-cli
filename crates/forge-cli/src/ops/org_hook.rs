//! `org hook list|create|set-url|set-active|batch` — GitHub organization
//! webhooks (an organization owner token with `admin:org_hook` is required).
//!
//! Spec / ops: `cli.forge-cli.org.hook.<action>.v1`. The engine lives in
//! [`super::webhook`]; this module resolves the organization and the command.
use nils_common::cli_contract::{OutputFormat, schema_version_for};

use super::webhook::{Action, HookTarget, Kind, create, execute};
use crate::cli::{BINARY, GlobalFlags, OrgArgs, OrgCommand, OrgHookCommand};
use crate::error::ForgeError;
use crate::provider::{Provider, detect_unscoped, git_remote_url};

pub fn run(global: &GlobalFlags, args: OrgArgs, format: OutputFormat) -> Result<i32, ForgeError> {
    let OrgCommand::Hook(hook) = args.command;
    let refuse = |kind: &'static str, message: &'static str| {
        ForgeError::validation(schema_version_for(BINARY, "error", 1), kind, message, None)
    };
    let ctx = detect_unscoped(global.provider_hint(), &global.remote, None, git_remote_url)?;
    if ctx.provider != Provider::GitHub {
        return Err(ForgeError::provider_unsupported(
            schema_version_for(BINARY, "error", 1),
            "organization webhooks are GitHub-only",
            None,
        ));
    }
    if let OrgHookCommand::Batch(batch) = hook.command {
        if args.org.is_some() {
            return Err(refuse(
                "org_conflict",
                "--file rows name their own organizations; omit --org",
            ));
        }
        return execute(
            global,
            &ctx,
            Kind::Org,
            None,
            Action::Batch(batch.file),
            format,
        );
    }
    let org = args
        .org
        .ok_or_else(|| refuse("org_required", "--org is required"))?;
    let action = match hook.command {
        OrgHookCommand::Create(create_args) => {
            return create(global, &ctx, &org, create_args, format);
        }
        OrgHookCommand::List => Action::List,
        OrgHookCommand::Redeliver(args) => Action::Redeliver(args),
        OrgHookCommand::SetUrl(set) => Action::SetUrl {
            hook: set.hook,
            from: set.from,
            to: set.to,
        },
        OrgHookCommand::SetActive(set) => Action::SetActive {
            hook: set.hook,
            to: set.active,
        },
        OrgHookCommand::Batch(_) => unreachable!("handled above"),
    };
    execute(
        global,
        &ctx,
        Kind::Org,
        Some(HookTarget::Org(org)),
        action,
        format,
    )
}
