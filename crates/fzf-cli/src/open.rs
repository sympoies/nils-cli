use crate::util;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenWith {
    Vi,
    Vscode,
}

pub fn parse_open_with_flags(args: &[String]) -> Result<(OpenWith, Vec<String>), i32> {
    let env_default = util::env_or_default("FZF_FILE_OPEN_WITH", "vi");
    let mut open_with = if env_default.trim() == "vscode" {
        OpenWith::Vscode
    } else {
        OpenWith::Vi
    };

    let mut seen_vi = false;
    let mut seen_vscode = false;

    let mut rest: Vec<String> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--vi" => {
                seen_vi = true;
                open_with = OpenWith::Vi;
                i += 1;
            }
            "--vscode" => {
                seen_vscode = true;
                open_with = OpenWith::Vscode;
                i += 1;
            }
            "--" => {
                i += 1;
                rest.extend_from_slice(&args[i..]);
                break;
            }
            flag if flag.starts_with("--") => {
                eprintln!("❌ Unknown flag: {flag}");
                return Err(2);
            }
            _ => {
                rest.extend_from_slice(&args[i..]);
                break;
            }
        }
    }

    if seen_vi && seen_vscode {
        eprintln!("❌ Flags are mutually exclusive: --vi and --vscode");
        return Err(2);
    }

    Ok((open_with, rest))
}

pub fn find_git_root_upwards(start_dir: &Path, max_depth: usize) -> Option<PathBuf> {
    let mut dir = start_dir.to_path_buf();
    for _ in 0..=max_depth {
        if dir.join(".git").exists() {
            return Some(dir);
        }
        if !dir.pop() {
            break;
        }
    }
    None
}

fn open_in_vscode_workspace(workspace_root: &Path, file: &Path, wait: bool) -> anyhow::Result<()> {
    ensure_code_available()?;
    run_code(&build_code_args(file, Some(workspace_root), wait))
}

pub fn open_in_vscode(file: &Path, wait: bool) -> anyhow::Result<()> {
    if let Some(parent) = file.parent()
        && let Some(git_root) = find_git_root_upwards(parent, 5)
    {
        return open_in_vscode_workspace(&git_root, file, wait);
    }

    ensure_code_available()?;
    run_code(&build_code_args(file, None, wait))
}

pub fn open_file(open_with: OpenWith, file: &Path, wait: bool) -> i32 {
    match open_with {
        OpenWith::Vscode => {
            if open_in_vscode(file, wait).is_err() {
                eprintln!("❌ Failed to open in VSCode; falling back to vi");
                return open_vi(file);
            }
            0
        }
        OpenWith::Vi => open_vi(file),
    }
}

pub fn open_file_in_workspace(
    open_with: OpenWith,
    workspace_root: &Path,
    file: &Path,
    wait: bool,
) -> i32 {
    match open_with {
        OpenWith::Vscode => {
            if open_in_vscode_workspace(workspace_root, file, wait).is_err() {
                eprintln!("❌ Failed to open in VSCode; falling back to vi");
                return open_vi(file);
            }
            0
        }
        OpenWith::Vi => open_vi(file),
    }
}

fn open_vi(file: &Path) -> i32 {
    let status = Command::new("vi")
        .arg("--")
        .arg(file)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status();

    match status {
        Ok(s) => s.code().unwrap_or(1),
        Err(_) => 127,
    }
}

fn ensure_code_available() -> anyhow::Result<()> {
    if !util::cmd_exists("code") {
        eprintln!("❌ 'code' not found");
        anyhow::bail!("code not found");
    }
    Ok(())
}

fn build_code_args(file: &Path, workspace_root: Option<&Path>, wait: bool) -> Vec<String> {
    let mut args: Vec<String> = Vec::new();
    if wait {
        args.push("--wait".to_string());
    }
    args.push("--goto".to_string());
    args.push(file.to_string_lossy().to_string());
    if let Some(root) = workspace_root {
        args.push("--".to_string());
        args.push(root.to_string_lossy().to_string());
    }
    args
}

fn run_code(args: &[String]) -> anyhow::Result<()> {
    let status = Command::new("code")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    if !status.success() {
        anyhow::bail!("code failed");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use std::sync::Mutex;
    use tempfile::TempDir;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    struct EnvGuard {
        key: &'static str,
        original: Option<String>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let original = std::env::var(key).ok();
            // SAFETY: tests mutate process env only in scoped guard usage.
            unsafe { std::env::set_var(key, value) };
            Self { key, original }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match &self.original {
                Some(value) => {
                    // SAFETY: tests restore process env only in scoped guard usage.
                    unsafe { std::env::set_var(self.key, value) };
                }
                None => {
                    // SAFETY: tests restore process env only in scoped guard usage.
                    unsafe { std::env::remove_var(self.key) };
                }
            }
        }
    }

    #[test]
    fn parse_open_with_flags_respects_env_and_explicit_flags() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _guard = EnvGuard::set("FZF_FILE_OPEN_WITH", "vscode");
        let (open_with, rest) = parse_open_with_flags(&[String::from("file.txt")]).expect("parse");
        assert_eq!(open_with, OpenWith::Vscode);
        assert_eq!(rest, vec!["file.txt".to_string()]);

        let (open_with, rest) = parse_open_with_flags(&[
            String::from("--vi"),
            String::from("--"),
            String::from("a.txt"),
        ])
        .expect("parse");
        assert_eq!(open_with, OpenWith::Vi);
        assert_eq!(rest, vec!["a.txt".to_string()]);
    }

    #[test]
    fn parse_open_with_flags_rejects_conflicts_and_unknowns() {
        let _lock = ENV_LOCK.lock().unwrap();
        let err =
            parse_open_with_flags(&[String::from("--vi"), String::from("--vscode")]).unwrap_err();
        assert_eq!(err, 2);

        let err = parse_open_with_flags(&[String::from("--nope")]).unwrap_err();
        assert_eq!(err, 2);
    }

    #[test]
    fn find_git_root_upwards_discovers_repo() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        std::fs::create_dir_all(root.join(".git")).unwrap();
        let nested = root.join("a/b/c");
        std::fs::create_dir_all(&nested).unwrap();

        let found = find_git_root_upwards(&nested, 5).expect("found");
        assert_eq!(found, root);
        assert_eq!(find_git_root_upwards(&nested, 1), None);
    }
}
