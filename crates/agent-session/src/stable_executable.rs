//! Version-stable `agent-session` path for launch wrappers.
//!
//! A session wrapper keeps `broker_bin`, the app-server proxy helper and
//! `AGENT_SESSION_BIN` for the whole life of the session, and re-executes them
//! at runtime exit (`broker stop`). A package-manager upgrade deletes the old
//! versioned directory, so a path that was resolved into it
//! (`<prefix>/Cellar/<package>/<version>/bin/agent-session`) fails after the
//! upgrade and never reaches the new release (`sympoies/nils-cli#2672`).
//!
//! The launch path is chosen by a fixed rule and always names the same file as
//! the running binary, so it is never a different release than the launcher:
//!
//! 1. an invocation path outside a versioned keg (`argv[0]` resolved through
//!    `PATH`, or the executable path the OS reports), e.g. a package `bin/` link;
//! 2. `<prefix>/opt/<package>/<path in keg>` for a binary inside
//!    `<prefix>/Cellar/<package>/<version>/`;
//! 3. `<prefix>/bin/agent-session` for the same prefix;
//! 4. otherwise the resolved path, reported as [`LaunchBasis::VersionedKeg`] so
//!    diagnostics show the failure instead of hiding it.
//!
//! Symlinks are never resolved into the versioned directory: the chosen path is
//! the link itself, which the package manager repoints on upgrade.

use std::env;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::PermissionsExt;
use std::path::{Component, Path, PathBuf};

/// Why a launch path was chosen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LaunchBasis {
    /// The invocation path, outside any versioned keg.
    Invocation,
    /// The package manager's `opt/<package>` link.
    PackageOpt,
    /// The package manager's `bin/` link.
    PackageBin,
    /// No stable path exists: the path is inside a versioned keg.
    VersionedKeg,
}

impl LaunchBasis {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Invocation => "invocation",
            Self::PackageOpt => "package-opt",
            Self::PackageBin => "package-bin",
            Self::VersionedKeg => "versioned-keg",
        }
    }
}

#[derive(Debug)]
pub(crate) struct LaunchExecutable {
    pub(crate) path: PathBuf,
    pub(crate) basis: LaunchBasis,
}

/// `<prefix>/Cellar/<package>/<version>/<relative>` split into its parts.
struct VersionedKeg {
    prefix: PathBuf,
    package: OsString,
    relative: PathBuf,
}

fn versioned_keg(path: &Path) -> Option<VersionedKeg> {
    let components: Vec<Component> = path.components().collect();
    let cellar = components
        .iter()
        .position(|component| component.as_os_str() == "Cellar")?;
    // Cellar, package, version and at least one path inside the keg.
    if components.len() < cellar + 4 {
        return None;
    }
    Some(VersionedKeg {
        prefix: components[..cellar].iter().collect(),
        package: components[cellar + 1].as_os_str().to_os_string(),
        relative: components[cellar + 3..].iter().collect(),
    })
}

fn binary_name() -> String {
    format!("agent-session{}", env::consts::EXE_SUFFIX)
}

fn is_executable_file(path: &Path) -> bool {
    fs::metadata(path)
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}

/// The running binary resolved to its real file, or its exact release sibling
/// when the invocation is a facade. A linked sibling is refused.
fn canonical_executable(invocation: &Path) -> io::Result<PathBuf> {
    // Linux reports the resolved `/proc/self/exe`, but macOS reports the
    // invocation path, which is a link for a Homebrew `bin/` install. Resolve
    // the running binary itself so the exact-sibling check below still runs in
    // its real release directory and still refuses a linked sibling.
    let invocation = &fs::canonicalize(invocation)?;
    let binary_name = binary_name();
    let executable = if invocation
        .file_name()
        .is_some_and(|name| name == OsStr::new(&binary_name))
    {
        invocation.to_path_buf()
    } else {
        invocation
            .parent()
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    "current executable has no release directory",
                )
            })?
            .join(binary_name)
    };
    let metadata = fs::symlink_metadata(&executable)?;
    if !metadata.file_type().is_file() || metadata.permissions().mode() & 0o111 == 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "agent-session executable is unavailable",
        ));
    }
    Ok(executable)
}

/// Whether `candidate` names the same file as `canonical` right now.
fn names(candidate: &Path, canonical: &Path) -> bool {
    candidate.is_absolute() && fs::canonicalize(candidate).is_ok_and(|real| real == canonical)
}

/// Choose the launch path for the binary reached through `invocation`, trying
/// each of `hints` (other ways the process was invoked) after it.
pub(crate) fn launch_executable_from(
    invocation: &Path,
    hints: &[PathBuf],
) -> io::Result<LaunchExecutable> {
    let canonical = canonical_executable(invocation)?;
    let binary_name = binary_name();
    for candidate in std::iter::once(invocation).chain(hints.iter().map(PathBuf::as_path)) {
        if versioned_keg(candidate).is_some() {
            continue;
        }
        let candidate = if candidate.file_name() == Some(OsStr::new(&binary_name)) {
            candidate.to_path_buf()
        } else {
            candidate.with_file_name(&binary_name)
        };
        if names(&candidate, &canonical) {
            return Ok(LaunchExecutable {
                path: candidate,
                basis: LaunchBasis::Invocation,
            });
        }
    }
    let Some(keg) = versioned_keg(&canonical) else {
        return Ok(LaunchExecutable {
            path: canonical,
            basis: LaunchBasis::Invocation,
        });
    };
    let opt = keg
        .prefix
        .join("opt")
        .join(&keg.package)
        .join(&keg.relative);
    if names(&opt, &canonical) {
        return Ok(LaunchExecutable {
            path: opt,
            basis: LaunchBasis::PackageOpt,
        });
    }
    let bin = keg.prefix.join("bin").join(&binary_name);
    if names(&bin, &canonical) {
        return Ok(LaunchExecutable {
            path: bin,
            basis: LaunchBasis::PackageBin,
        });
    }
    Ok(LaunchExecutable {
        path: canonical,
        basis: LaunchBasis::VersionedKeg,
    })
}

#[cfg(test)]
pub(crate) fn resolve_agent_session_executable_from(invocation: &Path) -> io::Result<PathBuf> {
    launch_executable_from(invocation, &[]).map(|launch| launch.path)
}

/// `argv[0]` as an absolute path: used as given when it names a directory,
/// otherwise searched through `path_env` like a shell would.
fn invocation_path(argv0: &OsStr, path_env: Option<&OsStr>) -> Option<PathBuf> {
    if argv0.as_bytes().contains(&b'/') {
        return std::path::absolute(argv0).ok();
    }
    env::split_paths(path_env?)
        .map(|dir| dir.join(argv0))
        .find(|candidate| candidate.is_absolute() && is_executable_file(candidate))
}

pub(crate) fn launch_executable() -> io::Result<LaunchExecutable> {
    let current_executable = env::current_exe()?;
    // Unit tests call launch helpers from the hashed `deps` harness; bind those
    // calls to Cargo's same-profile binary without changing production lookup.
    #[cfg(test)]
    if current_executable
        .parent()
        .and_then(Path::file_name)
        .is_some_and(|name| name == OsStr::new("deps"))
    {
        let profile_dir = current_executable
            .parent()
            .and_then(Path::parent)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    "test executable has no Cargo profile directory",
                )
            })?;
        let test_binary = profile_dir.join(format!("agent-session{}", env::consts::EXE_SUFFIX));
        return launch_executable_from(&test_binary, &[]);
    }
    let hints: Vec<PathBuf> = env::args_os()
        .next()
        .and_then(|argv0| invocation_path(&argv0, env::var_os("PATH").as_deref()))
        .into_iter()
        .collect();
    launch_executable_from(&current_executable, &hints)
}

pub(crate) fn resolve_agent_session_executable() -> io::Result<PathBuf> {
    launch_executable().map(|launch| launch.path)
}

#[cfg(test)]
#[path = "stable_executable_tests.rs"]
mod tests;
