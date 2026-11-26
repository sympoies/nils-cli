//! Edit specified release fields and independently verify the result.
use super::{api, model};
use crate::backend::BackendCall;
use crate::cli::ReleaseEditArgs;
use crate::envelope::emit_success;
use crate::error::ForgeError;
use crate::ops::github_write::{self as write, invalid};
use crate::provider::ProviderContext;
use nils_common::cli_contract::{OutputFormat, schema_version_for};
use serde::Serialize;
use std::path::Path;
use tempfile::NamedTempFile;

pub(super) const OP: &str = "release.edit";

#[derive(Serialize)]
pub(super) struct Changes {
    #[serde(rename = "body", skip_serializing_if = "Option::is_none")]
    notes: Option<String>,
    #[serde(rename = "name", skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub draft: Option<bool>,
}

impl Changes {
    pub fn new(args: &ReleaseEditArgs) -> Result<Self, ForgeError> {
        let draft = if args.draft {
            Some(true)
        } else {
            args.publish.then_some(false)
        };
        if args.notes_file.is_none() && args.title.is_none() && draft.is_none() {
            return Err(invalid(
                OP,
                "release_edit_empty",
                "specify at least one of --notes-file, --title, --draft, or --publish",
            ));
        }
        let notes = args
            .notes_file
            .as_deref()
            .map(|path| write::text_file(path, OP))
            .transpose()?;
        if let Some(notes) = &notes {
            write::guard_text(notes, "release notes")?;
        }
        if let Some(title) = &args.title {
            if title.trim().is_empty() {
                return Err(invalid(
                    OP,
                    "release_title_empty",
                    "release title must not be empty",
                ));
            }
            write::guard_text(title, "release title")?;
        }
        Ok(Self {
            notes,
            title: args.title.clone(),
            draft,
        })
    }

    pub fn payload_file(&self) -> Result<NamedTempFile, ForgeError> {
        let payload = serde_json::to_vec(self).expect("release edit fields are JSON serializable");
        write::payload_file(&payload, OP)
    }

    fn matches(&self, before: &model::Release, observed: &model::Release) -> bool {
        let notes_match = match &self.notes {
            Some(notes) => observed.body.as_deref().unwrap_or_default() == notes,
            None => observed.body == before.body,
        };
        notes_match
            && observed.name.as_ref() == self.title.as_ref().or(before.name.as_ref())
            && observed.draft == self.draft.unwrap_or(before.draft)
    }

    pub fn run(
        &self,
        ctx: &ProviderContext,
        before: model::Release,
        commit: String,
        format: OutputFormat,
    ) -> Result<i32, ForgeError> {
        let repo = ctx.repo.as_deref().expect("resolved repository");
        let path = format!("repos/{repo}/releases/{}", before.id);
        if self.draft.is_none() || !self.matches(&before, &before) {
            let file = self.payload_file()?;
            let call = patch_call(ctx, &path, file.path());
            // No automatic replay after an uncertain write outcome.
            api::request(OP, &call)
                .map_err(|e| failure("release_edit_failed", "release edit failed or its outcome is unknown; inspect the release before retrying", Some(&e)))?
                .ok_or_else(|| failure("release_edit_failed", "release disappeared before editing; inspect the exact release", None))?;
        }

        let value = api::get(ctx, OP, &path)
            .map_err(|e| {
                failure(
                    "release_readback_failed",
                    "release readback failed; inspect the release before retrying",
                    Some(&e),
                )
            })?
            .ok_or_else(|| {
                failure(
                    "release_readback_mismatch",
                    "release disappeared during editing",
                    None,
                )
            })?;
        let after = model::release(value, &before.tag_name, OP).map_err(|e| {
            failure(
                "release_readback_mismatch",
                "edited release identity could not be verified",
                Some(&e),
            )
        })?;
        if after.id != before.id
            || !self.matches(&before, &after)
            || after.target_commitish != before.target_commitish
            || after.prerelease != before.prerelease
            || after.assets != before.assets
        {
            return Err(failure(
                "release_readback_mismatch",
                "readback differs from requested fields or preserved release state; inspect before retrying",
                None,
            ));
        }
        let current_tag = model::tag_object(ctx, repo, &after.tag_name, OP).map_err(|e| {
            failure(
                "release_readback_failed",
                "edited release tag could not be read back",
                Some(&e),
            )
        })?;
        let current_commit = model::resolve(ctx, repo, &after, current_tag, OP).map_err(|e| {
            failure(
                "release_readback_failed",
                "edited release target could not be resolved",
                Some(&e),
            )
        })?;
        if current_commit != commit {
            return Err(failure(
                "release_readback_mismatch",
                "tag target changed while editing; inspect the release and tag before retrying",
                None,
            ));
        }
        Ok(emit_success(
            schema_version_for("forge-cli", OP, 1),
            model::View::new(repo, after, current_commit),
            format,
            model::View::print,
        ))
    }
}

pub(super) fn patch_call(ctx: &ProviderContext, path: &str, file: &Path) -> BackendCall {
    let mut call = api::call(ctx, path, "PATCH");
    call.argv
        .extend(["--input".into(), file.as_os_str().to_owned()]);
    call
}

fn failure(kind: &'static str, message: &str, cause: Option<&ForgeError>) -> ForgeError {
    ForgeError::unavailable_with_details(
        schema_version_for("forge-cli", OP, 1),
        kind,
        message,
        serde_json::json!({"next_action":"inspect_release_before_retry", "retryable":false, "cause":cause.map(ForgeError::kind)}),
    )
}
