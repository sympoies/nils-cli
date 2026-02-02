use std::process::Command;
use std::sync::OnceLock;

pub const TREE_MISSING_WARNING: &str =
    "⚠️  tree is not installed. Install it to see the directory tree.";
pub const TREE_UNSUPPORTED_WARNING: &str =
    "⚠️  tree does not support --fromfile. Please upgrade tree to enable directory tree output.";

#[derive(Debug, Clone, Copy)]
pub struct TreeSupport {
    pub is_installed: bool,
    pub supports_fromfile: bool,
    pub warning: Option<&'static str>,
}

pub fn tree_support() -> &'static TreeSupport {
    static SUPPORT: OnceLock<TreeSupport> = OnceLock::new();
    SUPPORT.get_or_init(detect_tree_support)
}

fn detect_tree_support() -> TreeSupport {
    if Command::new("tree").arg("--version").output().is_err() {
        return TreeSupport {
            is_installed: false,
            supports_fromfile: false,
            warning: Some(TREE_MISSING_WARNING),
        };
    }

    let support = Command::new("tree")
        .arg("--fromfile")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();

    if support.map(|s| !s.success()).unwrap_or(true) {
        return TreeSupport {
            is_installed: true,
            supports_fromfile: false,
            warning: Some(TREE_UNSUPPORTED_WARNING),
        };
    }

    TreeSupport {
        is_installed: true,
        supports_fromfile: true,
        warning: None,
    }
}
