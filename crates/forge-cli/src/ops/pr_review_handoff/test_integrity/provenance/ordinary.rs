//! Enumerate complete metadata before acquiring only inspectable source patches.
use super::{
    BackendRunner, ChangedFile, ForgeError, ProviderContext, Subject, api, diff, files, incomplete,
    limit, residual, residual_patch,
};

pub(in super::super) fn comparison<R: BackendRunner>(
    runner: &R,
    ctx: &ProviderContext,
    number: u64,
    subject: &Subject,
) -> Result<(String, Vec<diff::Finding>), ForgeError> {
    // Drop patches in gh, before its stdout reaches the backend capture buffer.
    let files = files(runner, ctx, number, subject, true)?;
    let selected: Vec<_> = files
        .iter()
        .enumerate()
        .filter(|(_, file)| {
            diff::source(&file.filename)
                || file.previous_filename.as_deref().is_some_and(diff::source)
        })
        .collect();
    let paths = selected
        .iter()
        .map(|(_, file)| file.filename.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    if selected.len() >= 300 {
        return Err(limit(
            "file_count",
            &format!("300 or more source paths require test inspection: {paths}"),
        ));
    }
    let repo = ctx.repo.as_deref().ok_or_else(incomplete)?;
    let mut patch = String::new();
    let mut merge_base = None;
    for (index, metadata) in selected {
        // A single-file page bounds capture and associates any refusal with a path.
        let output = runner
            .run(&api(
                ctx,
                format!(
                    "repos/{repo}/pulls/{number}/files?per_page=1&page={}",
                    index + 1
                ),
                &[
                    "--jq",
                    "map(.patch |= if . == null then null else .[:4194305] end)",
                ],
            ))
            .map_err(|error| {
                if error.kind() == "backend_output_limit" {
                    limit(
                        "diff_bytes",
                        &format!(
                            "test inspection patch exceeds capture limits: {}",
                            metadata.filename
                        ),
                    )
                } else {
                    error
                }
            })?;
        let mut batch: Vec<ChangedFile> =
            serde_json::from_str(&output.stdout).map_err(|_| incomplete())?;
        if batch.len() != 1 {
            return Err(incomplete());
        }
        let mut file = batch.pop().unwrap();
        let hunk = file.patch.take();
        if &file != metadata {
            return Err(incomplete());
        }
        file.patch = hunk;
        if file
            .patch
            .as_ref()
            .is_some_and(|text| text.len() > 4 * 1024 * 1024)
        {
            return Err(limit(
                "diff_bytes",
                &format!("test inspection patch exceeds 4 MiB: {}", file.filename),
            ));
        }
        let next = if file.patch.is_some() {
            residual_patch(&file)
        } else {
            if file.status != "added" && merge_base.is_none() {
                merge_base = Some(residual::merge_base(runner, ctx, subject)?);
            }
            residual::patch(
                runner,
                ctx,
                subject,
                merge_base.as_deref().unwrap_or(&subject.base),
                &file,
            )
        };
        let next = next.map_err(|error| {
            if matches!(
                error.kind(),
                "test_changes_limit_exceeded" | "backend_output_limit"
            ) {
                limit(
                    "diff_bytes",
                    &format!(
                        "test inspection patch exceeds 4 MiB or capture limits: {}",
                        file.filename
                    ),
                )
            } else {
                error
            }
        })?;
        if patch.len() + next.len() > 4 * 1024 * 1024 {
            return Err(limit(
                "diff_bytes",
                &format!(
                    "test inspection diff exceeds 4 MiB at {}; inspected paths: {paths}",
                    file.filename
                ),
            ));
        }
        patch.push_str(&next);
    }
    let findings = diff::detect(&patch)?;
    // Bind skipped paths to immutable blob IDs without retaining their contents.
    let binding = serde_json::json!({"files": files, "patch": patch, "merge_base": merge_base});
    Ok((binding.to_string(), findings))
}
