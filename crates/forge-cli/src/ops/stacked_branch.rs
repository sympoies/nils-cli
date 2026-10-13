//! Preserve source branches that open pull/merge requests still target.
use serde::Serialize;
use serde_json::Value;

use super::gitlab_api;
use crate::backend::{BackendCall, BackendProgram, BackendRunner};
use crate::error::ForgeError;
use crate::provider::{Provider, ProviderContext};

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct DependentPr {
    pub number: u64,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct BranchCleanup {
    pub branch: String,
    pub dependents: Vec<DependentPr>,
    pub overridden: bool,
    pub message: String,
}

fn unavailable() -> ForgeError {
    ForgeError::unavailable(
        "cli.forge-cli.error.v1",
        "branch_dependencies_unavailable",
        "could not verify open PR/MR branch dependencies; merge and branch deletion deferred",
        None,
    )
}

pub(crate) fn check<R: BackendRunner>(
    runner: &R,
    ctx: &ProviderContext,
    target_repo: &str,
    source_repo: Option<&str>,
    branch: &str,
    merging: u64,
    overridden: bool,
) -> Result<Option<BranchCleanup>, ForgeError> {
    let source = source_repo.unwrap_or(target_repo);
    let same_repository = source.eq_ignore_ascii_case(target_repo);
    let merging = same_repository.then_some(merging);
    let mut dependents = Vec::new();
    match ctx.provider {
        Provider::GitHub => {
            let path = format!(
                "repos/{source}/pulls?state=open&base={}&per_page=100",
                gitlab_api::encode_query_value(branch)
            );
            let call = github_api_call(ctx, &["api", "--paginate", "--slurp", &path]);
            let output = runner.run(&call).map_err(|_| unavailable())?;
            let pages: Value = serde_json::from_str(&output.stdout).map_err(|_| unavailable())?;
            for page in pages.as_array().ok_or_else(unavailable)? {
                collect(page, false, branch, merging, &mut dependents)?;
            }
        }
        Provider::GitLab => {
            let project = source
                .strip_prefix("gitlab-project-id:")
                .map(str::to_string)
                .unwrap_or_else(|| gitlab_api::encode_project_path(source));
            // Explicit pages avoid glab's concatenated JSON pagination output.
            let mut page = 1;
            loop {
                let path = format!(
                    "projects/{project}/merge_requests?state=opened&target_branch={}&per_page=100&page={page}",
                    gitlab_api::encode_query_value(branch)
                );
                let output = runner
                    .run(&gitlab_api::api_call(&ctx.host, path))
                    .map_err(|_| unavailable())?;
                let rows: Value =
                    serde_json::from_str(&output.stdout).map_err(|_| unavailable())?;
                let count = rows.as_array().ok_or_else(unavailable)?.len();
                collect(&rows, true, branch, merging, &mut dependents)?;
                if count < 100 {
                    break;
                }
                page += 1;
            }
        }
        Provider::Local => return Ok(None),
    }
    if dependents.is_empty() {
        return Ok(None);
    }
    dependents.sort_by_key(|pr| pr.number);
    dependents.dedup_by_key(|pr| pr.number);
    let numbers = dependents
        .iter()
        .map(|pr| format!("#{}", pr.number))
        .collect::<Vec<_>>()
        .join(", ");
    if !overridden && ctx.provider == Provider::GitHub && same_repository {
        // GitHub can delete a branch even without gh's --delete-branch flag.
        let call = github_api_call(ctx, &["api", &format!("repos/{target_repo}")]);
        let output = runner.run(&call).map_err(|_| unavailable())?;
        let repo: Value = serde_json::from_str(&output.stdout).map_err(|_| unavailable())?;
        if repo
            .get("delete_branch_on_merge")
            .and_then(Value::as_bool)
            .ok_or_else(unavailable)?
        {
            return Err(ForgeError::validation(
                "cli.forge-cli.error.v1",
                "stacked_branch_auto_delete",
                format!(
                    "merge deferred: automatic branch deletion could close dependent PRs {numbers}; retarget them or disable automatic deletion before merging"
                ),
                None,
            ));
        }
    }
    let message = if overridden {
        format!(
            "branch deletion override: '{branch}' is the base of open PRs/MRs {numbers}; deletion may close them"
        )
    } else {
        format!(
            "branch deletion skipped: '{branch}' is the base of open PRs/MRs {numbers}; retarget them before cleanup or explicitly use --delete-branch-even-if-stacked"
        )
    };
    Ok(Some(BranchCleanup {
        branch: branch.into(),
        dependents,
        overridden,
        message,
    }))
}

fn github_api_call(ctx: &ProviderContext, args: &[&str]) -> BackendCall {
    let mut argv = args.iter().map(std::ffi::OsString::from).collect();
    ctx.push_github_api_hostname(&mut argv);
    BackendCall::new(BackendProgram::Gh, argv)
}

fn collect(
    rows: &Value,
    gitlab: bool,
    branch: &str,
    merging: Option<u64>,
    result: &mut Vec<DependentPr>,
) -> Result<(), ForgeError> {
    for row in rows.as_array().ok_or_else(unavailable)? {
        let number = row
            .get(if gitlab { "iid" } else { "number" })
            .and_then(Value::as_u64)
            .ok_or_else(unavailable)?;
        let state = row
            .get("state")
            .and_then(Value::as_str)
            .ok_or_else(unavailable)?;
        let base = if gitlab {
            row.get("target_branch")
        } else {
            row.get("base").and_then(|base| base.get("ref"))
        }
        .and_then(Value::as_str)
        .ok_or_else(unavailable)?;
        let url = row
            .get(if gitlab { "web_url" } else { "html_url" })
            .and_then(Value::as_str)
            .filter(|url| !url.is_empty())
            .ok_or_else(unavailable)?;
        if Some(number) != merging
            && state == if gitlab { "opened" } else { "open" }
            && base == branch
        {
            result.push(DependentPr {
                number,
                url: url.into(),
            });
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "stacked_branch_tests.rs"]
mod tests;
