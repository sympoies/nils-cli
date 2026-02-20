use std::collections::BTreeSet;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::prompts;

use super::exec;

#[derive(Clone, Debug, Default)]
pub struct CommitOptions {
    pub push: bool,
    pub auto_stage: bool,
    pub extra: Vec<String>,
}

pub fn run(options: &CommitOptions) -> i32 {
    if !command_exists("git") {
        eprintln!("gemini-commit-with-scope: missing binary: git");
        return 1;
    }

    let git_root = match git_root() {
        Some(value) => value,
        None => {
            eprintln!("gemini-commit-with-scope: not a git repository");
            return 1;
        }
    };

    if options.auto_stage {
        let status = Command::new("git")
            .arg("-C")
            .arg(&git_root)
            .arg("add")
            .arg("-A")
            .status();
        if !status.map(|value| value.success()).unwrap_or(false) {
            return 1;
        }
    } else {
        let staged = staged_files(&git_root);
        if staged.trim().is_empty() {
            eprintln!("gemini-commit-with-scope: no staged changes (stage files then retry)");
            return 1;
        }
    }

    let extra_prompt = options.extra.join(" ");

    if !command_exists("semantic-commit") {
        return run_fallback(&git_root, options.push, &extra_prompt);
    }

    {
        let stderr = io::stderr();
        let mut stderr = stderr.lock();
        if !exec::require_allow_dangerous(Some("gemini-commit-with-scope"), &mut stderr) {
            return 1;
        }
    }

    let mode = if options.auto_stage {
        "autostage"
    } else {
        "staged"
    };
    let mut prompt = match semantic_commit_prompt(mode) {
        Some(value) => value,
        None => return 1,
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
    exec::exec_dangerous(&prompt, "gemini-commit-with-scope", &mut stderr)
}

fn run_fallback(git_root: &Path, push_flag: bool, extra_prompt: &str) -> i32 {
    let staged = staged_files(git_root);
    if staged.trim().is_empty() {
        eprintln!("gemini-commit-with-scope: no staged changes (stage files then retry)");
        return 1;
    }

    eprintln!("gemini-commit-with-scope: semantic-commit not found on PATH (fallback mode)");
    if !extra_prompt.trim().is_empty() {
        eprintln!("gemini-commit-with-scope: note: extra prompt is ignored in fallback mode");
    }

    println!("Staged files:");
    print!("{staged}");

    let suggested_scope = suggested_scope_from_staged(&staged);

    let mut commit_type = match read_prompt("Type [chore]: ") {
        Ok(value) => value,
        Err(_) => return 1,
    };
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
    let mut scope = match read_prompt(&scope_prompt) {
        Ok(value) => value,
        Err(_) => return 1,
    };
    scope.retain(|ch| !ch.is_whitespace());
    if scope.is_empty() {
        scope = suggested_scope;
    }

    let subject = loop {
        let raw = match read_prompt("Subject: ") {
            Ok(value) => value,
            Err(_) => return 1,
        };
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

    let confirm = match read_prompt("Proceed? [y/N] ") {
        Ok(value) => value,
        Err(_) => return 1,
    };
    if !matches!(confirm.trim().chars().next(), Some('y' | 'Y')) {
        eprintln!("Aborted.");
        return 1;
    }

    let status = Command::new("git")
        .arg("-C")
        .arg(git_root)
        .arg("commit")
        .arg("-m")
        .arg(&header)
        .status();
    if !status.map(|value| value.success()).unwrap_or(false) {
        return 1;
    }

    if push_flag {
        let status = Command::new("git")
            .arg("-C")
            .arg(git_root)
            .arg("push")
            .status();
        if !status.map(|value| value.success()).unwrap_or(false) {
            return 1;
        }
    }

    let _ = Command::new("git")
        .arg("-C")
        .arg(git_root)
        .arg("show")
        .arg("-1")
        .arg("--name-status")
        .arg("--oneline")
        .status();

    0
}

fn suggested_scope_from_staged(staged: &str) -> String {
    let mut top: BTreeSet<String> = BTreeSet::new();
    for line in staged.lines() {
        let file = line.trim();
        if file.is_empty() {
            continue;
        }
        if let Some((first, _)) = file.split_once('/') {
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

fn read_prompt(prompt: &str) -> io::Result<String> {
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
            eprintln!("_gemini_tools_semantic_commit_prompt: invalid mode: {other}");
            return None;
        }
    };

    let prompts_dir = match prompts::resolve_prompts_dir() {
        Some(value) => value,
        None => {
            eprintln!(
                "_gemini_tools_semantic_commit_prompt: prompts dir not found (expected: $ZDOTDIR/prompts)"
            );
            return None;
        }
    };

    let prompt_file = prompts_dir.join(format!("{template_name}.md"));
    if !prompt_file.is_file() {
        eprintln!(
            "_gemini_tools_semantic_commit_prompt: prompt template not found: {}",
            prompt_file.to_string_lossy()
        );
        return None;
    }

    match std::fs::read_to_string(&prompt_file) {
        Ok(content) => Some(content),
        Err(_) => {
            eprintln!(
                "_gemini_tools_semantic_commit_prompt: failed to read prompt template: {}",
                prompt_file.to_string_lossy()
            );
            None
        }
    }
}

fn command_exists(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }

    if name.contains('/') {
        return is_executable(Path::new(name));
    }

    let path = std::env::var_os("PATH").unwrap_or_default();
    for part in std::env::split_paths(&path) {
        let candidate = part.join(name);
        if is_executable(&candidate) {
            return true;
        }
    }
    false
}

fn is_executable(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path)
            .map(|meta| meta.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }

    #[cfg(not(unix))]
    {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::{command_exists, suggested_scope_from_staged};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::{Mutex, OnceLock};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .expect("env lock")
    }

    struct EnvGuard {
        key: &'static str,
        old: Option<std::ffi::OsString>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: impl AsRef<std::ffi::OsStr>) -> Self {
            let old = std::env::var_os(key);
            // SAFETY: tests mutate env in guarded scope.
            unsafe { std::env::set_var(key, value) };
            Self { key, old }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            if let Some(value) = self.old.take() {
                // SAFETY: tests restore env in guarded scope.
                unsafe { std::env::set_var(self.key, value) };
            } else {
                // SAFETY: tests restore env in guarded scope.
                unsafe { std::env::remove_var(self.key) };
            }
        }
    }

    fn temp_dir(prefix: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        path.push(format!("{prefix}-{}-{nanos}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("temp dir");
        path
    }

    #[cfg(unix)]
    fn write_executable(path: &Path, content: &str, mode: u32) {
        use std::os::unix::fs::PermissionsExt;
        fs::write(path, content).expect("write");
        let mut perms = fs::metadata(path).expect("metadata").permissions();
        perms.set_mode(mode);
        fs::set_permissions(path, perms).expect("chmod");
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

    #[cfg(unix)]
    #[test]
    fn command_exists_checks_executable_bit() {
        let _lock = env_lock();
        let dir = temp_dir("gemini-commit-command-exists");
        let executable = dir.join("tool-ok");
        let non_executable = dir.join("tool-no");
        write_executable(&executable, "#!/bin/sh\necho ok\n", 0o755);
        write_executable(&non_executable, "plain text", 0o644);

        let _path = EnvGuard::set("PATH", dir.as_os_str());
        assert!(command_exists("tool-ok"));
        assert!(!command_exists("tool-no"));
        assert!(!command_exists("tool-missing"));

        let _ = fs::remove_dir_all(dir);
    }
}
