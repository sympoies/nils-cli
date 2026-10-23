//! Reconstruct omitted residual text patches from immutable provider blobs.
use std::{process::Command, time::Duration};

use base64::{Engine, engine::general_purpose::STANDARD};
use serde::Deserialize;
use serde_json::json;
use sha1::{Digest, Sha1};

use super::{
    BackendRunner, ChangedFile, ForgeError, ProviderContext, Subject, incomplete, limit, read,
};

#[derive(Deserialize)]
struct Content {
    sha: String,
    size: usize,
    #[serde(rename = "type")]
    kind: String,
}
#[derive(Deserialize)]
struct Blob {
    sha: String,
    size: usize,
    encoding: String,
    content: String,
}

fn bytes<R: BackendRunner>(
    runner: &R,
    ctx: &ProviderContext,
    path: &str,
    commit: &str,
    expected: Option<&str>,
) -> Result<Vec<u8>, ForgeError> {
    let repo = ctx.repo.as_deref().ok_or_else(incomplete)?;
    let path = path
        .split('/')
        .map(|part| url::form_urlencoded::byte_serialize(part.as_bytes()).collect::<String>())
        .collect::<Vec<_>>()
        .join("/");
    let content: Content = read(
        runner,
        ctx,
        format!("repos/{repo}/contents/{path}?ref={commit}"),
        incomplete,
    )?;
    if content.kind != "file"
        || !super::super::super::valid_sha(&content.sha)
        || expected.is_some_and(|sha| sha != content.sha)
    {
        return Err(incomplete());
    }
    let blob: Blob = read(
        runner,
        ctx,
        format!("repos/{repo}/git/blobs/{}", content.sha),
        incomplete,
    )?;
    if blob.sha != content.sha || blob.size != content.size || blob.encoding != "base64" {
        return Err(incomplete());
    }
    let encoded: String = blob
        .content
        .chars()
        .filter(|c| !c.is_ascii_whitespace())
        .collect();
    let bytes = STANDARD.decode(encoded).map_err(|_| incomplete())?;
    let mut hash = Sha1::new();
    hash.update(format!("blob {}\0", bytes.len()));
    hash.update(&bytes);
    if bytes.len() != content.size || format!("{:x}", hash.finalize()) != content.sha {
        return Err(incomplete());
    }
    Ok(bytes)
}

pub(super) fn patch<R: BackendRunner>(
    runner: &R,
    ctx: &ProviderContext,
    subject: &Subject,
    file: &ChangedFile,
) -> Result<String, ForgeError> {
    // A write-capable PR command must still authorize these reads as api_read.
    let _scope =
        crate::identity::api_read_scope(&ctx.host, ctx.repo.as_deref().ok_or_else(incomplete)?)?;
    let before = if file.status == "added" {
        Vec::new()
    } else {
        bytes(
            runner,
            ctx,
            file.previous_filename.as_deref().unwrap_or(&file.filename),
            &subject.base,
            (file.status == "removed").then_some(file.sha.as_str()),
        )?
    };
    let after = if file.status == "removed" {
        Vec::new()
    } else {
        bytes(runner, ctx, &file.filename, &subject.head, Some(&file.sha))?
    };
    if [&before, &after]
        .iter()
        .any(|bytes| bytes.contains(&0) || std::str::from_utf8(bytes).is_err())
    {
        return Err(ForgeError::validation_with_details(
            super::super::super::schema(),
            "test_changes_incomplete",
            "provider omitted the residual binary patch; map identical upstream assets",
            json!({"cause":{"code":"residual_binary_patch_unavailable","path":file.filename},
                "next_action":"correct_provenance_declaration", "retryable":false,
                "recovery":"Keep upstream-identical binary additions mapped to their pinned blobs. A residual binary file without an inspectable provider patch is not admitted."}),
        ));
    }
    let dir = tempfile::tempdir().map_err(|_| incomplete())?;
    std::fs::write(dir.path().join("before"), &before).map_err(|_| incomplete())?;
    std::fs::write(dir.path().join("after"), &after).map_err(|_| incomplete())?;
    let output = crate::backend::output_with_limits(
        Command::new("git")
            .args([
                "diff",
                "--no-index",
                "--no-ext-diff",
                "--no-textconv",
                "--no-color",
                "--no-renames",
                "--text",
                "--diff-algorithm=myers",
                "--unified=3",
                "--",
                "before",
                "after",
            ])
            .current_dir(dir.path())
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_CONFIG_COUNT")
            .env_remove("GIT_CONFIG_PARAMETERS")
            .env_remove("GIT_DIFF_OPTS")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null"),
        Some(Duration::from_secs(15)),
        4 * 1024 * 1024,
    )
    .map_err(|error| match error {
        crate::backend::ProcessOutputError::OutputLimit { .. } => {
            limit("diff_bytes", "reconstructed diff exceeds 4 MiB")
        }
        _ => incomplete(),
    })?;
    if !matches!(output.status.code(), Some(0 | 1)) {
        return Err(incomplete());
    }
    let output = String::from_utf8(output.stdout).map_err(|_| incomplete())?;
    // Replace temporary Git paths with the provider's validated repository paths.
    let hunks = output
        .lines()
        .skip_while(|line| !line.starts_with("@@ "))
        .collect::<Vec<_>>()
        .join("\n");
    if before != after && hunks.is_empty() {
        return Err(incomplete());
    }
    let mut reconstructed = file.clone();
    reconstructed.additions = hunks.lines().filter(|line| line.starts_with('+')).count();
    reconstructed.deletions = hunks.lines().filter(|line| line.starts_with('-')).count();
    reconstructed.patch = Some(hunks);
    super::residual_patch(&reconstructed)
}
