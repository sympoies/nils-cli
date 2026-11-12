use std::ffi::OsStr;
use std::path::Path;

/// The directory that holds `git-cli` for a test executable's directory.
///
/// Cargo places test executables in `<profile>/deps` or, from Cargo 1.100, in
/// `<profile>/build/<package>/<hash>/out`. Any other directory, including an
/// `out` directory outside that exact shape, is returned unchanged.
pub(super) fn profile_dir(executable_dir: &Path) -> &Path {
    let profile = match executable_dir.file_name().and_then(OsStr::to_str) {
        Some("deps") => executable_dir.parent(),
        Some("out") => executable_dir
            .ancestors()
            .nth(3)
            .filter(|build| build.file_name() == Some(OsStr::new("build")))
            .and_then(Path::parent),
        _ => None,
    };
    profile.unwrap_or(executable_dir)
}

#[cfg(test)]
mod tests {
    use super::profile_dir;
    use pretty_assertions::assert_eq;
    use std::path::Path;

    #[test]
    fn profile_dir_resolves_both_cargo_test_layouts() {
        let profile = Path::new("/target/debug");
        assert_eq!(profile_dir(&profile.join("deps")), profile);
        assert_eq!(profile_dir(&profile.join("build/pkg/0123/out")), profile);
    }

    #[test]
    fn profile_dir_keeps_an_out_directory_outside_the_cargo_build_shape() {
        for dir in [
            "/work/out",
            "/work/a/b/out",
            "/work/build/out",
            "/work/x/pkg/hash/out",
        ] {
            assert_eq!(profile_dir(Path::new(dir)), Path::new(dir), "{dir}");
        }
        assert_eq!(
            profile_dir(Path::new("/target/debug")),
            Path::new("/target/debug")
        );
    }
}
