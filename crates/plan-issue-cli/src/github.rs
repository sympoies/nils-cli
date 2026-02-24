use std::fs;
use std::path::Path;

use nils_common::git as common_git;
use nils_common::process as common_process;
use nils_common::markdown;
use serde_json::Value;

use crate::commands::plan::CloseReason;

pub trait GitHubAdapter {
    fn issue_body(&self, repo: &str, issue: u64) -> Result<String, String>;

    fn create_issue(
        &self,
        repo: &str,
        title: &str,
        body_file: &Path,
        labels: &[String],
    ) -> Result<(u64, String), String>;

    fn edit_issue_body(&self, repo: &str, issue: u64, body_file: &Path) -> Result<(), String>;

    fn comment_issue(&self, repo: &str, issue: u64, body_file: &Path) -> Result<(), String>;

    fn edit_issue_labels(
        &self,
        repo: &str,
        issue: u64,
        add_labels: &[String],
        remove_labels: &[String],
    ) -> Result<(), String>;

    fn close_issue(
        &self,
        repo: &str,
        issue: u64,
        reason: CloseReason,
        close_comment: Option<&str>,
    ) -> Result<(), String>;

    fn pr_is_merged(&self, repo: &str, pr: u64) -> Result<bool, String>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct GhCliAdapter {
    force: bool,
}

impl GhCliAdapter {
    pub const fn new(force: bool) -> Self {
        Self { force }
    }

    fn run(args: &[String]) -> Result<String, String> {
        let arg_refs = args.iter().map(String::as_str).collect::<Vec<_>>();
        let output = common_process::run_output("gh", &arg_refs)
            .map(|output| output.into_std_output())
            .map_err(|err| format!("failed to execute gh: {err}"))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let detail = if stderr.is_empty() { stdout } else { stderr };
            return Err(format!("gh {} failed: {detail}", args.join(" ")));
        }

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    fn parse_json(stdout: &str, context: &str) -> Result<Value, String> {
        serde_json::from_str(stdout.trim())
            .map_err(|err| format!("failed to parse gh JSON for {context}: {err}"))
    }

    fn guard_markdown_payload(&self, payload: &str, context: &str) -> Result<(), String> {
        if self.force {
            return Ok(());
        }

        markdown::validate_markdown_payload(payload).map_err(|err| {
            format!("{context}: {err}. Replace escaped controls or re-run with --force.")
        })
    }

    fn guard_markdown_file(&self, path: &Path, context: &str) -> Result<(), String> {
        if self.force {
            return Ok(());
        }

        let payload = fs::read_to_string(path).map_err(|err| {
            format!(
                "{context}: failed to read markdown payload {}: {err}",
                path.display()
            )
        })?;

        self.guard_markdown_payload(&payload, context)
    }
}

impl GitHubAdapter for GhCliAdapter {
    fn issue_body(&self, repo: &str, issue: u64) -> Result<String, String> {
        let args = vec![
            "issue".to_string(),
            "view".to_string(),
            issue.to_string(),
            "--repo".to_string(),
            repo.to_string(),
            "--json".to_string(),
            "body".to_string(),
        ];
        let stdout = Self::run(&args)?;
        let json = Self::parse_json(&stdout, "issue view")?;
        let body = json
            .get("body")
            .and_then(Value::as_str)
            .ok_or_else(|| "gh issue view JSON missing `body`".to_string())?;
        Ok(body.to_string())
    }

    fn create_issue(
        &self,
        repo: &str,
        title: &str,
        body_file: &Path,
        labels: &[String],
    ) -> Result<(u64, String), String> {
        self.guard_markdown_file(body_file, "github issue create body write rejected")?;

        let mut args = vec![
            "issue".to_string(),
            "create".to_string(),
            "--repo".to_string(),
            repo.to_string(),
            "--title".to_string(),
            title.to_string(),
            "--body-file".to_string(),
            body_file.to_string_lossy().to_string(),
        ];

        for label in labels {
            let trimmed = label.trim();
            if !trimmed.is_empty() {
                args.push("--label".to_string());
                args.push(trimmed.to_string());
            }
        }

        let stdout = Self::run(&args)?;
        let url = stdout.trim().to_string();
        let issue_number = issue_number_from_url(&url)
            .ok_or_else(|| format!("unable to parse issue number from gh output: {url}"))?;
        Ok((issue_number, url))
    }

    fn edit_issue_body(&self, repo: &str, issue: u64, body_file: &Path) -> Result<(), String> {
        self.guard_markdown_file(body_file, "github issue body update rejected")?;

        let args = vec![
            "issue".to_string(),
            "edit".to_string(),
            issue.to_string(),
            "--repo".to_string(),
            repo.to_string(),
            "--body-file".to_string(),
            body_file.to_string_lossy().to_string(),
        ];
        Self::run(&args).map(|_| ())
    }

    fn comment_issue(&self, repo: &str, issue: u64, body_file: &Path) -> Result<(), String> {
        self.guard_markdown_file(body_file, "github issue comment write rejected")?;

        let args = vec![
            "issue".to_string(),
            "comment".to_string(),
            issue.to_string(),
            "--repo".to_string(),
            repo.to_string(),
            "--body-file".to_string(),
            body_file.to_string_lossy().to_string(),
        ];
        Self::run(&args).map(|_| ())
    }

    fn edit_issue_labels(
        &self,
        repo: &str,
        issue: u64,
        add_labels: &[String],
        remove_labels: &[String],
    ) -> Result<(), String> {
        let mut args = vec![
            "issue".to_string(),
            "edit".to_string(),
            issue.to_string(),
            "--repo".to_string(),
            repo.to_string(),
        ];

        let add_csv = add_labels
            .iter()
            .map(|label| label.trim())
            .filter(|label| !label.is_empty())
            .collect::<Vec<_>>()
            .join(",");
        if !add_csv.is_empty() {
            args.push("--add-label".to_string());
            args.push(add_csv);
        }

        let remove_csv = remove_labels
            .iter()
            .map(|label| label.trim())
            .filter(|label| !label.is_empty())
            .collect::<Vec<_>>()
            .join(",");
        if !remove_csv.is_empty() {
            args.push("--remove-label".to_string());
            args.push(remove_csv);
        }

        if args.len() == 5 {
            return Ok(());
        }

        Self::run(&args).map(|_| ())
    }

    fn close_issue(
        &self,
        repo: &str,
        issue: u64,
        reason: CloseReason,
        close_comment: Option<&str>,
    ) -> Result<(), String> {
        let mut args = vec![
            "issue".to_string(),
            "close".to_string(),
            issue.to_string(),
            "--repo".to_string(),
            repo.to_string(),
            "--reason".to_string(),
            match reason {
                CloseReason::Completed => "completed",
                CloseReason::NotPlanned => "not planned",
            }
            .to_string(),
        ];

        if let Some(comment) = close_comment {
            let trimmed = comment.trim();
            if !trimmed.is_empty() {
                self.guard_markdown_payload(trimmed, "github issue close comment write rejected")?;
                args.push("--comment".to_string());
                args.push(trimmed.to_string());
            }
        }

        Self::run(&args).map(|_| ())
    }

    fn pr_is_merged(&self, repo: &str, pr: u64) -> Result<bool, String> {
        let args = vec![
            "pr".to_string(),
            "view".to_string(),
            pr.to_string(),
            "--repo".to_string(),
            repo.to_string(),
            "--json".to_string(),
            "state,mergedAt".to_string(),
        ];
        let stdout = Self::run(&args)?;
        let json = Self::parse_json(&stdout, "pr view")?;

        let merged_at_present = !json.get("mergedAt").is_some_and(Value::is_null);
        let merged_state = json
            .get("state")
            .and_then(Value::as_str)
            .is_some_and(|state| state.eq_ignore_ascii_case("merged"));

        Ok(merged_at_present || merged_state)
    }
}

pub fn resolve_repo(repo_override: Option<&str>) -> Result<String, String> {
    if let Some(repo) = repo_override {
        return normalize_repo_slug(repo).ok_or_else(|| format!("invalid --repo value: {repo}"));
    }

    let output = common_git::run_output(&["remote", "get-url", "origin"])
        .map_err(|err| format!("failed to run `git remote get-url origin`: {err}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(format!(
            "failed to resolve repository from git remote: {}",
            if stderr.is_empty() {
                "unknown error"
            } else {
                &stderr
            }
        ));
    }

    let remote = String::from_utf8_lossy(&output.stdout).trim().to_string();
    normalize_repo_slug(&remote).ok_or_else(|| {
        format!(
            "unable to derive owner/repo from origin remote `{remote}`; pass --repo <owner/repo>"
        )
    })
}

fn normalize_repo_slug(raw: &str) -> Option<String> {
    let trimmed = raw.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return None;
    }

    let candidate = trimmed
        .strip_prefix("git@github.com:")
        .or_else(|| trimmed.strip_prefix("https://github.com/"))
        .or_else(|| trimmed.strip_prefix("http://github.com/"))
        .or_else(|| trimmed.strip_prefix("ssh://git@github.com/"));

    if let Some(candidate) = candidate {
        let normalized = candidate.trim_end_matches(".git").trim_end_matches('/');
        if is_owner_repo(normalized) {
            return Some(normalized.to_string());
        }
    }

    if is_owner_repo(trimmed) {
        Some(trimmed.to_string())
    } else {
        None
    }
}

fn is_owner_repo(value: &str) -> bool {
    if value.contains(':') || value.contains("://") || value.ends_with(".git") {
        return false;
    }

    let mut parts = value.split('/');
    let owner = parts.next().unwrap_or_default().trim();
    let repo = parts.next().unwrap_or_default().trim();
    parts.next().is_none() && !owner.is_empty() && !repo.is_empty()
}

fn issue_number_from_url(url: &str) -> Option<u64> {
    let trimmed = url.trim().trim_end_matches('/');
    let tail = trimmed.rsplit('/').next()?;
    tail.parse::<u64>().ok()
}

#[cfg(test)]
mod tests {
    use super::{issue_number_from_url, normalize_repo_slug};

    #[test]
    fn normalize_repo_slug_accepts_common_remote_forms() {
        let samples = [
            ("graysurf/nils-cli", "graysurf/nils-cli"),
            ("git@github.com:graysurf/nils-cli.git", "graysurf/nils-cli"),
            (
                "https://github.com/graysurf/nils-cli.git",
                "graysurf/nils-cli",
            ),
            (
                "ssh://git@github.com/graysurf/nils-cli.git",
                "graysurf/nils-cli",
            ),
        ];

        for (raw, expected) in samples {
            assert_eq!(normalize_repo_slug(raw).as_deref(), Some(expected));
        }
    }

    #[test]
    fn issue_number_from_url_extracts_tail_numeric_segment() {
        assert_eq!(
            issue_number_from_url("https://github.com/graysurf/nils-cli/issues/217"),
            Some(217)
        );
        assert_eq!(
            issue_number_from_url("https://github.com/graysurf/nils-cli/pull/221"),
            Some(221)
        );
    }
}
