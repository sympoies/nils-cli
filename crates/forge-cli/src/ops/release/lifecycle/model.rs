//! Normalize release identity and resolve a tag to an actual commit object.
use super::super::super::github_write::invalid;
use super::{api, guards};
use crate::error::ForgeError;
use crate::provider::ProviderContext;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
pub(super) struct Asset {
    name: String,
    size: u64,
    digest: Option<String>,
}
#[derive(Deserialize, Clone)]
pub(super) struct Release {
    pub id: u64,
    pub tag_name: String,
    pub target_commitish: String,
    pub html_url: String,
    pub name: Option<String>,
    pub body: Option<String>,
    pub draft: bool,
    pub prerelease: bool,
    pub assets: Vec<Asset>,
}
#[derive(Deserialize, Clone, PartialEq)]
pub(super) struct Object {
    pub r#type: String,
    pub sha: String,
}
#[derive(Serialize)]
pub(super) struct View {
    provider: &'static str,
    repository: String,
    release_id: u64,
    tag: String,
    url: String,
    title: Option<String>,
    target_commitish: String,
    target_commit: String,
    draft: bool,
    prerelease: bool,
    notes: String,
    assets: Vec<Asset>,
}
impl View {
    pub fn new(repo: &str, release: Release, commit: String) -> Self {
        Self {
            provider: "github",
            repository: repo.into(),
            release_id: release.id,
            tag: release.tag_name,
            url: release.html_url,
            title: release.name,
            target_commitish: release.target_commitish,
            target_commit: commit,
            draft: release.draft,
            prerelease: release.prerelease,
            notes: release.body.unwrap_or_default(),
            assets: release.assets,
        }
    }
    pub fn print(&self) {
        println!(
            "{} release #{} ({})\ntag: {}\ntarget: {} ({})\ndraft: {}\nprerelease: {}\ntitle: {}\n{}",
            self.repository,
            self.release_id,
            self.url,
            self.tag,
            self.target_commit,
            self.target_commitish,
            self.draft,
            self.prerelease,
            self.title.as_deref().unwrap_or(""),
            self.notes
        );
        for asset in &self.assets {
            println!(
                "asset: {} size={} digest={}",
                asset.name,
                asset.size,
                asset.digest.as_deref().unwrap_or("null")
            );
        }
    }
}
fn decode<T: serde::de::DeserializeOwned>(value: Value, op: &str) -> Result<T, ForgeError> {
    serde_json::from_value(value).map_err(|_| {
        invalid(
            op,
            "release_response_invalid",
            "provider release/tag response is incomplete or malformed",
        )
    })
}
pub(super) fn release(value: Value, tag: &str, op: &str) -> Result<Release, ForgeError> {
    let release: Release = decode(value, op)?;
    if release.id == 0 || release.tag_name != tag {
        return Err(invalid(
            op,
            "release_response_mismatch",
            "provider release identity differs from the selected tag; no further mutation is safe",
        ));
    }
    Ok(release)
}
pub(super) fn tag_path(repo: &str, tag: &str) -> String {
    format!("repos/{repo}/git/ref/tags/{}", guards::encode(tag))
}
pub(super) fn tag_object(
    ctx: &ProviderContext,
    repo: &str,
    tag: &str,
    op: &str,
) -> Result<Option<Object>, ForgeError> {
    let Some(value) = api::get(ctx, op, &tag_path(repo, tag))? else {
        return Ok(None);
    };
    if value["ref"].as_str() != Some(&format!("refs/tags/{tag}")) {
        return Err(invalid(
            op,
            "release_tag_mismatch",
            "provider returned a different tag ref; inspect the exact selector",
        ));
    }
    let object: Object = decode(value["object"].clone(), op)?;
    check_object(&object, op)?;
    Ok(Some(object))
}
fn check_object(object: &Object, op: &str) -> Result<(), ForgeError> {
    if object.sha.len() != 40
        || !object.sha.bytes().all(|b| b.is_ascii_hexdigit())
        || !matches!(object.r#type.as_str(), "commit" | "tag")
    {
        return Err(invalid(
            op,
            "release_target_unresolved",
            "tag did not resolve to a Git commit or annotated tag object",
        ));
    }
    Ok(())
}
pub(super) fn resolve(
    ctx: &ProviderContext,
    repo: &str,
    release: &Release,
    object: Option<Object>,
    op: &str,
) -> Result<String, ForgeError> {
    let mut object = match object {
        Some(object) => object,
        None if release.draft => {
            // A draft may precede tag creation. Resolve its target through the
            // commit API; target_commitish itself is never presented as a SHA.
            let path = format!(
                "repos/{repo}/commits/{}",
                guards::encode(&release.target_commitish)
            );
            let value = api::get(ctx, op, &path)?.ok_or_else(|| {
                invalid(
                    op,
                    "release_target_unresolved",
                    "draft target commit is unavailable",
                )
            })?;
            Object {
                r#type: "commit".into(),
                sha: value["sha"].as_str().unwrap_or_default().into(),
            }
        }
        None => {
            return Err(invalid(
                op,
                "release_target_unresolved",
                "release tag is absent; inspect the release and restore its tag before retrying",
            ));
        }
    };
    for _ in 0..16 {
        check_object(&object, op)?;
        if object.r#type == "commit" {
            return Ok(object.sha);
        }
        let path = format!("repos/{repo}/git/tags/{}", object.sha);
        let value = api::get(ctx, op, &path)?.ok_or_else(|| {
            invalid(
                op,
                "release_target_unresolved",
                "annotated tag object is unavailable",
            )
        })?;
        if value["sha"].as_str() != Some(&object.sha) {
            return Err(invalid(
                op,
                "release_target_unresolved",
                "annotated tag identity does not match its object SHA",
            ));
        }
        object = decode(value["object"].clone(), op)?;
    }
    Err(invalid(
        op,
        "release_target_unresolved",
        "annotated tag nesting exceeds the resolution limit",
    ))
}
