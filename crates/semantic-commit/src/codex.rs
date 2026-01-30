use std::path::PathBuf;

fn infer_codex_home_from_exe() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let parent = exe.parent()?;
    if parent.file_name()?.to_str()? != "commands" {
        return None;
    }
    parent.parent().map(|p| p.to_path_buf())
}

pub fn codex_home() -> Option<PathBuf> {
    if let Some(value) = std::env::var_os("CODEX_HOME") {
        if !value.is_empty() {
            return Some(PathBuf::from(value));
        }
    }
    infer_codex_home_from_exe()
}

pub fn commands_dir() -> Option<PathBuf> {
    if let Some(value) = std::env::var_os("CODEX_COMMANDS_PATH") {
        if !value.is_empty() {
            return Some(PathBuf::from(value));
        }
    }
    codex_home().map(|home| home.join("commands"))
}

pub fn resolve_command(name: &str) -> Option<PathBuf> {
    let commands_dir = commands_dir()?;
    let candidate = commands_dir.join(name);
    is_executable(&candidate).then_some(candidate)
}

#[cfg(unix)]
fn is_executable(path: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    meta.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable(path: &std::path::Path) -> bool {
    std::fs::metadata(path).is_ok()
}
