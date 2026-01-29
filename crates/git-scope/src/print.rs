use anyhow::{Context, Result};
use std::fs;
use std::io::Write;
use std::process::{Command, Stdio};

pub fn print_file_content(path: &str) -> Result<()> {
    if path.is_empty() {
        println!("❗ Missing file path");
        return Ok(());
    }

    if std::path::Path::new(path).is_file() {
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
        return Ok(());
    }

    if git_has_object(&format!("HEAD:{path}"))? {
        let tmp = mktemp_path()?;
        git_show_to_file(&format!("HEAD:{path}"), &tmp)?;
        if is_binary_file(&tmp)? {
            println!("📄 {path} (binary file in HEAD)");
            println!("🔹 [Binary file content omitted]");
        } else {
            println!("📄 {path} (from HEAD)");
            println!("```");
            let content = fs::read(&tmp)?;
            std::io::stdout().write_all(&content)?;
            println!();
            println!("```");
        }
        let _ = fs::remove_file(tmp);
        return Ok(());
    }

    println!("❗ File not found: {path}");
    Ok(())
}

pub fn print_file_content_index(path: &str) -> Result<()> {
    if path.is_empty() {
        println!("❗ Missing file path");
        return Ok(());
    }

    if git_has_object(&format!(":{path}"))? {
        let tmp = mktemp_path()?;
        if !git_show_to_file(&format!(":{path}"), &tmp).is_ok() {
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
        return Ok(());
    }

    if git_has_object(&format!("HEAD:{path}"))? {
        let tmp = mktemp_path()?;
        git_show_to_file(&format!("HEAD:{path}"), &tmp)?;
        if is_binary_file(&tmp)? {
            println!("📄 {path} (deleted in index; binary file in HEAD)");
            println!("🔹 [Binary file content omitted]");
        } else {
            println!("📄 {path} (deleted in index; from HEAD)");
            println!("```");
            let content = fs::read(&tmp)?;
            std::io::stdout().write_all(&content)?;
            println!();
            println!("```");
        }
        let _ = fs::remove_file(tmp);
        return Ok(());
    }

    println!("❗ File not found in index: {path}");
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
    let output = Command::new("file")
        .arg("--mime")
        .arg(path)
        .output()
        .with_context(|| format!("file --mime {path}"))?;

    if !output.status.success() {
        return Ok(false);
    }
    let text = String::from_utf8_lossy(&output.stdout);
    Ok(text.contains("charset=binary"))
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
