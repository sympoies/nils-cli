//! Exact provider enumeration and immutable upstream blob identity for imports.
use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};

use super::super::valid_sha;
use super::{
    BackendRunner, ForgeError, ProviderContext, Subject, api, diff, incomplete, limit,
    sha256_digest,
};

mod declaration;
mod residual;
pub(super) use declaration::parse;
use declaration::{Declaration, valid_path};

const PREFIX: &str = "provenance-import-v1\n";

fn refusal(kind: &'static str, message: &str, recovery: &str) -> ForgeError {
    ForgeError::validation_with_details(
        super::super::schema(),
        kind,
        message,
        json!({"next_action": if kind == "provenance_import_incomplete" { "retry_complete_upstream" } else { "correct_provenance_declaration" },
            "retryable": kind == "provenance_import_incomplete", "recovery": recovery}),
    )
}

fn upstream_incomplete() -> ForgeError {
    refusal(
        "provenance_import_incomplete",
        "cannot read the complete pinned upstream tree",
        "Retry with readable, non-truncated upstream commit/tree evidence. The complete upstream tree must fit provider and backend capture limits; splitting the downstream PR alone cannot repair an oversized upstream tree. No partial tree is admitted.",
    )
}

fn mismatch() -> ForgeError {
    refusal(
        "provenance_import_mismatch",
        "import does not match the declared upstream provenance",
        "Correct the upstream repository, full commit SHA and path mapping. Restore identical additions or list each changed mapped addition in adaptations, then declare and review its import-adaptation finding. List exact newly added mapped paths in additions; missing upstream blobs cannot be adapted.",
    )
}

fn read<R: BackendRunner, T: DeserializeOwned>(
    runner: &R,
    ctx: &ProviderContext,
    endpoint: String,
    failure: fn() -> ForgeError,
) -> Result<T, ForgeError> {
    let output = runner.run(&api(ctx, endpoint, &[]))?;
    serde_json::from_str(&output.stdout).map_err(|_| failure())
}

#[derive(Clone, Deserialize, Serialize)]
struct ChangedFile {
    filename: String,
    status: String,
    sha: String,
    additions: usize,
    deletions: usize,
    #[serde(default)]
    previous_filename: Option<String>,
    #[serde(default)]
    patch: Option<String>,
}

#[derive(Deserialize)]
struct Object {
    sha: String,
}
#[derive(Deserialize)]
struct Commit {
    sha: String,
    tree: Object,
}
#[derive(Deserialize)]
struct Tree {
    sha: String,
    truncated: bool,
    tree: Vec<Entry>,
}
#[derive(Deserialize)]
struct Entry {
    path: String,
    sha: String,
    mode: String,
    #[serde(rename = "type")]
    kind: String,
}

fn upstream<R: BackendRunner>(
    runner: &R,
    ctx: &ProviderContext,
    d: &Declaration,
) -> Result<BTreeMap<String, Entry>, ForgeError> {
    let _scope = crate::identity::api_read_scope(&ctx.host, &d.repository)?;
    let commit: Commit = read(
        runner,
        ctx,
        format!("repos/{}/git/commits/{}", d.repository, d.commit),
        upstream_incomplete,
    )?;
    if commit.sha != d.commit || !valid_sha(&commit.tree.sha) {
        return Err(mismatch());
    }
    let tree: Tree = read(
        runner,
        ctx,
        format!(
            "repos/{}/git/trees/{}?recursive=1",
            d.repository, commit.tree.sha
        ),
        upstream_incomplete,
    )?;
    if tree.truncated {
        return Err(upstream_incomplete());
    }
    if tree.sha != commit.tree.sha {
        return Err(mismatch());
    }
    let mut entries = BTreeMap::new();
    for entry in tree.tree {
        if !valid_sha(&entry.sha) || entries.insert(entry.path.clone(), entry).is_some() {
            return Err(upstream_incomplete());
        }
    }
    Ok(entries)
}

/// PR-files pagination is bounded by the provider's 3,000-file ceiling. Exact
/// page sizes, unique paths and changed_files must agree before any omission.
fn files<R: BackendRunner>(
    runner: &R,
    ctx: &ProviderContext,
    number: u64,
    subject: &Subject,
) -> Result<Vec<ChangedFile>, ForgeError> {
    let count = subject.changed_files.ok_or_else(incomplete)?;
    if count >= 3000 {
        return Err(limit(
            "provider_file_count",
            "provenance comparisons require fewer than 3,000 files",
        ));
    }
    let repo = ctx.repo.as_deref().ok_or_else(incomplete)?;
    let mut files = Vec::new();
    let mut paths = BTreeSet::new();
    for page in 1..=count.div_ceil(100) {
        let batch: Vec<ChangedFile> = read(
            runner,
            ctx,
            format!("repos/{repo}/pulls/{number}/files?per_page=100&page={page}"),
            incomplete,
        )?;
        if batch.len() != (count - files.len()).min(100) {
            return Err(incomplete());
        }
        for file in batch {
            if !valid_path(&file.filename)
                || file
                    .previous_filename
                    .as_deref()
                    .is_some_and(|p| !valid_path(p))
            {
                return Err(limit(
                    "path_bytes",
                    "provenance comparison contains unsupported paths",
                ));
            }
            if !paths.insert(file.filename.clone())
                || !valid_sha(&file.sha)
                || !matches!(
                    file.status.as_str(),
                    "added" | "modified" | "removed" | "renamed" | "changed"
                )
                || (file.status == "renamed") != file.previous_filename.is_some()
            {
                return Err(incomplete());
            }
            files.push(file);
        }
    }
    files.sort_by(|a, b| a.filename.cmp(&b.filename));
    Ok(files)
}

fn residual_patch(file: &ChangedFile) -> Result<String, ForgeError> {
    let patch = file.patch.as_deref().ok_or_else(incomplete)?;
    // Hunk counts alone cannot detect omission of an entire final hunk.
    if patch.lines().filter(|l| l.starts_with('+')).count() != file.additions
        || patch.lines().filter(|l| l.starts_with('-')).count() != file.deletions
        || patch
            .lines()
            .any(|l| !l.starts_with(['@', '+', '-', ' ', '\\']))
    {
        return Err(incomplete());
    }
    let old = file.previous_filename.as_deref().unwrap_or(&file.filename);
    let mut result = format!("diff --git a/{old} b/{}\n", file.filename);
    if file.status == "removed" {
        result.push_str("deleted file mode 100644\n");
    }
    if file.status == "renamed" {
        result.push_str(&format!("rename from {old}\nrename to {}\n", file.filename));
    }
    result.push_str(patch);
    result.push('\n');
    Ok(result)
}

pub(super) fn comparison<R: BackendRunner>(
    runner: &R,
    ctx: &ProviderContext,
    number: u64,
    subject: &Subject,
    d: &Declaration,
) -> Result<(String, Vec<diff::Finding>), ForgeError> {
    let files = files(runner, ctx, number, subject)?;
    let tree = upstream(runner, ctx, d)?;
    let merge_base = if files
        .iter()
        .any(|file| file.patch.is_none() && file.status != "added")
    {
        Some(residual::merge_base(runner, ctx, subject)?)
    } else {
        None
    };
    let mut manifest = Vec::new();
    let mut residual = String::new();
    let mut adaptations = BTreeSet::new();
    let mut additions = BTreeSet::new();
    for file in &files {
        if file.status == "added"
            && let Some(mapping) = d
                .mappings
                .iter()
                .find(|m| file.filename.starts_with(&m.destination))
        {
            let path = format!(
                "{}{}",
                mapping.source,
                &file.filename[mapping.destination.len()..]
            );
            if d.additions.contains(&file.filename) {
                if tree.contains_key(&path) {
                    return Err(mismatch());
                }
                additions.insert(file.filename.clone());
            } else {
                let blob = tree.get(&path).ok_or_else(mismatch)?;
                if blob.kind != "blob" || !matches!(blob.mode.as_str(), "100644" | "100755") {
                    return Err(mismatch());
                }
                if blob.sha == file.sha {
                    manifest.push((&file.filename, path, &blob.sha));
                    continue;
                }
                if !d.adaptations.contains(&file.filename) {
                    return Err(mismatch());
                }
                adaptations.insert(file.filename.clone());
            }
        }
        if file.patch.is_some() {
            residual.push_str(&residual_patch(file)?);
        } else {
            residual.push_str(&residual::patch(
                runner,
                ctx,
                subject,
                merge_base.as_deref().unwrap_or(&subject.base),
                file,
            )?);
        }
    }
    if adaptations != d.adaptations.iter().cloned().collect()
        || additions != d.additions.iter().cloned().collect()
    {
        return Err(mismatch());
    }
    let mut findings = diff::detect(&residual)?;
    findings.extend(adaptations.iter().map(|path| diff::Finding {
        path: path.clone(),
        kind: "import-adaptation",
    }));
    findings.extend(additions.iter().map(|path| diff::Finding {
        path: path.clone(),
        kind: "import-addition",
    }));
    findings.sort();
    let mut binding = json!({"repository":d.repository, "commit":d.commit,
        "file_count":files.len(), "verified_files":manifest.len(),
        "manifest_digest":sha256_digest(&serde_json::to_vec(&manifest).unwrap()),
        "comparison_digest":sha256_digest(&serde_json::to_vec(&files).unwrap()),
        "mappings":d.mappings, "adaptations":d.adaptations, "additions":d.additions,
        "declaration_digest":sha256_digest(subject.body.as_bytes())});
    if let Some(base) = merge_base {
        binding["merge_base"] = json!(base);
    }
    Ok((format!("{PREFIX}{binding}"), findings))
}

pub(super) fn summary(binding: &str) -> Option<Value> {
    serde_json::from_str(binding.strip_prefix(PREFIX)?).ok()
}
