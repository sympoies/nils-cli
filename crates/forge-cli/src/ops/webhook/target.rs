//! Where a webhook lives.
use nils_common::forge_identity::{self as identity, Target};

/// A repository (`OWNER/NAME`) or an organization login.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum HookTarget {
    Repo(String),
    Org(String),
}

fn valid_segment(part: &str) -> bool {
    !part.is_empty()
        && part != "."
        && part != ".."
        && part
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

impl HookTarget {
    pub(crate) fn name(&self) -> &str {
        match self {
            Self::Repo(name) | Self::Org(name) => name,
        }
    }

    pub(crate) fn valid(&self) -> bool {
        match self {
            Self::Repo(repo) => {
                matches!(repo.split_once('/'), Some((owner, name)) if valid_segment(owner) && valid_segment(name))
            }
            Self::Org(org) => valid_segment(org),
        }
    }

    /// REST path of this target's hook collection plus `tail`.
    pub(crate) fn path(&self, tail: &str) -> String {
        match self {
            Self::Repo(repo) => format!("repos/{repo}/hooks{tail}"),
            Self::Org(org) => format!("orgs/{org}/hooks{tail}"),
        }
    }

    /// Identity scope under which this target's calls are authorized.
    pub(crate) fn identity(&self, host: &str) -> identity::Result<Target> {
        match self {
            Self::Repo(repo) => Target::new(host, repo),
            Self::Org(org) => Target::organization(host, org),
        }
    }
}
