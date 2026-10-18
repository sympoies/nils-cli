//! Remove ambient skill catalogs without changing HOME, cwd, or repository files.
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub(super) fn apply(command: &mut Command, cwd: &Path) -> Result<(), String> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let mut roots = roots(cwd, home.as_deref());
    roots.push(super::child_home::original_codex_home().join("skills"));
    apply_roots(command, roots)
}

pub(super) fn apply_for_home(command: &mut Command, cwd: &Path, home: &Path) -> Result<(), String> {
    apply_roots(command, roots(cwd, Some(home)))
}

fn roots(cwd: &Path, home: Option<&Path>) -> Vec<PathBuf> {
    // Match the isolated child's explicit project marker. Outside a Git
    // project Codex uses cwd alone; unrelated filesystem ancestors are not
    // part of the project skill inventory.
    let boundary = cwd
        .ancestors()
        .find(|ancestor| ancestor.join(".git").exists())
        .unwrap_or(cwd);
    let mut roots = Vec::new();
    for ancestor in cwd.ancestors() {
        roots.extend([
            ancestor.join(".agents/skills"),
            ancestor.join(".codex/skills"),
        ]);
        if ancestor == boundary {
            break;
        }
    }
    if let Some(home) = home {
        roots.push(home.join(".agents/skills"));
        roots.push(home.join(".codex/skills"));
    }
    roots
}

fn apply_roots(command: &mut Command, roots: Vec<PathBuf>) -> Result<(), String> {
    let failure =
        || "isolated-runtime-unsupported: cannot disable ambient skills safely".to_string();
    let mut pending = roots;
    let mut visited = BTreeSet::new();
    let mut skills = BTreeSet::new();
    while let Some(directory) = pending.pop() {
        let canonical = match fs::canonicalize(&directory) {
            Ok(path) => path,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => return Err(failure()),
        };
        if !visited.insert(canonical.clone()) {
            continue;
        }
        if visited.len() > 20_000 {
            return Err(failure());
        }
        for entry in fs::read_dir(canonical).map_err(|_| failure())? {
            let path = entry.map_err(|_| failure())?.path();
            let metadata = match fs::metadata(&path) {
                Ok(metadata) => metadata,
                // A dangling link or removed entry has no readable skill.
                // Permission and other inventory errors remain fail-closed.
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(_) => return Err(failure()),
            };
            if metadata.is_dir() {
                pending.push(path);
            } else if path.file_name().is_some_and(|name| name == "SKILL.md") {
                skills.insert(fs::canonicalize(path).map_err(|_| failure())?);
            }
        }
    }
    // Keep the single override below common per-argument exec limits.
    // Oversized inventories fail closed instead of reaching a platform-specific
    // spawn error. Other argv entries (including the prompt) remain separate.
    const MAX_CONFIG_BYTES: usize = 64 * 1024;
    let mut config = String::from("skills.config=[");
    for path in skills {
        let path = path.to_str().ok_or_else(failure)?;
        let entry = format!(
            "{{path={},enabled=false}},",
            toml::Value::String(path.to_string())
        );
        if config.len() + entry.len() + 1 > MAX_CONFIG_BYTES {
            return Err(
                "isolated-runtime-unsupported: ambient skill inventory exceeds launch budget"
                    .to_string(),
            );
        }
        config.push_str(&entry);
    }
    config.push(']');
    command.args([
        "-c",
        r#"project_root_markers=[".git"]"#,
        "-c",
        "skills.include_instructions=false",
        "-c",
        "skills.bundled.enabled=false",
        "-c",
        "cloud.skills.enabled=false",
    ]);
    command.args(["-c", &config]);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::apply_roots;
    use pretty_assertions::assert_eq;
    use std::fs;
    use std::os::unix::fs::symlink;
    use std::process::Command;

    #[test]
    fn canonical_skill_disables_cover_nested_linked_directories_and_cycles() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("skills with spaces");
        let nested = root.join("nested/skill");
        fs::create_dir_all(&nested).unwrap();
        fs::write(nested.join("SKILL.md"), "description never read").unwrap();
        symlink(&nested, root.join("alias")).unwrap();
        symlink(&root, nested.join("cycle")).unwrap();
        let mut command = Command::new("codex");
        apply_roots(&mut command, vec![root]).unwrap();
        let args = command
            .get_args()
            .map(|arg| arg.to_str().unwrap())
            .collect::<Vec<_>>();
        let config = args
            .iter()
            .find_map(|arg| arg.strip_prefix("skills.config="))
            .unwrap();
        let parsed: toml::Value = toml::from_str(&format!("entries={config}")).unwrap();
        let entries = parsed["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["enabled"].as_bool(), Some(false));
        assert_eq!(
            entries[0]["path"].as_str(),
            nested.join("SKILL.md").canonicalize().unwrap().to_str()
        );
    }

    #[test]
    fn invalid_inventory_fails_without_exposing_its_path() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("private-sentinel");
        fs::write(&root, "not a directory").unwrap();
        let message = apply_roots(&mut Command::new("codex"), vec![root]).unwrap_err();
        assert!(message.starts_with("isolated-runtime-unsupported:"));
        assert!(!message.contains("private-sentinel"));
    }
    #[test]
    fn oversized_skill_inventory_fails_before_constructing_child_argv() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("skills");
        for index in 0..450 {
            let directory = root.join(format!("{index}-{}", "x".repeat(150)));
            fs::create_dir_all(&directory).unwrap();
            fs::write(directory.join("SKILL.md"), "fixture").unwrap();
        }
        let mut command = Command::new("codex");
        let message = apply_roots(&mut command, vec![root]).unwrap_err();
        assert!(message.starts_with("isolated-runtime-unsupported:"));
        assert_eq!(command.get_args().count(), 0);
    }
    #[test]
    fn dangling_entries_do_not_reject_valid_skills() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("skills");
        let good = root.join("good");
        fs::create_dir_all(&good).unwrap();
        fs::write(good.join("SKILL.md"), "fixture").unwrap();
        symlink(root.join("missing"), root.join("dangling")).unwrap();
        let mut command = Command::new("codex");
        apply_roots(&mut command, vec![root]).unwrap();
        let args = command
            .get_args()
            .map(|arg| arg.to_str().unwrap())
            .collect::<Vec<_>>();
        assert!(
            args.iter()
                .any(|arg| arg.contains(good.join("SKILL.md").to_str().unwrap()))
        );
    }

    #[test]
    fn unreadable_directories_still_fail_closed() {
        use std::os::unix::fs::PermissionsExt;
        if unsafe { libc::geteuid() } == 0 {
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("skills");
        let blocked = root.join("blocked");
        fs::create_dir_all(&blocked).unwrap();
        fs::set_permissions(&blocked, fs::Permissions::from_mode(0o000)).unwrap();
        let result = apply_roots(&mut Command::new("codex"), vec![root]);
        fs::set_permissions(&blocked, fs::Permissions::from_mode(0o700)).unwrap();
        assert!(
            result
                .unwrap_err()
                .starts_with("isolated-runtime-unsupported:")
        );
    }

    #[test]
    fn project_discovery_stops_at_the_nearest_git_marker() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("project");
        let cwd = project.join("child");
        fs::create_dir_all(&cwd).unwrap();
        fs::write(project.join(".git"), "gitdir: fixture").unwrap();
        assert_eq!(
            super::roots(&cwd, None),
            vec![
                cwd.join(".agents/skills"),
                cwd.join(".codex/skills"),
                project.join(".agents/skills"),
                project.join(".codex/skills"),
            ]
        );
    }
}
