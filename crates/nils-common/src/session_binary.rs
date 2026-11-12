//! The session binary name shared by every call site.
//!
//! A distribution can install the session binary under another name by building
//! with `NILS_SESSION_BIN_NAME` set. Every lookup and every rendered provider
//! command takes the name from [`NAME`], so the default build keeps
//! `agent-session` and a renamed build never mentions it.

use std::ffi::OsStr;
use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

/// Executable name of the session binary, fixed when the workspace is built.
pub const NAME: &str = match option_env!("NILS_SESSION_BIN_NAME") {
    Some(name) => name,
    None => "agent-session",
};

const _: () = assert!(
    valid_name(NAME),
    "NILS_SESSION_BIN_NAME must start with a letter or digit and use only letters, digits, dot, underscore and hyphen"
);

/// Letters, digits, `.`, `_` and `-`, starting alphanumeric: one literal word in
/// every shell command string and hook-command matcher that renders the name.
const fn valid_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    if bytes.is_empty() || !bytes[0].is_ascii_alphanumeric() {
        return false;
    }
    let mut index = 0;
    while index < bytes.len() {
        if !(bytes[index].is_ascii_alphanumeric() || matches!(bytes[index], b'.' | b'_' | b'-')) {
            return false;
        }
        index += 1;
    }
    true
}

/// Resolve an owner-controlled session executable from an approved absolute root.
pub fn trusted_root(root: &Path) -> Option<PathBuf> {
    if !root.is_absolute() || !root.is_dir() {
        return None;
    }
    let candidate = fs::canonicalize(root.join(NAME)).ok()?;
    is_owner_controlled_executable(&candidate).then_some(candidate)
}

/// A regular executable file owned by the caller that no other user can write.
pub fn is_owner_controlled_executable(path: &Path) -> bool {
    fs::metadata(path).is_ok_and(|metadata| {
        metadata.is_file()
            && metadata.uid() == unsafe { libc::geteuid() }
            && metadata.permissions().mode() & 0o022 == 0
            && metadata.permissions().mode() & 0o111 != 0
    })
}

/// First regular executable file named [`NAME`] in a `PATH`-style list.
pub fn locate_on_path(path: Option<&OsStr>) -> Option<PathBuf> {
    std::env::split_paths(path?)
        .map(|directory| directory.join(NAME))
        .find(|candidate| {
            fs::metadata(candidate).is_ok_and(|metadata| {
                metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
            })
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn name_validation_rejects_empty_and_path_like_names() {
        assert!(valid_name("agent-session"));
        assert!(!valid_name(""));
        assert!(!valid_name("bin/agent-session"));
    }

    #[test]
    fn name_validation_keeps_every_command_renderer_single_word() {
        for safe in ["agent-session", "renamed_session.v2", "s1"] {
            assert!(valid_name(safe), "{safe}");
        }
        for unsafe_name in [
            "two words",
            "quote'name",
            "semi;colon",
            "dollar$name",
            "back`tick",
            "new\nline",
            "-leading-dash",
            "renamed' ; printf injected #",
        ] {
            assert!(!valid_name(unsafe_name), "{unsafe_name:?}");
        }
    }

    #[test]
    fn trusted_roots_skip_writable_helper() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        for (root, mode) in [(first.path(), 0o777), (second.path(), 0o700)] {
            let path = root.join(NAME);
            fs::write(&path, "fixture").unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
        }
        let selected = [first.path(), second.path()]
            .into_iter()
            .find_map(trusted_root);
        assert_eq!(
            selected,
            Some(fs::canonicalize(second.path().join(NAME)).unwrap())
        );
    }

    #[test]
    fn locate_on_path_finds_the_named_executable_only() {
        let directory = tempfile::tempdir().expect("tempdir");
        let skipped = directory.path().join("skipped");
        let found = directory.path().join("found");
        fs::create_dir_all(&skipped).expect("skipped dir");
        fs::create_dir_all(&found).expect("found dir");
        fs::write(skipped.join(NAME), "#!/bin/sh\n").expect("non-executable");
        fs::write(found.join(NAME), "#!/bin/sh\n").expect("executable");
        fs::set_permissions(found.join(NAME), fs::Permissions::from_mode(0o755)).expect("mode");
        fs::write(found.join("other"), "#!/bin/sh\n").expect("other");

        let path = std::env::join_paths([&skipped, &found]).expect("join");
        assert_eq!(locate_on_path(Some(&path)), Some(found.join(NAME)));
        assert_eq!(locate_on_path(None), None);
    }
}
