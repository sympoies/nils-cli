use anyhow::{Context, Result};
use nils_common::git as common_git;
use std::fs;
use std::io::{Read, Write};
use std::path::Path;
use std::process::Command;
use std::sync::OnceLock;
use tempfile::NamedTempFile;

#[derive(Debug, Clone, Copy)]
pub enum PrintSource {
    Worktree,
    Index,
}

#[derive(Debug, Clone, Copy)]
pub enum HeadFallback {
    FromHead,
    DeletedInIndex,
}

pub fn emit_file(source: PrintSource, path: &str, fallback: HeadFallback) -> Result<()> {
    if path.is_empty() {
        println!("❗ Missing file path");
        return Ok(());
    }

    match source {
        PrintSource::Worktree => {
            if Path::new(path).is_file() {
                return emit_from_worktree(path);
            }
            if git_has_object(&format!("HEAD:{path}"))? {
                return emit_from_head(path, fallback);
            }
            println!("❗ File not found: {path}");
            Ok(())
        }
        PrintSource::Index => {
            if git_has_object(&format!(":{path}"))? {
                return emit_from_index(path);
            }
            if git_has_object(&format!("HEAD:{path}"))? {
                return emit_from_head(path, fallback);
            }
            println!("❗ File not found in index: {path}");
            Ok(())
        }
    }
}

pub fn emit_file_from_commit(commit: &str, path: &str) -> Result<()> {
    if path.is_empty() {
        println!("❗ Missing file path");
        return Ok(());
    }

    let tmp = new_temp_file()?;
    if git_show_to_file(&format!("{commit}:{path}"), tmp.path()).is_err() {
        println!("❗ File not found in commit {commit}: {path}");
        return Ok(());
    }

    if is_binary_file(tmp.path())? {
        println!("📄 {path} (binary file in {commit})");
        println!("🔹 [Binary file content omitted]");
    } else {
        println!("📄 {path} (from {commit})");
        println!("```");
        let content = fs::read(tmp.path())?;
        std::io::stdout().write_all(&content)?;
        println!();
        println!("```");
    }

    Ok(())
}

fn emit_from_worktree(path: &str) -> Result<()> {
    if is_binary_file(Path::new(path))? {
        println!("📄 {path} (binary file in working tree)");
        println!("🔹 [Binary file content omitted]");
    } else {
        println!("📄 {path} (working tree)");
        println!("```");
        let content = fs::read(path)?;
        std::io::stdout().write_all(&content)?;
        println!();
        println!("```");
    }
    Ok(())
}

fn emit_from_index(path: &str) -> Result<()> {
    let tmp = new_temp_file()?;
    if git_show_to_file(&format!(":{path}"), tmp.path()).is_err() {
        println!("❗ Failed to read file from index: {path}");
        return Ok(());
    }

    if is_binary_file(tmp.path())? {
        println!("📄 {path} (binary file in index)");
        println!("🔹 [Binary file content omitted]");
    } else {
        println!("📄 {path} (index)");
        println!("```");
        let content = fs::read(tmp.path())?;
        std::io::stdout().write_all(&content)?;
        println!();
        println!("```");
    }

    Ok(())
}

fn emit_from_head(path: &str, fallback: HeadFallback) -> Result<()> {
    let tmp = new_temp_file()?;
    git_show_to_file(&format!("HEAD:{path}"), tmp.path())?;

    if is_binary_file(tmp.path())? {
        match fallback {
            HeadFallback::DeletedInIndex => {
                println!("📄 {path} (deleted in index; binary file in HEAD)");
            }
            _ => {
                println!("📄 {path} (binary file in HEAD)");
            }
        }
        println!("🔹 [Binary file content omitted]");
    } else {
        match fallback {
            HeadFallback::DeletedInIndex => {
                println!("📄 {path} (deleted in index; from HEAD)");
            }
            _ => {
                println!("📄 {path} (from HEAD)");
            }
        }
        println!("```");
        let content = fs::read(tmp.path())?;
        std::io::stdout().write_all(&content)?;
        println!();
        println!("```");
    }

    Ok(())
}

fn git_has_object(spec: &str) -> Result<bool> {
    let status = common_git::run_status_quiet(&["cat-file", "-e", spec])?;
    Ok(status.success())
}

fn git_show_to_file(spec: &str, dest: &Path) -> Result<()> {
    let output =
        common_git::run_output(&["show", spec]).with_context(|| format!("git show {spec}"))?;
    if !output.status.success() {
        anyhow::bail!("git show {spec} failed");
    }
    let mut file = fs::File::create(dest)?;
    file.write_all(&output.stdout)?;
    Ok(())
}

fn is_binary_file(path: &Path) -> Result<bool> {
    let path_display = path.display();

    if let Some(false) = file_mime_available().get() {
        return is_binary_file_fallback(path);
    }

    match Command::new("file").arg("--mime").arg(path).output() {
        Ok(output) => {
            file_mime_available().get_or_init(|| true);
            if output.status.success() {
                let text = String::from_utf8_lossy(&output.stdout);
                return Ok(text.contains("charset=binary"));
            }
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            file_mime_available().get_or_init(|| false);
            return is_binary_file_fallback(path);
        }
        Err(err) => return Err(err).with_context(|| format!("file --mime {path_display}")),
    }

    is_binary_file_fallback(path)
}

fn file_mime_available() -> &'static OnceLock<bool> {
    static FILE_MIME_AVAILABLE: OnceLock<bool> = OnceLock::new();
    &FILE_MIME_AVAILABLE
}

fn is_binary_file_fallback(path: &Path) -> Result<bool> {
    let path_display = path.display();
    const CHUNK_SIZE: usize = 8192;
    let mut file = fs::File::open(path).with_context(|| format!("open {path_display}"))?;
    let mut buf = [0u8; CHUNK_SIZE];
    let n = file
        .read(&mut buf)
        .with_context(|| format!("read {path_display}"))?;
    Ok(buf[..n].contains(&0))
}

fn new_temp_file() -> Result<NamedTempFile> {
    NamedTempFile::new().context("create temp file")
}
