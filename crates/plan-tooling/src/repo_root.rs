use std::path::PathBuf;

pub fn detect() -> PathBuf {
    if let Ok(root) = std::env::var("CODEX_HOME") {
        let p = PathBuf::from(root);
        if p.is_dir() {
            return p;
        }
    }

    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}
