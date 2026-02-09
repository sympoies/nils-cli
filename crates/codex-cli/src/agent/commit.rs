use anyhow::Result;
use std::collections::BTreeSet;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::prompts;

use super::exec;

pub struct CommitOptions {
    pub push: bool,
    pub auto_stage: bool,
    pub extra: Vec<String>,
}

pub fn run(options: &CommitOptions) -> Result<i32> {
    if !command_exists("git") {
        eprintln!("codex-commit-with-scope: missing binary: git");
        return Ok(1);
    }

    let git_root = match git_root() {
        Some(value) => value,
        None => {
            eprintln!("codex-commit-with-scope: not a git repository");
            return Ok(1);
        }
    };

    if options.auto_stage {
        let status = Command::new("git")
            .arg("-C")
            .arg(&git_root)
            .arg("add")
            .arg("-A")
            .status()?;
        if !status.success() {
            return Ok(1);
        }
    } else {
        let staged = staged_files(&git_root);
        if staged.trim().is_empty() {
            eprintln!("codex-commit-with-scope: no staged changes (stage files then retry)");
            return Ok(1);
        }
    }

    let extra_prompt = options.extra.join(" ");

    if !command_exists("semantic-commit") {
        return run_fallback(&git_root, options.push, &extra_prompt);
    }

    {
        let stderr = io::stderr();
        let mut stderr = stderr.lock();
        if !exec::require_allow_dangerous(Some("codex-commit-with-scope"), &mut stderr) {
            return Ok(1);
        }
    }

    let mode = if options.auto_stage {
        "autostage"
    } else {
        "staged"
    };
    let mut prompt = match semantic_commit_prompt(mode) {
        Some(value) => value,
        None => return Ok(1),
    };

    if options.push {
        prompt.push_str(
            "\n\nFurthermore, please push the committed changes to the remote repository.",
        );
    }

    if !extra_prompt.trim().is_empty() {
        prompt.push_str("\n\nAdditional instructions from user:\n");
        prompt.push_str(extra_prompt.trim());
    }

    let stderr = io::stderr();
    let mut stderr = stderr.lock();
    Ok(exec::exec_dangerous(
        &prompt,
        "codex-commit-with-scope",
        &mut stderr,
    ))
}

fn run_fallback(git_root: &Path, push_flag: bool, extra_prompt: &str) -> Result<i32> {
    let staged = staged_files(git_root);
    if staged.trim().is_empty() {
        eprintln!("codex-commit-with-scope: no staged changes (stage files then retry)");
        return Ok(1);
    }

    eprintln!("codex-commit-with-scope: semantic-commit not found on PATH (fallback mode)");
    if !extra_prompt.trim().is_empty() {
        eprintln!("codex-commit-with-scope: note: extra prompt is ignored in fallback mode");
    }

    if command_exists("git-scope") {
        let _ = Command::new("git-scope")
            .current_dir(git_root)
            .arg("staged")
            .status();
    } else {
        println!("Staged files:");
        print!("{staged}");
    }

    let suggested_scope = suggested_scope_from_staged(&staged);

    let mut commit_type = read_prompt("Type [chore]: ")?;
    commit_type = commit_type.to_ascii_lowercase();
    commit_type.retain(|ch| !ch.is_whitespace());
    if commit_type.is_empty() {
        commit_type = "chore".to_string();
    }

    let scope_prompt = if suggested_scope.is_empty() {
        "Scope (optional): ".to_string()
    } else {
        format!("Scope (optional) [{suggested_scope}]: ")
    };
    let mut scope = read_prompt(&scope_prompt)?;
    scope.retain(|ch| !ch.is_whitespace());
    if scope.is_empty() {
        scope = suggested_scope;
    }

    let subject = loop {
        let raw = read_prompt("Subject: ")?;
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            break trimmed.to_string();
        }
    };

    let header = if scope.is_empty() {
        format!("{commit_type}: {subject}")
    } else {
        format!("{commit_type}({scope}): {subject}")
    };

    println!();
    println!("Commit message:");
    println!("  {header}");

    let confirm = read_prompt("Proceed? [y/N] ")?;
    if !matches!(confirm.trim().chars().next(), Some('y' | 'Y')) {
        eprintln!("Aborted.");
        return Ok(1);
    }

    let status = Command::new("git")
        .arg("-C")
        .arg(git_root)
        .arg("commit")
        .arg("-m")
        .arg(&header)
        .status()?;
    if !status.success() {
        return Ok(1);
    }

    if push_flag {
        let status = Command::new("git")
            .arg("-C")
            .arg(git_root)
            .arg("push")
            .status()?;
        if !status.success() {
            return Ok(1);
        }
    }

    if command_exists("git-scope") {
        let _ = Command::new("git-scope")
            .current_dir(git_root)
            .arg("commit")
            .arg("HEAD")
            .status();
    } else {
        let _ = Command::new("git")
            .arg("-C")
            .arg(git_root)
            .arg("show")
            .arg("-1")
            .arg("--name-status")
            .arg("--oneline")
            .status();
    }

    Ok(0)
}

fn suggested_scope_from_staged(staged: &str) -> String {
    let mut top: BTreeSet<String> = BTreeSet::new();
    for line in staged.lines() {
        let file = line.trim();
        if file.is_empty() {
            continue;
        }
        if let Some((first, _rest)) = file.split_once('/') {
            top.insert(first.to_string());
        } else {
            top.insert(String::new());
        }
    }

    if top.len() == 1 {
        return top.iter().next().cloned().unwrap_or_default();
    }

    if top.len() == 2 && top.contains("") {
        for part in top {
            if !part.is_empty() {
                return part;
            }
        }
    }

    String::new()
}

fn read_prompt(prompt: &str) -> Result<String> {
    print!("{prompt}");
    let _ = io::stdout().flush();

    let mut line = String::new();
    let bytes = io::stdin().read_line(&mut line)?;
    if bytes == 0 {
        return Ok(String::new());
    }
    Ok(line.trim_end_matches(&['\r', '\n'][..]).to_string())
}

fn staged_files(git_root: &Path) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(git_root)
        .arg("-c")
        .arg("core.quotepath=false")
        .arg("diff")
        .arg("--cached")
        .arg("--name-only")
        .arg("--diff-filter=ACMRTUXBD")
        .output();

    match output {
        Ok(out) => String::from_utf8_lossy(&out.stdout).to_string(),
        Err(_) => String::new(),
    }
}

fn git_root() -> Option<PathBuf> {
    let output = Command::new("git")
        .arg("rev-parse")
        .arg("--show-toplevel")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if path.is_empty() {
        return None;
    }
    Some(PathBuf::from(path))
}

fn semantic_commit_prompt(mode: &str) -> Option<String> {
    let template_name = match mode {
        "staged" => "semantic-commit-staged",
        "autostage" => "semantic-commit-autostage",
        other => {
            eprintln!("_codex_tools_semantic_commit_prompt: invalid mode: {other}");
            return None;
        }
    };

    let prompts_dir = match prompts::resolve_prompts_dir() {
        Some(value) => value,
        None => {
            eprintln!(
                "_codex_tools_semantic_commit_prompt: prompts dir not found (expected: $ZDOTDIR/prompts)"
            );
            return None;
        }
    };

    let prompt_file = prompts_dir.join(format!("{template_name}.md"));
    if !prompt_file.is_file() {
        eprintln!(
            "_codex_tools_semantic_commit_prompt: prompt template not found: {}",
            prompt_file.to_string_lossy()
        );
        return None;
    }

    match std::fs::read_to_string(&prompt_file) {
        Ok(content) => Some(content),
        Err(_) => {
            eprintln!(
                "_codex_tools_semantic_commit_prompt: failed to read prompt template: {}",
                prompt_file.to_string_lossy()
            );
            None
        }
    }
}

fn command_exists(name: &str) -> bool {
    let Ok(path) = std::env::var("PATH") else {
        return false;
    };

    for dir in std::env::split_paths(&path) {
        let full = dir.join(name);
        if !full.is_file() {
            continue;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Ok(meta) = std::fs::metadata(&full)
                && meta.permissions().mode() & 0o111 != 0
            {
                return true;
            }
        }
        #[cfg(not(unix))]
        {
            return true;
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::{command_exists, semantic_commit_prompt, suggested_scope_from_staged};
    use pretty_assertions::assert_eq;

    struct EnvGuard {
        key: &'static str,
        old: Option<std::ffi::OsString>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: impl AsRef<std::ffi::OsStr>) -> Self {
            let old = std::env::var_os(key);
            // SAFETY: tests mutate process env only in scoped guard usage.
            unsafe { std::env::set_var(key, value) };
            Self { key, old }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            if let Some(value) = self.old.take() {
                // SAFETY: tests restore process env only in scoped guard usage.
                unsafe { std::env::set_var(self.key, value) };
            } else {
                // SAFETY: tests restore process env only in scoped guard usage.
                unsafe { std::env::remove_var(self.key) };
            }
        }
    }

    #[test]
    fn suggested_scope_prefers_single_top_level_directory() {
        let staged = "src/main.rs\nsrc/lib.rs\n";
        assert_eq!(suggested_scope_from_staged(staged), "src");
    }

    #[test]
    fn suggested_scope_ignores_root_file_when_single_directory_exists() {
        let staged = "README.md\nsrc/main.rs\n";
        assert_eq!(suggested_scope_from_staged(staged), "src");
    }

    #[test]
    fn suggested_scope_returns_empty_for_multiple_directories() {
        let staged = "src/main.rs\ncrates/a.rs\n";
        assert_eq!(suggested_scope_from_staged(staged), "");
    }

    #[test]
    fn semantic_commit_prompt_rejects_invalid_mode() {
        assert!(semantic_commit_prompt("unknown").is_none());
    }

    #[cfg(unix)]
    #[test]
    fn command_exists_checks_executable_bit() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::TempDir::new().expect("tempdir");
        let executable = dir.path().join("tool-ok");
        let non_executable = dir.path().join("tool-no");
        std::fs::write(&executable, "#!/bin/sh\necho ok\n").expect("write executable");
        std::fs::write(&non_executable, "plain text").expect("write non executable");

        let mut perms = std::fs::metadata(&executable)
            .expect("metadata")
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&executable, perms).expect("chmod executable");

        let mut perms = std::fs::metadata(&non_executable)
            .expect("metadata")
            .permissions();
        perms.set_mode(0o644);
        std::fs::set_permissions(&non_executable, perms).expect("chmod non executable");

        let _path_guard = EnvGuard::set("PATH", dir.path().as_os_str());
        assert!(command_exists("tool-ok"));
        assert!(!command_exists("tool-no"));
        assert!(!command_exists("tool-missing"));
    }
}
