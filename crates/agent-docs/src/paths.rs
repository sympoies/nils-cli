use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};

pub fn normalize_root_path(candidate: &Path, cwd: &Path) -> PathBuf {
    if candidate.is_absolute() {
        normalize_path(candidate)
    } else {
        normalize_path(&cwd.join(candidate))
    }
}

pub fn normalize_path(path: &Path) -> PathBuf {
    let mut prefix: Option<OsString> = None;
    let mut has_root = false;
    let mut segments: Vec<OsString> = Vec::new();

    for component in path.components() {
        match component {
            Component::Prefix(value) => {
                prefix = Some(value.as_os_str().to_os_string());
            }
            Component::RootDir => {
                has_root = true;
            }
            Component::CurDir => {}
            Component::ParentDir => {
                if segments.pop().is_none() && !has_root {
                    segments.push(OsString::from(".."));
                }
            }
            Component::Normal(value) => {
                segments.push(value.to_os_string());
            }
        }
    }

    let mut normalized = PathBuf::new();
    if let Some(prefix) = prefix {
        normalized.push(prefix);
    }
    if has_root {
        normalized.push(Path::new(std::path::MAIN_SEPARATOR_STR));
    }
    for segment in segments {
        normalized.push(segment);
    }

    if normalized.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        normalized
    }
}

#[cfg(test)]
mod tests {
    use super::{normalize_path, normalize_root_path};
    use std::path::Path;

    #[test]
    fn normalize_root_path_joins_relative_to_cwd() {
        let normalized = normalize_root_path(Path::new("./docs/../policy"), Path::new("/tmp/repo"));
        assert_eq!(normalized, Path::new("/tmp/repo/policy"));
    }

    #[test]
    fn normalize_path_collapses_parent_segments_for_relative_paths() {
        let normalized = normalize_path(Path::new("alpha/beta/../gamma/./delta"));
        assert_eq!(normalized, Path::new("alpha/gamma/delta"));
    }

    #[test]
    fn normalize_path_keeps_leading_parent_when_no_root() {
        let normalized = normalize_path(Path::new("../alpha/../beta"));
        assert_eq!(normalized, Path::new("../beta"));
    }

    #[test]
    fn normalize_path_returns_dot_for_empty_input() {
        let normalized = normalize_path(Path::new(""));
        assert_eq!(normalized, Path::new("."));
    }

    #[cfg(windows)]
    #[test]
    fn normalize_path_collapses_segments_for_drive_absolute_paths() {
        let normalized = normalize_path(Path::new(r"C:\repo\docs\..\policy\.\notes"));
        assert_eq!(normalized, Path::new(r"C:\repo\policy\notes"));
    }

    #[cfg(windows)]
    #[test]
    fn normalize_path_keeps_drive_relative_prefix_without_root() {
        let normalized = normalize_path(Path::new(r"C:repo\docs\..\policy"));
        assert_eq!(normalized, Path::new(r"C:repo\policy"));
    }

    #[cfg(windows)]
    #[test]
    fn normalize_path_collapses_segments_for_unc_paths() {
        let normalized = normalize_path(Path::new(r"\\server\share\alpha\..\beta\.\gamma"));
        assert_eq!(normalized, Path::new(r"\\server\share\beta\gamma"));
    }
}
