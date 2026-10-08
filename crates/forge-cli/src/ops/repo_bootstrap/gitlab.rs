//! GitLab project reads and mutations for the shared bounded bootstrap flow.

use super::*;
use crate::ops::gitlab_api::encode_project_path;

pub(super) struct GitlabClient {
    pub(super) context: ProviderContext,
    runner: Box<dyn BackendRunner>,
}

impl GitlabClient {
    pub(super) fn from_global(global: &GlobalFlags) -> Result<Self, ForgeError> {
        let context = crate::provider::detect(
            global.provider_hint(),
            &global.remote,
            global.repo.as_deref(),
            |_| None,
        )?;
        Ok(Self {
            context,
            runner: Box::new(default_runner()),
        })
    }

    fn run_glab(&self, args: &[OsString]) -> Result<ProcessResult, ForgeError> {
        let call = BackendCall::new(BackendProgram::Glab, args.iter().cloned())
            .with_host(Provider::GitLab, &self.context.host);
        let output = self
            .runner
            .run_raw_with_timeout(&call, Some(PROCESS_TIMEOUT))?;
        Ok(ProcessResult {
            success: output.status_success,
            code: output.exit_code,
            http_status: None,
            stdout: output.stdout,
            stderr: output.stderr,
        })
    }

    fn api_result(&self, endpoint: &str, tail: &[OsString]) -> Result<ProcessResult, ForgeError> {
        let mut args = vec![
            "api".into(),
            "--include".into(),
            "--hostname".into(),
            self.context.host.clone().into(),
            endpoint.into(),
        ];
        args.extend_from_slice(tail);
        let mut result = self.run_glab(&args)?;
        // Both provider CLIs emit the HTTP status line before the JSON body.
        match split_bootstrap_http_response(&result.stdout) {
            Ok((status, body)) => {
                result.http_status = Some(status);
                result.stdout = body.to_string();
            }
            Err(_) if result.success => {
                return Err(software("GitLab API response omitted valid HTTP headers"));
            }
            Err(_) => {}
        }
        Ok(result)
    }

    fn decode(&self, result: ProcessResult) -> Result<serde_json::Value, ForgeError> {
        let result = require_success(
            result,
            "bootstrap_gitlab_api_failed",
            "GitLab bootstrap API request failed",
        )?;
        serde_json::from_str(&result.stdout).map_err(|error| {
            unavailable(
                "bootstrap_gitlab_api_invalid",
                "GitLab bootstrap API returned invalid JSON",
                Some(error.to_string()),
            )
        })
    }

    pub(super) fn api_json(
        &self,
        endpoint: &str,
        tail: &[OsString],
    ) -> Result<serde_json::Value, ForgeError> {
        self.decode(self.api_result(endpoint, tail)?)
    }

    pub(super) fn api_optional(
        &self,
        endpoint: &str,
    ) -> Result<Option<serde_json::Value>, ForgeError> {
        let result = self.api_result(endpoint, &[])?;
        if !result.success && result.http_status == Some(404) {
            return Ok(None);
        }
        self.decode(result).map(Some)
    }

    pub(super) fn project_endpoint(owner: &str, repo: &str) -> String {
        format!(
            "projects/{}",
            encode_project_path(&format!("{owner}/{repo}"))
        )
    }

    pub(super) fn authenticated_user(&self) -> Result<String, ForgeError> {
        self.api_json("user", &[])?
            .get("username")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| software("GitLab authenticated user response omitted username"))
    }

    pub(super) fn create_repo(
        &self,
        kind: RepoBootstrapOwnerKind,
        owner: &str,
        repo: &str,
        private: bool,
    ) -> Result<(), ForgeError> {
        let namespace =
            self.api_json(&format!("namespaces/{}", encode_project_path(owner)), &[])?;
        verify_namespace(&namespace, owner, kind)?;
        let id = namespace
            .get("id")
            .and_then(serde_json::Value::as_u64)
            .filter(|id| *id > 0)
            .ok_or_else(|| software("GitLab namespace response omitted a valid id"))?;
        let args = [
            "--method".into(),
            "POST".into(),
            "-f".into(),
            format!("name={repo}").into(),
            "-f".into(),
            format!("path={repo}").into(),
            "-F".into(),
            format!("namespace_id={id}").into(),
            "-f".into(),
            format!("visibility={}", if private { "private" } else { "public" }).into(),
            "-F".into(),
            "initialize_with_readme=false".into(),
        ];
        self.api_json("projects", &args).map(|_| ())
    }

    fn refs(
        &self,
        owner: &str,
        repo: &str,
        kind: &str,
    ) -> Result<Vec<serde_json::Value>, ForgeError> {
        // Two entries suffice to detect any extra ref. A later page implies a
        // full first page, which already fails the one-branch bootstrap contract.
        let value = self.api_json(
            &format!(
                "{}/repository/{kind}?per_page=2",
                Self::project_endpoint(owner, repo)
            ),
            &[],
        )?;
        value
            .as_array()
            .cloned()
            .ok_or_else(|| software("GitLab references response was not an array"))
    }

    pub(super) fn remote_empty(&self, owner: &str, repo: &str) -> Result<bool, ForgeError> {
        Ok(self.refs(owner, repo, "branches")?.is_empty()
            && self.refs(owner, repo, "tags")?.is_empty())
    }

    pub(super) fn parse_repo_snapshot(
        &self,
        value: &serde_json::Value,
        owner: &str,
        repo: &str,
        kind: RepoBootstrapOwnerKind,
        private: bool,
    ) -> Result<RepoSnapshot, ForgeError> {
        verify_namespace(
            value
                .get("namespace")
                .ok_or_else(|| software("GitLab project omitted namespace"))?,
            owner,
            kind,
        )?;
        let path = format!("{owner}/{repo}");
        let expected_url = format!("https://{}/{path}.git", self.context.host);
        let visibility = if private { "private" } else { "public" };
        if value
            .get("path_with_namespace")
            .and_then(serde_json::Value::as_str)
            != Some(path.as_str())
            || value.get("path").and_then(serde_json::Value::as_str) != Some(repo)
            || value.get("visibility").and_then(serde_json::Value::as_str) != Some(visibility)
            || value
                .get("http_url_to_repo")
                .and_then(serde_json::Value::as_str)
                != Some(expected_url.as_str())
        {
            return Err(validation(
                "remote_drift",
                "GitLab project read-back does not match the requested namespace, path, visibility, and clone URL",
                None,
            ));
        }
        let empty = value
            .get("empty_repo")
            .and_then(serde_json::Value::as_bool)
            .ok_or_else(|| software("GitLab project read-back omitted empty_repo"))?;
        Ok(RepoSnapshot {
            clone_url: expected_url,
            default_branch: value
                .get("default_branch")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string(),
            empty: empty && self.remote_empty(owner, repo)?,
        })
    }

    pub(super) fn branch_optional(
        &self,
        owner: &str,
        repo: &str,
        branch: &str,
    ) -> Result<Option<serde_json::Value>, ForgeError> {
        self.api_optional(&format!(
            "{}/repository/branches/{}",
            Self::project_endpoint(owner, repo),
            encode_project_path(branch)
        ))
    }

    pub(super) fn verify_final_refs(
        &self,
        owner: &str,
        repo: &str,
        branch: &str,
        sha: &str,
    ) -> Result<(), ForgeError> {
        let branches = self.refs(owner, repo, "branches")?;
        if branches.len() != 1
            || branches[0].get("name").and_then(serde_json::Value::as_str) != Some(branch)
            || branches[0]
                .pointer("/commit/id")
                .and_then(serde_json::Value::as_str)
                != Some(sha)
            || !self.refs(owner, repo, "tags")?.is_empty()
        {
            return Err(validation(
                "remote_drift",
                "GitLab bootstrap requires exactly the delivered root branch and no other refs",
                None,
            ));
        }
        Ok(())
    }

    pub(super) fn update_default_branch(
        &self,
        owner: &str,
        repo: &str,
        branch: &str,
    ) -> Result<(), ForgeError> {
        self.api_json(
            &Self::project_endpoint(owner, repo),
            &[
                "--method".into(),
                "PUT".into(),
                "-f".into(),
                format!("default_branch={branch}").into(),
            ],
        )
        .map(|_| ())
    }

    pub(super) fn commit(
        &self,
        owner: &str,
        repo: &str,
        sha: &str,
    ) -> Result<serde_json::Value, ForgeError> {
        let endpoint = format!(
            "{}/repository/commits/{sha}",
            Self::project_endpoint(owner, repo)
        );
        let mut commit = self.api_json(&endpoint, &[])?;
        let signature = self.api_optional(&format!("{endpoint}/signature"))?;
        commit
            .as_object_mut()
            .ok_or_else(|| software("GitLab commit response was not an object"))?
            .insert(
                "signature".to_string(),
                signature.unwrap_or(serde_json::Value::Null),
            );
        Ok(commit)
    }

    pub(super) fn verify_provider_signature(
        value: &serde_json::Value,
        sha: &str,
    ) -> Result<(), ForgeError> {
        let observed = value.get("id").and_then(serde_json::Value::as_str);
        if observed != Some(sha) {
            return Err(remote_drift(sha, observed.unwrap_or("<missing>")));
        }
        if value
            .get("parent_ids")
            .and_then(serde_json::Value::as_array)
            .is_none_or(|parents| !parents.is_empty())
        {
            return Err(validation(
                "bootstrap_commit_not_root",
                "GitLab delivered commit must have zero parents",
                None,
            ));
        }
        if value
            .pointer("/signature/verification_status")
            .and_then(serde_json::Value::as_str)
            != Some("verified")
        {
            return Err(validation(
                "provider_signature_unverified",
                "GitLab did not verify the delivered root commit signature",
                None,
            ));
        }
        Ok(())
    }

    pub(super) fn auth_token(&self) -> Result<String, ForgeError> {
        let result = require_success(
            self.run_glab(&[
                "config".into(),
                "get".into(),
                "token".into(),
                "--host".into(),
                self.context.host.clone().into(),
            ])?,
            "bootstrap_gitlab_auth_failed",
            "failed to retrieve the GitLab credential for the bounded push",
        )?;
        let token = result.stdout.trim();
        if token.is_empty() || token.chars().any(char::is_whitespace) {
            return Err(validation(
                "bootstrap_gitlab_auth_invalid",
                "GitLab credential response was empty or malformed",
                None,
            ));
        }
        Ok(token.to_string())
    }
}

fn verify_namespace(
    value: &serde_json::Value,
    owner: &str,
    kind: RepoBootstrapOwnerKind,
) -> Result<(), ForgeError> {
    let expected_kind = match kind {
        RepoBootstrapOwnerKind::User => "user",
        RepoBootstrapOwnerKind::Org => "group",
    };
    if value.get("full_path").and_then(serde_json::Value::as_str) != Some(owner)
        || value.get("kind").and_then(serde_json::Value::as_str) != Some(expected_kind)
    {
        return Err(validation(
            "remote_drift",
            "GitLab namespace read-back does not match the requested path and owner kind",
            None,
        ));
    }
    Ok(())
}

pub(super) fn repo_parts(global: &GlobalFlags) -> Result<(String, String), ForgeError> {
    let value = global.repo.as_deref().unwrap_or_default();
    let valid = value.split('/').all(|part| {
        !part.is_empty()
            && part.len() <= 255
            && part != "."
            && part != ".."
            && part
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    });
    if !valid {
        return Err(validation(
            "repo_invalid",
            "GitLab bootstrap requires --repo namespace/project with safe path components",
            None,
        ));
    }
    value
        .rsplit_once('/')
        .map(|(owner, repo)| (owner.to_string(), repo.to_string()))
        .ok_or_else(|| {
            validation(
                "repo_invalid",
                "GitLab bootstrap requires --repo namespace/project",
                None,
            )
        })
}
