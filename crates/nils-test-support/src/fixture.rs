//! Private filesystem and child-process environment for a test fixture.

use std::ffi::OsStr;
use std::fs;
use std::path::Path;
use std::process::Command;

use crate::tempdir::ScopedTempDir;
use crate::{git, hermetic};

/// Owns cleanup-checked temporary and runtime directories for one fixture.
/// Keep this value alive until all fixture children have exited.
pub struct Fixture {
    temp: ScopedTempDir,
    runtime: ScopedTempDir,
}

impl Fixture {
    pub fn new() -> Self {
        let temp = ScopedTempDir::outside_git_ancestry("nils-fixture-");
        #[cfg(unix)]
        let runtime = crate::tempdir::short_runtime_dir();
        #[cfg(not(unix))]
        let runtime = ScopedTempDir::new();

        fs::create_dir(temp.path().join("home")).expect("create fixture home");
        fs::write(
            temp.path().join("gitconfig"),
            "[commit]\n\tgpgSign = false\n[tag]\n\tgpgSign = false\n",
        )
        .expect("write fixture Git config");
        Self { temp, runtime }
    }

    /// Private temporary directory outside Git ancestry.
    pub fn temp_dir(&self) -> &Path {
        self.temp.path()
    }

    /// Private runtime directory with a short path on Unix.
    pub fn runtime_dir(&self) -> &Path {
        self.runtime.path()
    }

    /// Build a child command with private cwd, home, temp, runtime, and Git settings.
    /// Apply fixture-specific arguments and environment overrides to the result.
    pub fn command(&self, program: impl AsRef<OsStr>) -> Command {
        let mut command = Command::new(program);
        command
            .env_clear()
            .envs(std::env::vars_os().filter(|(name, _)| {
                name.to_str()
                    .is_some_and(|name| hermetic::allowed(name, &[]))
            }));
        command
            .current_dir(self.temp_dir())
            .env("HOME", self.temp_dir().join("home"))
            .env("USERPROFILE", self.temp_dir().join("home"))
            .env("TMPDIR", self.temp_dir())
            .env("TMP", self.temp_dir())
            .env("TEMP", self.temp_dir())
            .env("XDG_RUNTIME_DIR", self.runtime_dir())
            .env("GIT_CONFIG_GLOBAL", self.temp_dir().join("gitconfig"))
            .env("GIT_CONFIG_NOSYSTEM", "1");
        command
    }

    /// Build a command using the shared test Git executable resolver.
    pub fn git_command(&self, dir: &Path) -> Command {
        let mut command = self.command(git::git_command());
        command.current_dir(dir);
        command
    }
}

impl Default for Fixture {
    fn default() -> Self {
        Self::new()
    }
}
