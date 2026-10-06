//! PR/MR holds are independent of repository-wide merge-freeze control issues.

use std::ffi::OsString;

use crate::backend::{BackendCall, BackendProgram, BackendRunner};
use crate::config::ForgeConfig;
use crate::error::ForgeError;
use crate::ops::gitlab_api;
use crate::provider::{Provider, ProviderContext};

const SCHEMA: &str = "cli.forge-cli.pr.merge.v1";

pub(crate) fn validate_config(cfg: &ForgeConfig) -> Result<(), ForgeError> {
    if cfg
        .warnings
        .iter()
        .any(|warning| warning.starts_with("invalid-config-value:merge.hold_labels"))
    {
        return Err(ForgeError::validation(
            SCHEMA,
            "invalid_hold_labels_config",
            "merge.hold_labels must be an array of non-empty label names (at most 128 bytes each, without control characters)",
            None,
        ));
    }
    Ok(())
}

pub(crate) fn ensure_clear<R: BackendRunner>(
    runner: &R,
    ctx: &ProviderContext,
    id: u64,
    hold_labels: &[String],
) -> Result<(), ForgeError> {
    let repo = ctx.repo.as_deref().ok_or_else(unavailable)?;
    let call = match ctx.provider {
        Provider::GitHub => {
            // REST pagination avoids the bounded labels connection in gh pr view.
            let mut argv = vec![
                OsString::from("api"),
                OsString::from("--paginate"),
                OsString::from("--slurp"),
                OsString::from(format!("repos/{repo}/issues/{id}/labels?per_page=100")),
            ];
            ctx.push_github_api_hostname(&mut argv);
            BackendCall::new(BackendProgram::Gh, argv)
        }
        Provider::GitLab => gitlab_api::api_call(
            &ctx.host,
            format!(
                "projects/{}/merge_requests/{id}",
                gitlab_api::encode_project_path(repo)
            ),
        ),
        Provider::Local => return Err(unavailable()),
    };
    let output = runner.run(&call).map_err(|_| unavailable())?;
    let value: serde_json::Value =
        serde_json::from_str(&output.stdout).map_err(|_| unavailable())?;
    let labels = parse_labels(ctx.provider, &value)?;
    for label in hold_labels {
        if labels.contains(&label.as_str()) {
            return Err(ForgeError::validation(
                SCHEMA,
                "pr_hold_active",
                format!(
                    "PR/MR is held by label {label:?}; ask the maintainer or authorized hold owner to remove this label after the hold's release conditions are met, then retry. Head changes do not lift a hold."
                ),
                Some(label.clone()),
            ));
        }
    }
    Ok(())
}

fn unavailable() -> ForgeError {
    ForgeError::unavailable(
        SCHEMA,
        "pr_hold_labels_unavailable",
        "cannot verify the complete current PR/MR hold labels; refresh provider access and retry before merging or enqueueing",
        None,
    )
}

fn parse_labels(provider: Provider, value: &serde_json::Value) -> Result<Vec<&str>, ForgeError> {
    match provider {
        Provider::GitHub => {
            let pages = value
                .as_array()
                .filter(|pages| !pages.is_empty())
                .ok_or_else(unavailable)?;
            let mut labels = Vec::new();
            for page in pages {
                for label in page.as_array().ok_or_else(unavailable)? {
                    labels.push(
                        label
                            .get("name")
                            .and_then(|name| name.as_str())
                            .ok_or_else(unavailable)?,
                    );
                }
            }
            Ok(labels)
        }
        Provider::GitLab => value
            .get("labels")
            .and_then(|labels| labels.as_array())
            .ok_or_else(unavailable)?
            .iter()
            .map(|label| label.as_str().ok_or_else(unavailable))
            .collect(),
        Provider::Local => Err(unavailable()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use serde_json::json;

    #[test]
    fn github_hold_labels_include_later_pages() {
        assert_eq!(
            parse_labels(
                Provider::GitHub,
                &json!([[{"name":"type::bug"}], [{"name":"control::hold"}]] )
            )
            .unwrap(),
            vec!["type::bug", "control::hold"]
        );
    }

    #[test]
    fn hold_label_reads_fail_closed_on_incomplete_or_malformed_data() {
        for value in [
            json!(null),
            json!([]),
            json!([null]),
            json!([[{}]]),
            json!([[{"name":null}]]),
        ] {
            assert_eq!(
                parse_labels(Provider::GitHub, &value).unwrap_err().kind(),
                "pr_hold_labels_unavailable"
            );
        }
        for value in [json!({}), json!({"labels":null}), json!({"labels":[{}]})] {
            assert_eq!(
                parse_labels(Provider::GitLab, &value).unwrap_err().kind(),
                "pr_hold_labels_unavailable"
            );
        }
        assert_eq!(
            parse_labels(Provider::GitHub, &json!([[]])).unwrap(),
            Vec::<&str>::new()
        );
        assert_eq!(
            parse_labels(Provider::GitLab, &json!({"labels":[]})).unwrap(),
            Vec::<&str>::new()
        );
    }
}
