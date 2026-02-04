use crate::{fzf, open, util};
use nils_term::progress::{Progress, ProgressFinish, ProgressOptions};
use std::path::Path;
use walkdir::WalkDir;

pub fn run(args: &[String]) -> i32 {
    let (open_with, query_parts) = match open::parse_open_with_flags(args) {
        Ok(v) => v,
        Err(code) => return code,
    };

    let max_depth = util::env_or_default("FZF_FILE_MAX_DEPTH", "10")
        .parse::<usize>()
        .unwrap_or(10);

    let files = list_files(max_depth);
    let input = if files.is_empty() {
        String::new()
    } else {
        format!("{}\n", files.join("\n"))
    };

    let query = util::join_args(&query_parts);
    let fzf_args = [
        "--ansi",
        "--query",
        &query,
        "--preview",
        "bat --color=always --style=numbers --line-range :100 {}",
    ];

    let (code, lines) = match fzf::run_lines(&input, &fzf_args, &[]) {
        Ok(v) => v,
        Err(err) => {
            eprintln!("{err:#}");
            return 1;
        }
    };

    if code != 0 {
        return 0;
    }
    let Some(selected) = lines.first() else {
        return 0;
    };

    open::open_file(open_with, Path::new(selected), false)
}

fn list_files(max_depth: usize) -> Vec<String> {
    let spinner = Progress::spinner(
        ProgressOptions::default()
            .with_prefix("index ")
            .with_finish(ProgressFinish::Clear),
    );
    spinner.set_message("files");
    spinner.tick();

    let walker = WalkDir::new(".")
        .follow_links(true)
        .max_depth(max_depth)
        .into_iter()
        .filter_entry(|e| e.file_name() != ".git");

    let mut out = Vec::new();
    let mut scanned: usize = 0;
    for entry in walker.flatten() {
        scanned = scanned.saturating_add(1);
        if scanned.is_multiple_of(128) {
            spinner.tick();
        }

        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        if path.components().any(|c| c.as_os_str() == ".git") {
            continue;
        }
        let display = path.strip_prefix(".").unwrap_or(path).to_string_lossy();
        let display = display.trim_start_matches('/');
        if display.is_empty() {
            continue;
        }
        out.push(display.to_string());
    }
    out.sort();
    spinner.finish_and_clear();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::Mutex;
    use tempfile::TempDir;

    static CWD_LOCK: Mutex<()> = Mutex::new(());

    struct CwdGuard {
        original: PathBuf,
    }

    impl CwdGuard {
        fn new(original: PathBuf) -> Self {
            Self { original }
        }
    }

    impl Drop for CwdGuard {
        fn drop(&mut self) {
            let _ = std::env::set_current_dir(&self.original);
        }
    }

    #[test]
    fn list_files_skips_git_and_sorts() {
        let _lock = CWD_LOCK.lock().unwrap();
        let dir = TempDir::new().unwrap();

        std::fs::create_dir_all(dir.path().join(".git/objects")).unwrap();
        std::fs::write(dir.path().join(".git/ignored.txt"), "x").unwrap();
        std::fs::create_dir_all(dir.path().join("nested")).unwrap();
        std::fs::write(dir.path().join("a.txt"), "x").unwrap();
        std::fs::write(dir.path().join("b.txt"), "x").unwrap();
        std::fs::write(dir.path().join("nested/c.txt"), "x").unwrap();

        let original = std::env::current_dir().unwrap();
        let _guard = CwdGuard::new(original);
        std::env::set_current_dir(dir.path()).unwrap();

        let files = list_files(5);
        assert_eq!(files, vec!["a.txt", "b.txt", "nested/c.txt"]);
    }

    #[test]
    fn list_files_respects_max_depth() {
        let _lock = CWD_LOCK.lock().unwrap();
        let dir = TempDir::new().unwrap();

        std::fs::write(dir.path().join("root.txt"), "x").unwrap();
        std::fs::create_dir_all(dir.path().join("nested")).unwrap();
        std::fs::write(dir.path().join("nested/deeper.txt"), "x").unwrap();

        let original = std::env::current_dir().unwrap();
        let _guard = CwdGuard::new(original);
        std::env::set_current_dir(dir.path()).unwrap();

        let files = list_files(1);
        assert_eq!(files, vec!["root.txt"]);
    }
}
