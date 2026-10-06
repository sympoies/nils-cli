//! Shared target, local text, and dry-run boundary for narrow GitHub writes.
use crate::backend::{BackendCall, BackendRunner, DryRunPayload};
use crate::cli::{BINARY, GlobalFlags};
use crate::envelope::emit_success;
use crate::error::ForgeError;
use crate::provider::{Provider, ProviderContext, detect, git_remote_url};
use crate::validations::{no_agent_attribution, no_escaped_control_markdown, no_local_path};
use nils_common::cli_contract::{OutputFormat, schema_version_for};
use serde::Serialize;
use std::fs;
use std::io::{Read, Write};
use tempfile::NamedTempFile;

pub(super) fn target(global: &GlobalFlags, op: &str) -> Result<ProviderContext, ForgeError> {
    let ctx = detect(
        global.provider_hint(),
        &global.remote,
        global.repo.as_deref(),
        git_remote_url,
    )?;
    if ctx.provider != Provider::GitHub {
        return Err(ForgeError::provider_unsupported(
            schema_version_for(BINARY, op, 1),
            "this operation supports GitHub only",
            None,
        ));
    }
    if ctx.repo.is_none() {
        return Err(invalid(
            op,
            "repository_required",
            "a resolved repository is required (supply --repo)",
        ));
    }
    Ok(ctx)
}
pub(super) fn invalid(op: &str, kind: &'static str, message: &str) -> ForgeError {
    ForgeError::validation(schema_version_for(BINARY, op, 1), kind, message, None)
}
pub(super) fn text_file(path: &str, op: &str) -> Result<String, ForgeError> {
    if path == "-" {
        let mut body = String::new();
        std::io::stdin()
            .read_to_string(&mut body)
            .map_err(|_| invalid(op, "body_file_unreadable", "unable to read text from stdin"))?;
        Ok(body)
    } else {
        fs::read_to_string(path)
            .map_err(|_| invalid(op, "body_file_unreadable", "unable to read text file"))
    }
}
pub(super) fn payload_file(bytes: &[u8], op: &str) -> Result<NamedTempFile, ForgeError> {
    let failure = || {
        ForgeError::software(
            schema_version_for(BINARY, op, 1),
            "unable to prepare provider payload file",
            None,
        )
    };
    let mut file = tempfile::Builder::new()
        .prefix("forge-cli-write-payload-")
        .tempfile()
        .map_err(|_| failure())?;
    file.write_all(bytes).map_err(|_| failure())?;
    Ok(file)
}
pub(super) fn guard_text(text: &str, source: &str) -> Result<(), ForgeError> {
    no_local_path(text, source)?;
    no_agent_attribution(text, source)?;
    no_escaped_control_markdown(text)?;
    Ok(())
}
pub(super) fn selector(value: &str, op: &str) -> Result<(), ForgeError> {
    if value.trim().is_empty() || value.starts_with('-') || value.chars().any(char::is_control) {
        return Err(invalid(
            op,
            "selector_invalid",
            "target selector must be non-empty, contain no control characters, and not start with '-'",
        ));
    }
    Ok(())
}
pub(super) fn execute<T: Serialize>(
    global: &GlobalFlags,
    op: &str,
    ctx: &ProviderContext,
    call: BackendCall,
    format: OutputFormat,
    payload: impl FnOnce(String) -> Result<T, ForgeError>,
) -> Result<i32, ForgeError> {
    let schema = schema_version_for(BINARY, op, 1);
    if global.dry_run {
        return Ok(emit_success(
            schema,
            DryRunPayload::new(ctx.provider, &call),
            format,
            |p| println!("would run: {}", p.plan.join(" ")),
        ));
    }
    // The ordinary runner owns prepare_api, actor verification, redaction,
    // deadlines, and the selected-principal credential environment.
    let output = crate::rate_limit::default_runner().run(&call)?;
    Ok(emit_success(
        schema,
        payload(output.stdout)?,
        format,
        |_| println!("{op}: completed"),
    ))
}
