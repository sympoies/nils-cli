//! GitHub-only view, notes editing and guarded deletion of exact releases.
mod api;
mod delete;
mod guards;
mod lookup;
mod model;

use super::super::github_write::{self as write, invalid};
use crate::backend::{BackendCall, DryRunPayload};
use crate::cli::{GlobalFlags, ReleaseCommand, ReleaseEditArgs};
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
}
fn dry_run(ctx: &ProviderContext, op: &str, calls: Vec<BackendCall>, format: OutputFormat) -> i32 {
    let mut calls = calls.iter();
    emit_success(
        schema_version_for("forge-cli", op, 1),
        Plan {
            first: DryRunPayload::new(ctx.provider, calls.next().expect("release lookup")),
            follow_up: calls
                .map(|call| DryRunPayload::new(ctx.provider, call))
                .collect(),
        },
        format,
        |p| {
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
    let (op, tag) = match &command {
        ReleaseCommand::View(args) => ("release.view", &args.tag),
        ReleaseCommand::Edit(args) => ("release.edit", &args.tag),
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
    // Validate local payloads before provider calls. A scoped JSON file carries
    // literal notes/title; neither content nor secrets are placed in argv.
    let edit_payload = if let ReleaseCommand::Edit(args) = &command {
        let notes = write::text_file(&args.notes_file, op)?;
        write::guard_text(&notes, "release notes")?;
        if let Some(title) = &args.title {
            if title.trim().is_empty() {
                return Err(invalid(
                    op,
                    "release_title_empty",
                    "release title must not be empty",
                ));
            }
            write::guard_text(title, "release title")?;
        }
        let mut payload = serde_json::json!({"body":notes});
        if let Some(title) = &args.title {
            payload["name"] = title.clone().into();
        }
        Some((
            write::payload_file(payload.to_string().as_bytes(), op)?,
            notes,
        ))
    } else {
        None
    };
    let lookup = release_path(repo, tag);
    if global.dry_run {
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
                if matches!(&command, ReleaseCommand::Edit(_)) {
                    let mut patch = api::call(
                        &ctx,
                        &format!("repos/{repo}/releases/<release-id>"),
                        "PATCH",
                    );
                    patch.argv.extend([
                        "--input".into(),
                        edit_payload
                            .as_ref()
                            .unwrap()
                            .0
                            .path()
                            .as_os_str()
                            .to_owned(),
                    ]);
                    calls.extend([
                        patch,
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
        return Ok(dry_run(&ctx, op, calls, format));
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
        ReleaseCommand::Edit(args) => {
            let commit = model::resolve(&ctx, repo, &release, object, op)?;
            let (file, notes) = edit_payload.expect("edit payload");
            edit(&ctx, &args, release, &notes, file.path(), commit, format)
        }
        _ => unreachable!(),
    }
}
fn edit(
    ctx: &ProviderContext,
    args: &ReleaseEditArgs,
    before: model::Release,
    notes: &str,
    file: &std::path::Path,
    commit: String,
    format: OutputFormat,
) -> Result<i32, ForgeError> {
    let op = "release.edit";
    let repo = ctx.repo.as_deref().expect("resolved repository");
    let path = format!("repos/{repo}/releases/{}", before.id);
    let mut call = api::call(ctx, &path, "PATCH");
    call.argv
        .extend(["--input".into(), file.as_os_str().to_owned()]);
    // No automatic replay after an uncertain write outcome.
    api::request(op, &call).map_err(|e| edit_failure("release_edit_failed", "release edit failed or its outcome is unknown; inspect the release before retrying", Some(&e)))?
        .ok_or_else(|| edit_failure("release_edit_failed", "release disappeared before editing; inspect the exact release", None))?;
    let value = api::get(ctx, op, &path)
        .map_err(|e| {
            edit_failure(
                "release_readback_failed",
                "edit was accepted but release readback failed; inspect notes before retrying",
                Some(&e),
            )
        })?
        .ok_or_else(|| {
            edit_failure(
                "release_readback_mismatch",
                "release disappeared after editing",
                None,
            )
        })?;
    let after = model::release(value, &args.tag, op).map_err(|e| {
        edit_failure(
            "release_readback_mismatch",
            "edited release identity could not be verified",
            Some(&e),
        )
    })?;
    let expected_title = args.title.as_ref().or(before.name.as_ref());
    if after.id != before.id
        || after.body.as_deref().unwrap_or_default() != notes
        || after.name.as_ref() != expected_title
        || after.target_commitish != before.target_commitish
        || after.draft != before.draft
        || after.prerelease != before.prerelease
        || after.assets != before.assets
    {
        return Err(edit_failure(
            "release_readback_mismatch",
            "readback differs from requested notes/title or preserved release state; inspect before retrying",
            None,
        ));
    }
    let current_tag = model::tag_object(ctx, repo, &args.tag, op).map_err(|e| {
        edit_failure(
            "release_readback_failed",
            "edited release tag could not be read back",
            Some(&e),
        )
    })?;
    let current_commit = model::resolve(ctx, repo, &after, current_tag, op).map_err(|e| {
        edit_failure(
            "release_readback_failed",
            "edited release target could not be resolved",
            Some(&e),
        )
    })?;
    if current_commit != commit {
        return Err(edit_failure(
            "release_readback_mismatch",
            "tag target changed while editing; inspect the release and tag before retrying",
            None,
        ));
    }
    Ok(emit_success(
        schema_version_for("forge-cli", op, 1),
        model::View::new(repo, after, current_commit),
        format,
        model::View::print,
    ))
}
fn edit_failure(kind: &'static str, message: &str, cause: Option<&ForgeError>) -> ForgeError {
    ForgeError::unavailable_with_details(
        schema_version_for("forge-cli", "release.edit", 1),
        kind,
        message,
        serde_json::json!({"next_action":"inspect_release_before_retry", "retryable":false, "cause":cause.map(ForgeError::kind)}),
    )
}
