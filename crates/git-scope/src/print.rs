use anyhow::{Context, Result};
use std::fs;
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::OnceLock;

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

fn emit_from_worktree(path: &str) -> Result<()> {
    if is_binary_file(path)? {
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
    let tmp = mktemp_path()?;
    if git_show_to_file(&format!(":{path}"), &tmp).is_err() {
        let _ = fs::remove_file(&tmp);
        println!("❗ Failed to read file from index: {path}");
        return Ok(());
    }

    if is_binary_file(&tmp)? {
        println!("📄 {path} (binary file in index)");
        println!("🔹 [Binary file content omitted]");
    } else {
        println!("📄 {path} (index)");
        println!("```");
        let content = fs::read(&tmp)?;
        std::io::stdout().write_all(&content)?;
        println!();
        println!("```");
    }

    let _ = fs::remove_file(&tmp);
    Ok(())
}

fn emit_from_head(path: &str, fallback: HeadFallback) -> Result<()> {
    let tmp = mktemp_path()?;
    git_show_to_file(&format!("HEAD:{path}"), &tmp)?;

    if is_binary_file(&tmp)? {
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
        let content = fs::read(&tmp)?;
        std::io::stdout().write_all(&content)?;
        println!();
        println!("```");
    }

    let _ = fs::remove_file(&tmp);
    Ok(())
}

fn git_has_object(spec: &str) -> Result<bool> {
    let status = Command::new("git")
        .args(["cat-file", "-e", spec])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    Ok(status.success())
}

fn git_show_to_file(spec: &str, dest: &str) -> Result<()> {
    let output = Command::new("git")
        .args(["show", spec])
        .output()
        .with_context(|| format!("git show {spec}"))?;
    if !output.status.success() {
        anyhow::bail!("git show {spec} failed");
    }
    let mut file = fs::File::create(dest)?;
    file.write_all(&output.stdout)?;
    Ok(())
}

fn is_binary_file(path: &str) -> Result<bool> {
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
        Err(err) => return Err(err).with_context(|| format!("file --mime {path}")),
    }

    is_binary_file_fallback(path)
}

fn file_mime_available() -> &'static OnceLock<bool> {
    static FILE_MIME_AVAILABLE: OnceLock<bool> = OnceLock::new();
    &FILE_MIME_AVAILABLE
}

fn is_binary_file_fallback(path: &str) -> Result<bool> {
    const CHUNK_SIZE: usize = 8192;
    let mut file = fs::File::open(path).with_context(|| format!("open {path}"))?;
    let mut buf = [0u8; CHUNK_SIZE];
    let n = file
        .read(&mut buf)
        .with_context(|| format!("read {path}"))?;
    Ok(buf[..n].contains(&0))
}

fn mktemp_path() -> Result<String> {
    if let Ok(path) = run_mktemp(&["mktemp"]) {
        return Ok(path);
    }

    if let Ok(path) = run_mktemp(&["mktemp", "-t", "git-scope.XXXXXX"]) {
        return Ok(path);
    }

    println!("❗ Failed to create temp file");
    anyhow::bail!("mktemp failed")
}

fn run_mktemp(args: &[&str]) -> Result<String> {
    let output = Command::new(args[0]).args(&args[1..]).output()?;
    if !output.status.success() {
        anyhow::bail!("mktemp failed")
    }
    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if path.is_empty() {
        anyhow::bail!("mktemp produced empty output")
    } else {
        Ok(path)
    }
}
