//! Pure store-path resolution.
//!
//! This module owns the metadata-only logic ported from the bash script's
//! `slug()` and `store_file()` helpers: where the store lives, how a repo's
//! `origin` remote maps to an `owner/repo` slug, and how a `[name]` argument
//! resolves to a store entry. None of these functions touch secret *values* —
//! they deal only with paths and slugs, which keeps them trivially testable
//! without a real `sops` or `git`.

use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use nils_common::git::parse_git_remote_url;

/// Selection metadata explaining why a store root was chosen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreSelection {
    pub root: PathBuf,
    pub source: String,
    pub matched_by: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct StoreConfig {
    default: Option<PathBuf>,
    #[serde(alias = "paths")]
    path_prefixes: BTreeMap<PathBuf, PathBuf>,
    remotes: BTreeMap<String, PathBuf>,
}

/// Resolve the store using env override, longest checkout prefix, longest
/// remote selector, configured default, then the neutral XDG data location.
pub fn select_store(
    secrets_repo_env: Option<&str>,
    config_path: &Path,
    cwd: &Path,
    remote: Option<&str>,
    data_home: Option<&Path>,
) -> Result<StoreSelection, String> {
    if let Some(value) = secrets_repo_env.filter(|value| !value.trim().is_empty()) {
        return Ok(StoreSelection {
            root: PathBuf::from(value),
            source: "SECRETS_REPO".to_string(),
            matched_by: None,
        });
    }

    let config = match std::fs::read_to_string(config_path) {
        Ok(contents) => toml::from_str::<StoreConfig>(&contents)
            .map_err(|_| format!("invalid store configuration at {}", config_path.display()))?,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => StoreConfig::default(),
        Err(_) => {
            return Err(format!(
                "cannot read store configuration at {}",
                config_path.display()
            ));
        }
    };
    let config_dir = config_path.parent().unwrap_or_else(|| Path::new("."));
    let resolve = |path: PathBuf| {
        if path.is_absolute() {
            path
        } else {
            config_dir.join(path)
        }
    };

    let cwd = lexical_normalize(cwd);
    if let Some((prefix, path)) = config
        .path_prefixes
        .iter()
        .filter_map(|(prefix, path)| {
            let prefix = lexical_normalize(prefix);
            cwd.starts_with(&prefix).then_some((prefix, path))
        })
        .max_by_key(|(prefix, _)| prefix.components().count())
    {
        return Ok(StoreSelection {
            root: resolve(path.clone()),
            source: "path-prefix".to_string(),
            matched_by: Some(prefix.display().to_string()),
        });
    }

    if let Some(remote) = remote.and_then(parse_git_remote_url) {
        let remote_key = format!(
            "{}/{}",
            remote.host.to_ascii_lowercase(),
            remote.path.to_ascii_lowercase()
        );
        if let Some((selector, path)) = config
            .remotes
            .iter()
            .filter(|(selector, _)| {
                remote_key == selector.to_ascii_lowercase()
                    || remote_key
                        .strip_prefix(&selector.to_ascii_lowercase())
                        .is_some_and(|suffix| suffix.starts_with('/'))
            })
            .max_by_key(|(selector, _)| selector.len())
        {
            return Ok(StoreSelection {
                root: resolve(path.clone()),
                source: "remote".to_string(),
                matched_by: Some(selector.clone()),
            });
        }
    }

    if let Some(path) = config.default {
        return Ok(StoreSelection {
            root: resolve(path),
            source: "config-default".to_string(),
            matched_by: None,
        });
    }

    let root = data_home
        .map(|base| base.join("secrets/store"))
        .ok_or_else(|| {
            "no store selected; set SECRETS_REPO or configure stores.toml".to_string()
        })?;
    Ok(StoreSelection {
        root,
        source: "xdg-default".to_string(),
        matched_by: None,
    })
}

fn lexical_normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// The `.enc.env` suffix every encrypted store entry carries.
pub const ENC_SUFFIX: &str = ".enc.env";

/// Derive the `owner/repo` (or nested `group/.../repo`) slug from a git remote
/// URL, mirroring the bash `slug()` sed pipeline but reusing the workspace's
/// canonical URL parser so SCP, ssh, http(s), ports, userinfo, and multi-segment
/// GitLab paths are all handled consistently. Returns `None` when the URL is not
/// a recognizable git remote.
pub fn slug_from_remote_url(remote: &str) -> Option<String> {
    parse_git_remote_url(remote).map(|parsed| parsed.path)
}

/// A resolved store entry: where it lives and how it is referenced relative to
/// the store root (the form shown to users).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreEntry {
    /// Absolute path to the `.enc.env` file in the store.
    pub path: PathBuf,
    /// Store-relative path (e.g. `repos/owner/repo.enc.env`).
    pub rel: String,
    /// Whether the file currently exists on disk.
    pub exists: bool,
}

/// Resolve the store entry for an explicit `[name]` argument, mirroring the bash
/// `store_file` lookup order: try `<name>`, then `repos/<name>`, then
/// `stacks/<name>`, each as both a bare path and with the `.enc.env` suffix
/// appended. When nothing matches on disk, fall back to `repos/<name>.enc.env`
/// (the default target `add` would create), with `exists = false`.
pub fn store_entry_for_name(store_root: &Path, name: &str) -> StoreEntry {
    for cand in [name, &format!("repos/{name}"), &format!("stacks/{name}")] {
        // Suffixed form: `<cand>.enc.env`.
        let with_suffix = format!("{cand}{ENC_SUFFIX}");
        let suffixed = store_root.join(&with_suffix);
        if suffixed.is_file() {
            return StoreEntry {
                path: suffixed,
                rel: with_suffix,
                exists: true,
            };
        }
        // Bare form: `<cand>` exactly as given (already carries an extension).
        let bare = store_root.join(cand);
        if bare.is_file() {
            return StoreEntry {
                path: bare,
                rel: cand.to_string(),
                exists: true,
            };
        }
    }

    let rel = format!("repos/{name}{ENC_SUFFIX}");
    StoreEntry {
        path: store_root.join(&rel),
        rel,
        exists: false,
    }
}

/// Resolve the store entry for the auto-detected repo slug
/// (`repos/<slug>.enc.env`).
pub fn store_entry_for_slug(store_root: &Path, slug: &str) -> StoreEntry {
    let rel = format!("repos/{slug}{ENC_SUFFIX}");
    let path = store_root.join(&rel);
    let exists = path.is_file();
    StoreEntry { path, rel, exists }
}

/// List every `*.enc.env` entry under `stacks/` and `repos/`, returned as
/// sorted store-relative paths with the `.enc.env` suffix stripped (matching the
/// bash `list` command). Only entry *names* are returned — never contents.
pub fn list_entries(store_root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    for top in ["stacks", "repos"] {
        collect_enc_env(&store_root.join(top), top, &mut out);
    }
    out.sort();
    out
}

fn collect_enc_env(dir: &Path, rel_prefix: &str, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let child_rel = format!("{rel_prefix}/{name}");
        if file_type.is_dir() {
            collect_enc_env(&path, &child_rel, out);
        } else if let Some(stripped) = child_rel.strip_suffix(ENC_SUFFIX) {
            out.push(stripped.to_string());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn store_selection_precedence_and_explanation() {
        let tmp = TempDir::new().expect("tempdir");
        let config = tmp.path().join("stores.toml");
        fs::write(
            &config,
            r##"default = "default-store"
[path_prefixes]
"/work" = "work-store"
"/work/team" = "team-store"
[remotes]
"github.com" = "host-store"
"github.com/acme" = "owner-store"
"github.com/acme/service" = "repo-store"
"gitlab.example/acme/platform" = "nested-store"
"github.com/acme/other" = "other-store"
"##,
        )
        .expect("config");
        let selected = select_store(
            None,
            &config,
            Path::new("/work/team/app"),
            Some("https://github.com/acme/service.git"),
            None,
        )
        .unwrap();
        assert_eq!(selected.root, tmp.path().join("team-store"));
        assert_eq!(selected.source, "path-prefix");
        assert_eq!(selected.matched_by.as_deref(), Some("/work/team"));
        let selected = select_store(
            None,
            &config,
            Path::new("/elsewhere"),
            Some("git@github.com:acme/service.git"),
            None,
        )
        .unwrap();
        assert_eq!(selected.root, tmp.path().join("repo-store"));
        assert_eq!(selected.source, "remote");
        assert_eq!(
            selected.matched_by.as_deref(),
            Some("github.com/acme/service")
        );
        let selected = select_store(None, &config, Path::new("/elsewhere"), None, None).unwrap();
        assert_eq!(selected.root, tmp.path().join("default-store"));
        let selected = select_store(
            Some("/override"),
            &config,
            Path::new("/work/team/app"),
            None,
            None,
        )
        .unwrap();
        assert_eq!(selected.root, PathBuf::from("/override"));
        assert_eq!(selected.source, "SECRETS_REPO");
        let selected = select_store(
            None,
            &tmp.path().join("missing.toml"),
            Path::new("/elsewhere"),
            None,
            Some(Path::new("/xdg")),
        )
        .unwrap();
        assert_eq!(selected.root, PathBuf::from("/xdg/secrets/store"));
        assert_eq!(selected.source, "xdg-default");
    }

    #[test]
    fn slug_handles_scp_https_and_nested_gitlab() {
        assert_eq!(
            slug_from_remote_url("git@github.com:example/service.git").as_deref(),
            Some("example/service")
        );
        assert_eq!(
            slug_from_remote_url("https://github.com/example/service").as_deref(),
            Some("example/service")
        );
        assert_eq!(
            slug_from_remote_url("https://gitlab.example.com/acme/platform/backend/svc.git")
                .as_deref(),
            Some("acme/platform/backend/svc")
        );
        assert_eq!(slug_from_remote_url("not a url"), None);
    }

    #[test]
    fn store_entry_for_slug_reports_existence() {
        let tmp = TempDir::new().expect("tempdir");
        let entry = store_entry_for_slug(tmp.path(), "owner/repo");
        assert_eq!(entry.rel, "repos/owner/repo.enc.env");
        assert!(!entry.exists);

        fs::create_dir_all(tmp.path().join("repos/owner")).expect("mkdir");
        fs::write(tmp.path().join("repos/owner/repo.enc.env"), "x").expect("write");
        let entry = store_entry_for_slug(tmp.path(), "owner/repo");
        assert!(entry.exists);
    }

    #[test]
    fn store_entry_for_name_lookup_order_and_fallback() {
        let tmp = TempDir::new().expect("tempdir");
        fs::create_dir_all(tmp.path().join("stacks")).expect("mkdir");
        fs::write(tmp.path().join("stacks/web.enc.env"), "x").expect("write");

        // Bare name resolves through `stacks/<name>.enc.env`.
        let entry = store_entry_for_name(tmp.path(), "web");
        assert_eq!(entry.rel, "stacks/web.enc.env");
        assert!(entry.exists);

        // Explicit `stacks/web` resolves the same file.
        let entry = store_entry_for_name(tmp.path(), "stacks/web");
        assert_eq!(entry.rel, "stacks/web.enc.env");
        assert!(entry.exists);

        // Unknown name falls back to the default add target, not existing.
        let entry = store_entry_for_name(tmp.path(), "ghost");
        assert_eq!(entry.rel, "repos/ghost.enc.env");
        assert!(!entry.exists);
    }

    #[test]
    fn list_entries_strips_suffix_and_sorts() {
        let tmp = TempDir::new().expect("tempdir");
        fs::create_dir_all(tmp.path().join("repos/owner")).expect("mkdir");
        fs::create_dir_all(tmp.path().join("stacks")).expect("mkdir");
        fs::write(tmp.path().join("repos/owner/repo.enc.env"), "x").expect("write");
        fs::write(tmp.path().join("stacks/web.enc.env"), "x").expect("write");
        // Non-matching files are ignored.
        fs::write(tmp.path().join("stacks/README.md"), "x").expect("write");

        let entries = list_entries(tmp.path());
        assert_eq!(
            entries,
            vec!["repos/owner/repo".to_string(), "stacks/web".to_string()]
        );
    }
}
