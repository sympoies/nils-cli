use std::path::{Path, PathBuf};

use crate::error::HookError;
use crate::model::Product;

pub(super) fn provider_path(product: Product) -> Result<PathBuf, HookError> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .ok_or_else(|| HookError::runtime("home-unavailable", "HOME is required for setup"))?;
    Ok(match product {
        Product::Codex => config_dir("CODEX_HOME", home.join(".codex")).join("config.toml"),
        Product::Claude => claude_config_path(
            &home,
            std::env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from),
        ),
        Product::Dsh => home.join(".dsh/cordis.patch.yml"),
        Product::Hermes => home.join(".hermes/config.yaml"),
    })
}

fn config_dir(variable: &str, fallback: PathBuf) -> PathBuf {
    std::env::var_os(variable)
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or(fallback)
}

fn claude_config_path(home: &Path, config_dir: Option<PathBuf>) -> PathBuf {
    config_dir
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| home.join(".claude"))
        .join("settings.json")
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use pretty_assertions::assert_eq;

    use super::claude_config_path;

    #[test]
    fn claude_config_path_uses_an_absolute_override() {
        let home = Path::new("/home/example");
        let override_dir = Path::new("/custom/claude").to_path_buf();

        assert_eq!(
            claude_config_path(home, Some(override_dir)),
            Path::new("/custom/claude/settings.json")
        );
    }

    #[test]
    fn claude_config_path_ignores_a_relative_override() {
        let home = Path::new("/home/example");
        let relative_dir = Path::new("claude-config").to_path_buf();

        assert_eq!(
            claude_config_path(home, Some(relative_dir)),
            Path::new("/home/example/.claude/settings.json")
        );
    }
}
