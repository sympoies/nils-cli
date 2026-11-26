//! GitHub-only view, editing and guarded deletion of exact releases.
mod api;
mod delete;
mod delete_asset;
mod edit;
mod guards;
mod lookup;
mod model;

use super::super::github_write::{self as write, invalid};
use crate::backend::{BackendCall, DryRunPayload};
use crate::cli::{GlobalFlags, ReleaseCommand};
use crate::envelope::emit_success;
use crate::error::ForgeError;
use crate::provider::ProviderContext;
use nils_common::cli_contract::{OutputFormat, schema_version_for};
use serde::Serialize;

pub(super) fn release_path(repo: &str, tag: &str) -> String {
    format!("repos/{repo}/releases/tags/{}", guards::encode(tag))
}
#[derive(Serialize)]
struct Plan {
    #[serde(flatten)]
    first: DryRunPayload,
    follow_up: Vec<DryRunPayload>,
    #[serde(skip_serializing_if = "Option::is_none")]
    draft: Option<bool>,
}
fn dry_run(
    ctx: &ProviderContext,
    op: &str,
    calls: Vec<BackendCall>,
    draft: Option<bool>,
    format: OutputFormat,
) -> i32 {
    let mut calls = calls.iter();
    emit_success(
        schema_version_for("forge-cli", op, 1),
        Plan {
            first: DryRunPayload::new(ctx.provider, calls.next().expect("release lookup")),
            follow_up: calls
                .map(|call| DryRunPayload::new(ctx.provider, call))
                .collect(),
            draft,
        },
        format,
        |p| {
            if let Some(draft) = p.draft {
                println!("would set draft to {draft} if needed");
            }
            println!("would run: {}", p.first.plan.join(" "));
            for step in &p.follow_up {
                println!("then: {}", step.plan.join(" "));
            }
        },
    )
}
pub(super) fn run(
    global: &GlobalFlags,
    command: ReleaseCommand,
    format: OutputFormat,
) -> Result<i32, ForgeError> {
    if let ReleaseCommand::DeleteAsset(args) = command {
        return delete_asset::run(global, args, format);
    }
    let (op, tag) = match &command {
        ReleaseCommand::View(args) => ("release.view", &args.tag),
        ReleaseCommand::Edit(args) => (edit::OP, &args.tag),
        ReleaseCommand::Delete(args) => ("release.delete", &args.tag),
        _ => unreachable!("create/upload have their existing owner"),
    };
    guards::tag(tag, op)?;
    if let ReleaseCommand::Delete(args) = &command
        && guards::version_tag(tag)
        && !args.allow_version_tag
    {
        return Err(invalid(
            op,
            "release_version_tag_protected",
            "canonical SemVer release tags require explicit --allow-version-tag; --cleanup-tag separately permits tag removal",
        ));
    }
    let ctx = write::target(global, op)?;
    let repo = ctx.repo.as_deref().expect("resolved repository");
    let changes = match &command {
        ReleaseCommand::Edit(args) => Some(edit::Changes::new(args)?),
        _ => None,
    };
    let lookup = release_path(repo, tag);
    if global.dry_run {
        let payload = changes
            .as_ref()
            .map(edit::Changes::payload_file)
            .transpose()?;
        let mut calls = vec![
            api::call(&ctx, &lookup, "GET"),
            lookup::draft_call(&ctx, repo, tag),
            api::call(
                &ctx,
                &format!("repos/{repo}/releases/<draft-release-id-if-needed>"),
                "GET",
            ),
            api::call(&ctx, &model::tag_path(repo, tag), "GET"),
        ];
        match &command {
            ReleaseCommand::View(_) | ReleaseCommand::Edit(_) => {
                // The next read depends on whether the exact ref is an
                // annotated tag or an as-yet untagged draft.
                calls.extend([
                    api::call(
                        &ctx,
                        &format!("repos/{repo}/git/tags/<annotated-tag-sha-if-needed>"),
                        "GET",
                    ),
                    api::call(
                        &ctx,
                        &format!("repos/{repo}/commits/<draft-target-if-tag-absent>"),
                        "GET",
                    ),
                ]);
                if let Some(file) = &payload {
                    calls.extend([
                        edit::patch_call(
                            &ctx,
                            &format!("repos/{repo}/releases/<release-id>"),
                            file.path(),
                        ),
                        api::call(&ctx, &format!("repos/{repo}/releases/<release-id>"), "GET"),
                        api::call(&ctx, &model::tag_path(repo, tag), "GET"),
                        api::call(
                            &ctx,
                            &format!("repos/{repo}/git/tags/<annotated-tag-sha-if-needed>"),
                            "GET",
                        ),
                        api::call(
                            &ctx,
                            &format!("repos/{repo}/commits/<draft-target-if-tag-absent>"),
                            "GET",
                        ),
                    ]);
                }
            }
            ReleaseCommand::Delete(args) => {
                calls.extend([
                    api::call(
                        &ctx,
                        &format!("repos/{repo}/releases/<release-id>"),
                        "DELETE",
                    ),
                    api::call(&ctx, &format!("repos/{repo}/releases/<release-id>"), "GET"),
                    api::call(&ctx, &lookup, "GET"),
                ]);
                calls.extend([
                    lookup::draft_call(&ctx, repo, tag),
                    api::call(
                        &ctx,
                        &format!("repos/{repo}/releases/<draft-release-id-if-needed>"),
                        "GET",
                    ),
                ]);
                if args.cleanup_tag {
                    calls.extend([
                        api::call(&ctx, &model::tag_path(repo, tag), "GET"),
                        api::call(
                            &ctx,
                            &format!("repos/{repo}/git/refs/tags/{}", guards::encode(tag)),
                            "DELETE",
                        ),
                        api::call(&ctx, &model::tag_path(repo, tag), "GET"),
                    ]);
                }
            }
            _ => unreachable!(),
        }
        let draft = changes.and_then(|changes| changes.draft);
        return Ok(dry_run(&ctx, op, calls, draft, format));
    }
    let value = lookup::find(&ctx, repo, tag, op)?.ok_or_else(|| invalid(op, "release_not_found", "no accessible release exists for this exact tag; check --repo, tag spelling and release read access"))?;
    let release = model::release(value, tag, op)?;
    let object = model::tag_object(&ctx, repo, tag, op)?;
    match command {
        ReleaseCommand::Delete(args) => delete::run(&ctx, repo, &args, &release, object, format),
        ReleaseCommand::View(_) => {
            let commit = model::resolve(&ctx, repo, &release, object, op)?;
            Ok(emit_success(
                schema_version_for("forge-cli", op, 1),
                model::View::new(repo, release, commit),
                format,
                model::View::print,
            ))
        }
        ReleaseCommand::Edit(_) => {
            let commit = model::resolve(&ctx, repo, &release, object, op)?;
            changes
                .expect("validated edit")
                .run(&ctx, release, commit, format)
        }
        _ => unreachable!(),
    }
}
