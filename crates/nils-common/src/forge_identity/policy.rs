use super::{Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    ApiRead,
    ApiWrite,
    GitRead,
    GitPush,
    Commit,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Activation {
    #[default]
    Always,
    AssertedOnly,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    pub version: u32,
    #[serde(default)]
    pub activation: Activation,
    #[serde(default)]
    pub require_session_binding: bool,
    #[serde(default)]
    pub launch_rules: Vec<LaunchRule>,
    pub principals: BTreeMap<String, Principal>,
    pub profiles: BTreeMap<String, Profile>,
    pub credentials: BTreeMap<String, Credential>,
    #[serde(default)]
    pub rules: Vec<Rule>,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LaunchRule {
    pub id: String,
    pub initiator: String,
    pub role: Option<String>,
    pub principal: String,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Principal {
    pub profiles: Vec<String>,
    pub default_profile: Option<String>,
    #[serde(default)]
    pub default_repositories: Vec<String>,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub expected_login: Option<String>,
    pub expected_app_id: Option<u64>,
    pub app_slug: Option<String>,
    pub credential: String,
    pub commit_name: String,
    pub commit_email: String,
    pub signing_fingerprint: String,
    pub operations: Vec<Operation>,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Credential {
    GhUser { user: String },
    Env { name: String },
}
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    pub id: String,
    pub principal: String,
    pub profile: String,
    pub repo: Option<String>,
    pub org: Option<String>,
    pub path: Option<PathBuf>,
    #[serde(default)]
    pub repositories: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Target {
    pub host: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub repo: String,
}
impl Target {
    pub fn new(host: &str, repo: &str) -> Result<Self> {
        let host = canonical_host(host)?;
        let parts: Vec<_> = repo.split('/').collect();
        if parts.len() != 2
            || parts
                .iter()
                .any(|p| !identifier(p) || *p == "." || *p == "..")
        {
            return Err(Error::new("identity_target_invalid"));
        }
        Ok(Self {
            host,
            repo: repo.to_ascii_lowercase(),
        })
    }
    /// Host-only scope for cross-repository API reads. Repository writes and Git
    /// operations still require a concrete repository through `new`.
    pub fn cross_repository(host: &str) -> Result<Self> {
        Ok(Self {
            host: canonical_host(host)?,
            repo: String::new(),
        })
    }
    pub fn key(&self) -> String {
        if self.repo.is_empty() {
            self.host.clone()
        } else {
            format!("{}/{}", self.host, self.repo)
        }
    }
    pub fn org(&self) -> String {
        format!("{}/{}", self.host, self.repo.split('/').next().unwrap())
    }
    pub fn from_key(key: &str) -> Result<Self> {
        let (host, repo) = key
            .split_once('/')
            .ok_or(Error::new("identity_target_invalid"))?;
        Self::new(host, repo)
    }
}
fn canonical_host(host: &str) -> Result<String> {
    if !host
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
        || host.is_empty()
        || host.starts_with('-')
    {
        return Err(Error::new("identity_target_invalid"));
    }
    Ok(crate::git::canonical_git_host(host))
}
pub(super) fn identifier(s: &str) -> bool {
    !crate::redact::token_secret_regex().is_match(s)
        && !s.is_empty()
        && s.len() <= 128
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}
impl Policy {
    pub fn parse(text: &str) -> Result<Self> {
        // Do not return TOML diagnostics: they include source lines and unknown field values.
        let policy: Self =
            toml::from_str(text).map_err(|_| Error::new("identity_policy_invalid"))?;
        policy.validate()?;
        Ok(policy)
    }
    fn validate(&self) -> Result<()> {
        let invalid = || Error::new("identity_policy_invalid");
        if self.version != 1 {
            return Err(Error::new("identity_policy_version"));
        }
        if self.principals.is_empty() || self.profiles.is_empty() || self.credentials.is_empty() {
            return Err(invalid());
        }
        for (id, credential) in &self.credentials {
            if !identifier(id) {
                return Err(invalid());
            }
            match credential {
                Credential::GhUser { user } if !identifier(user) => return Err(invalid()),
                Credential::Env { name }
                    if name.is_empty()
                        || !name.bytes().enumerate().all(|(i, b)| {
                            b == b'_' || b.is_ascii_alphabetic() || (i > 0 && b.is_ascii_digit())
                        }) =>
                {
                    return Err(invalid());
                }
                _ => {}
            }
        }
        for (id, profile) in &self.profiles {
            let fingerprint = &profile.signing_fingerprint;
            if !identifier(id)
                || !self.credentials.contains_key(&profile.credential)
                || profile.operations.is_empty()
                || ![40, 64].contains(&fingerprint.len())
                || !fingerprint.bytes().all(|b| b.is_ascii_hexdigit())
                || crate::redact::token_secret_regex().is_match(&profile.commit_name)
                || crate::redact::token_secret_regex().is_match(&profile.commit_email)
                || profile.commit_name.trim().is_empty()
                || profile.commit_name.contains(['\n', '\r', '<', '>'])
                || !profile.commit_email.contains('@')
                || profile.commit_email.contains(['\n', '\r', ' ', '<', '>'])
            {
                return Err(invalid());
            }
            match (
                &profile.expected_login,
                profile.expected_app_id,
                &profile.app_slug,
            ) {
                (Some(login), None, None) if identifier(login) => {}
                (None, Some(id), Some(slug)) if id > 0 && identifier(slug) => {}
                _ => return Err(invalid()),
            }
        }
        for (id, principal) in &self.principals {
            if !identifier(id)
                || principal.profiles.is_empty()
                || principal
                    .profiles
                    .iter()
                    .any(|p| !self.profiles.contains_key(p))
                || principal
                    .default_profile
                    .as_ref()
                    .is_some_and(|p| !principal.profiles.contains(p))
                || principal.default_profile.is_some() == principal.default_repositories.is_empty()
            {
                return Err(invalid());
            }
            for repo in &principal.default_repositories {
                if Target::from_key(repo)?.key() != *repo {
                    return Err(invalid());
                }
            }
        }
        let mut launch_ids = BTreeSet::new();
        for rule in &self.launch_rules {
            if !identifier(&rule.id)
                || !launch_ids.insert(&rule.id)
                || !identifier(&rule.initiator)
                || rule.role.as_ref().is_some_and(|role| !identifier(role))
                || !self.principals.contains_key(&rule.principal)
            {
                return Err(invalid());
            }
        }
        let mut ids = BTreeSet::new();
        for rule in &self.rules {
            let Some(principal) = self.principals.get(&rule.principal) else {
                return Err(invalid());
            };
            if !identifier(&rule.id)
                || !ids.insert(&rule.id)
                || !principal.profiles.contains(&rule.profile)
                || [rule.repo.is_some(), rule.org.is_some(), rule.path.is_some()]
                    .iter()
                    .filter(|x| **x)
                    .count()
                    != 1
            {
                return Err(invalid());
            }
            if let Some(repo) = &rule.repo
                && Target::from_key(repo)?.key() != *repo
            {
                return Err(invalid());
            }
            if let Some(org) = &rule.org {
                let probe = Target::from_key(&format!("{org}/probe"))?;
                if probe.org() != *org {
                    return Err(invalid());
                }
            }
            if let Some(path) = &rule.path {
                if !path.is_absolute()
                    || rule.repositories.is_empty()
                    || !std::fs::canonicalize(path).is_ok_and(|canonical| canonical == *path)
                    || path
                        .components()
                        .any(|c| matches!(c, std::path::Component::ParentDir))
                {
                    return Err(invalid());
                }
            } else if !rule.repositories.is_empty() {
                return Err(invalid());
            }
            for repo in &rule.repositories {
                if Target::from_key(repo)?.key() != *repo {
                    return Err(invalid());
                }
            }
        }
        Ok(())
    }
    pub fn resolve(
        &self,
        principal: &str,
        target: &Target,
        managed_path: Option<&Path>,
        operation: Operation,
    ) -> Result<Selection> {
        let entry = self
            .principals
            .get(principal)
            .ok_or(Error::new("identity_principal_unknown"))?;
        if target.repo.is_empty() {
            if operation != Operation::ApiRead {
                return Err(Error::new("identity_operation_denied"));
            }
            let candidates: BTreeSet<_> = entry.profiles.iter().collect();
            if candidates.len() != 1 {
                return Err(Error::with_detail(
                    "identity_target_ambiguous",
                    format!(
                        "candidate profiles: {}; pass --repo owner/repo to apply repository rules, or set FORGE_IDENTITY_PRINCIPAL to a principal with exactly one profile",
                        candidates
                            .into_iter()
                            .cloned()
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                ));
            }
            let profile_id = (*candidates.first().unwrap()).clone();
            let host_matches = |key: &str| {
                key.split_once('/')
                    .is_some_and(|(host, _)| host == target.host)
            };
            // Host-only reads ignore checkout scope, but a credential must stay
            // within authorities declared for this principal and profile.
            let host_declared = self.rules.iter().any(|rule| {
                rule.principal == principal
                    && rule.profile == profile_id
                    && (rule.repo.as_deref().is_some_and(host_matches)
                        || rule.org.as_deref().is_some_and(host_matches)
                        || rule.repositories.iter().any(|key| host_matches(key)))
            }) || (entry.default_profile.as_deref()
                == Some(profile_id.as_str())
                && entry
                    .default_repositories
                    .iter()
                    .any(|key| host_matches(key)));
            if !host_declared {
                return Err(Error::new("identity_repository_unknown"));
            }
            let profile = self.profiles.get(&profile_id).unwrap().clone();
            if !profile.operations.contains(&operation) {
                return Err(Error::new("identity_operation_denied"));
            }
            return Ok(Selection {
                principal: principal.to_string(),
                target: target.clone(),
                operation,
                profile_id,
                matched_rule: "principal-single-profile".to_string(),
                profile,
                session_binding: None,
            });
        }
        let key = target.key();
        let rules: Vec<_> = self
            .rules
            .iter()
            .filter(|r| r.principal == principal)
            .collect();
        let paths: Vec<_> = rules
            .iter()
            .copied()
            .filter(|r| {
                r.path
                    .as_ref()
                    .is_some_and(|p| managed_path.is_some_and(|m| m.starts_with(p)))
            })
            .collect();
        if paths.iter().any(|r| !r.repositories.contains(&key)) {
            return Err(Error::new("identity_path_conflict"));
        }
        let repos: Vec<_> = rules
            .iter()
            .copied()
            .filter(|r| r.repo.as_deref() == Some(&key))
            .collect();
        let orgs: Vec<_> = rules
            .iter()
            .copied()
            .filter(|r| r.org.as_deref() == Some(&target.org()))
            .collect();
        let chosen = [&repos, &orgs, &paths]
            .into_iter()
            .find(|group| !group.is_empty());
        let (profile_id, matched_rule) = if let Some(group) = chosen {
            if group.len() != 1 {
                return Err(Error::new("identity_rule_ambiguous"));
            }
            let rule = group[0];
            if paths.iter().any(|r| r.profile != rule.profile) {
                return Err(Error::new("identity_path_conflict"));
            }
            (rule.profile.clone(), rule.id.clone())
        } else {
            if !entry.default_repositories.contains(&key) {
                return Err(Error::new("identity_repository_unknown"));
            }
            (
                entry
                    .default_profile
                    .clone()
                    .ok_or(Error::new("identity_repository_unknown"))?,
                "principal-default".to_string(),
            )
        };
        let profile = self.profiles.get(&profile_id).unwrap().clone();
        if !profile.operations.contains(&operation) {
            return Err(Error::new("identity_operation_denied"));
        }
        Ok(Selection {
            principal: principal.to_string(),
            target: target.clone(),
            operation,
            profile_id,
            matched_rule,
            profile,
            session_binding: None,
        })
    }
}
#[derive(Debug, Clone, Serialize)]
pub struct Selection {
    pub principal: String,
    pub target: Target,
    pub operation: Operation,
    pub profile_id: String,
    pub matched_rule: String,
    pub profile: Profile,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_binding: Option<super::session::SessionDecision>,
}
