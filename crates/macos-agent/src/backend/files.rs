use std::io;

use super::*;

pub(super) fn run_tool(program: &Path, args: &[&str]) -> Result<process::ProcessOutput, CliError> {
    let program = if program.components().count() == 1 {
        let name = program
            .to_str()
            .ok_or_else(|| backend_error("required backend tool name is invalid"))?;
        backend_tool(name)?
    } else {
        program.to_path_buf()
    };
    let args = args
        .iter()
        .map(|arg| (*arg).to_string())
        .collect::<Vec<_>>();
    process::run(&program, &args, &[], &[], None, Duration::from_secs(30))
        .map_err(|_| backend_error("required backend verification tool is unavailable"))
}

pub(super) fn backend_tool(name: &str) -> Result<PathBuf, CliError> {
    let logical = Path::new(name);
    if let Some(path) = test_mode::verification_tool_override(logical) {
        return Ok(path);
    }
    production_tool_path(name)
        .map(Path::to_path_buf)
        .ok_or_else(|| backend_error("required backend tool is not allowlisted"))
}

pub(super) fn production_tool_path(name: &str) -> Option<&'static Path> {
    match name {
        "lsof" => Some(Path::new("/usr/sbin/lsof")),
        "curl" => Some(Path::new("/usr/bin/curl")),
        "tar" => Some(Path::new("/usr/bin/tar")),
        "unzip" => Some(Path::new("/usr/bin/unzip")),
        "lipo" => Some(Path::new("/usr/bin/lipo")),
        "codesign" => Some(Path::new("/usr/bin/codesign")),
        "spctl" => Some(Path::new("/usr/sbin/spctl")),
        _ => None,
    }
}

pub(super) fn copy_tree(source: &Path, destination: &Path) -> Result<(), CliError> {
    let metadata = fs::symlink_metadata(source)
        .map_err(|_| backend_error("failed to inspect a backend file during activation"))?;
    if metadata.file_type().is_symlink() {
        let target = fs::read_link(source)
            .map_err(|_| backend_error("failed to read an internal app symlink"))?;
        if target.is_absolute() {
            return Err(backend_error("refusing to copy an absolute app symlink"));
        }
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)
                .map_err(|_| backend_error("failed to create app symlink parent"))?;
        }
        symlink(target, destination)
            .map_err(|_| backend_error("failed to copy an internal app symlink"))?;
        return Ok(());
    }
    if metadata.is_dir() {
        fs::create_dir_all(destination)
            .map_err(|_| backend_error("failed to create backend app directory"))?;
        fs::set_permissions(destination, metadata.permissions())
            .map_err(|_| backend_error("failed to preserve backend app permissions"))?;
        for entry in fs::read_dir(source)
            .map_err(|_| backend_error("failed to enumerate backend app directory"))?
        {
            let entry = entry.map_err(|_| backend_error("failed to read backend app entry"))?;
            copy_tree(&entry.path(), &destination.join(entry.file_name()))?;
        }
        return Ok(());
    }
    copy_file(source, destination, metadata.permissions().mode())
}

pub(super) fn copy_file(source: &Path, destination: &Path, mode: u32) -> Result<(), CliError> {
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)
            .map_err(|_| backend_error("failed to create backend file parent"))?;
    }
    fs::copy(source, destination).map_err(|_| backend_error("failed to copy backend file"))?;
    fs::set_permissions(destination, fs::Permissions::from_mode(mode))
        .map_err(|_| backend_error("failed to set backend file permissions"))?;
    Ok(())
}

pub(super) fn create_private_dir(path: &Path) -> Result<(), CliError> {
    reject_symlink_components(path)?;
    fs::create_dir_all(path)
        .map_err(|_| backend_error("failed to create private backend storage"))?;
    reject_symlink_components(path)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|_| backend_error("failed to secure private backend storage"))?;
    Ok(())
}

pub(super) fn reject_symlink_components(path: &Path) -> Result<(), CliError> {
    let mut current = PathBuf::new();
    for component in path.components() {
        current.push(component.as_os_str());
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                if allowed_platform_symlink(&current) {
                    continue;
                }
                return Err(backend_error(
                    "private backend storage path contains a symlink",
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => break,
            Err(_) => {
                return Err(backend_error(
                    "failed to inspect private backend storage path",
                ));
            }
        }
    }
    Ok(())
}

pub(super) fn allowed_platform_symlink(path: &Path) -> bool {
    #[cfg(target_os = "macos")]
    {
        let expected_target = match path.to_str() {
            Some("/var") => Path::new("private/var"),
            Some("/tmp") => Path::new("private/tmp"),
            Some("/etc") => Path::new("private/etc"),
            _ => return false,
        };
        fs::read_link(path).ok().as_deref() == Some(expected_target)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = path;
        false
    }
}

pub(super) fn hash_file(path: &Path) -> Result<String, CliError> {
    sha256_file(path).map_err(|_| backend_error("failed to hash a backend file"))
}
