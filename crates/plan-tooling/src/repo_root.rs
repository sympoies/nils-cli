use std::path::PathBuf;
use std::process::Command;

pub fn detect() -> PathBuf {
    if let Ok(out) = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()
    {
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !s.is_empty() {
                return PathBuf::from(s);
            }
        }
    }

    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}
