use crate::git;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

pub fn run(args: &[String]) -> i32 {
    if args.iter().any(|arg| arg == "-h" || arg == "--help") {
        print_usage_stdout();
        return 0;
    }

    if let Some(arg) = args.first() {
        eprintln!("error: unknown argument: {arg}");
        print_usage_stderr();
        return 1;
    }

    if !git::is_inside_work_tree() {
        eprintln!("error: must run inside a git work tree");
        return 1;
    }

    match git::has_staged_changes() {
        Ok(true) => {}
        Ok(false) => {
            eprintln!("error: no staged changes (stage files with git add first)");
            return 2;
        }
        Err(err) => {
            eprintln!("{err:#}");
            return 1;
        }
    }

    let (context_json, patch) = match build_bundle() {
        Ok(bundle) => bundle,
        Err(err) => {
            eprintln!("{err:#}");
            return 1;
        }
    };

    println!("===== commit-context.json =====");
    println!("{context_json}");
    println!();
    println!("===== staged.patch =====");
    if let Err(err) = std::io::stdout().write_all(&patch) {
        eprintln!("{err:#}");
        return 1;
    }

    0
}

fn build_bundle() -> anyhow::Result<(String, Vec<u8>)> {
    let generated_at = OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .ok()
        .map(|s| s.to_string());

    let repo_root = git_string(&["rev-parse", "--show-toplevel"])?;
    let repo_name = Path::new(&repo_root)
        .file_name()
        .and_then(|name| name.to_str())
        .map(|s| s.to_string());

    let branch = git_string_ok(&["symbolic-ref", "--quiet", "--short", "HEAD"]);
    let head = git_string_ok(&["rev-parse", "--short", "HEAD"]);

    let name_status = git_bytes(&[
        "-c",
        "core.quotepath=false",
        "diff",
        "--cached",
        "--name-status",
        "-z",
    ])?;

    let mut status_counts: BTreeMap<String, u32> = BTreeMap::new();
    let mut top_dir_counts: BTreeMap<String, u32> = BTreeMap::new();
    let mut insertions: i64 = 0;
    let mut deletions: i64 = 0;
    let mut file_count: i64 = 0;
    let mut binary_file_count: i64 = 0;
    let mut lockfile_count: i64 = 0;
    let mut root_file_count: i64 = 0;

    let mut files: Vec<Value> = Vec::new();
    for entry in parse_name_status_z(&name_status)? {
        file_count += 1;

        *status_counts.entry(entry.status.clone()).or_insert(0) += 1;

        if let Some((top, _)) = entry.path.split_once('/') {
            *top_dir_counts.entry(top.to_string()).or_insert(0) += 1;
        } else {
            root_file_count += 1;
        }

        let lockfile = is_lockfile(&entry.path);
        if lockfile {
            lockfile_count += 1;
        }

        let (file_insertions, file_deletions, binary) = diff_numstat(&entry.path)?;
        if binary {
            binary_file_count += 1;
        } else {
            if let Some(n) = file_insertions {
                insertions += n;
            }
            if let Some(n) = file_deletions {
                deletions += n;
            }
        }

        let mut obj = serde_json::Map::new();
        obj.insert("path".to_string(), json!(entry.path));
        obj.insert("status".to_string(), json!(entry.status));
        if let Some(score) = entry.score {
            obj.insert("score".to_string(), json!(score));
        }
        if let Some(old_path) = entry.old_path {
            obj.insert("oldPath".to_string(), json!(old_path));
        }
        obj.insert(
            "insertions".to_string(),
            file_insertions.map_or(Value::Null, |n| json!(n)),
        );
        obj.insert(
            "deletions".to_string(),
            file_deletions.map_or(Value::Null, |n| json!(n)),
        );
        obj.insert("binary".to_string(), json!(binary));
        obj.insert("lockfile".to_string(), json!(lockfile));
        files.push(Value::Object(obj));
    }

    let status_counts: Vec<Value> = status_counts
        .into_iter()
        .map(|(status, count)| json!({ "status": status, "count": count }))
        .collect();

    let top_level_dirs: Vec<Value> = top_dir_counts
        .iter()
        .map(|(name, count)| json!({ "name": name, "count": count }))
        .collect();

    let context = json!({
        "schemaVersion": 1,
        "generatedAt": generated_at,
        "repo": { "name": repo_name },
        "git": { "branch": branch, "head": head },
        "staged": {
            "summary": {
                "fileCount": file_count,
                "insertions": insertions,
                "deletions": deletions,
                "binaryFileCount": binary_file_count,
                "lockfileCount": lockfile_count,
                "rootFileCount": root_file_count,
                "topLevelDirCount": top_level_dirs.len(),
            },
            "statusCounts": status_counts,
            "structure": {
                "topLevelDirs": top_level_dirs,
            },
            "files": files,
            "patch": { "path": "staged.patch", "format": "git diff --cached" }
        },
    });

    let context_json = serde_json::to_string(&context)?;
    let patch = git_bytes(&[
        "-c",
        "core.quotepath=false",
        "diff",
        "--cached",
        "--no-color",
    ])?;

    Ok((context_json, patch))
}

struct NameStatusEntry {
    status: String,
    score: Option<u32>,
    path: String,
    old_path: Option<String>,
}

fn parse_name_status_z(buf: &[u8]) -> anyhow::Result<Vec<NameStatusEntry>> {
    let parts: Vec<&[u8]> = buf.split(|b| *b == 0).filter(|p| !p.is_empty()).collect();
    let mut out: Vec<NameStatusEntry> = Vec::new();

    let mut i = 0;
    while i < parts.len() {
        let raw_status = std::str::from_utf8(parts[i])?;
        i += 1;

        let (path, old_path) = if raw_status.starts_with('R') || raw_status.starts_with('C') {
            let old = parts
                .get(i)
                .ok_or_else(|| anyhow::anyhow!("error: malformed name-status output"))?;
            let new = parts
                .get(i + 1)
                .ok_or_else(|| anyhow::anyhow!("error: malformed name-status output"))?;
            i += 2;
            (
                std::str::from_utf8(new)?.to_string(),
                Some(std::str::from_utf8(old)?.to_string()),
            )
        } else {
            let file = parts
                .get(i)
                .ok_or_else(|| anyhow::anyhow!("error: malformed name-status output"))?;
            i += 1;
            (std::str::from_utf8(file)?.to_string(), None)
        };

        let status_letter = raw_status
            .chars()
            .next()
            .ok_or_else(|| anyhow::anyhow!("error: malformed name-status output"))?;
        let status = status_letter.to_string();

        let score = raw_status[status_letter.len_utf8()..].parse::<u32>().ok();

        out.push(NameStatusEntry {
            status,
            score,
            path,
            old_path,
        });
    }

    Ok(out)
}

fn diff_numstat(path: &str) -> anyhow::Result<(Option<i64>, Option<i64>, bool)> {
    let stdout = git_string(&[
        "-c",
        "core.quotepath=false",
        "diff",
        "--cached",
        "--numstat",
        "--",
        path,
    ])?;

    let line = stdout.lines().next().unwrap_or("").to_string();
    if line.trim().is_empty() {
        return Ok((None, None, false));
    }

    let mut parts = line.split('\t');
    let added = parts.next().unwrap_or("");
    let deleted = parts.next().unwrap_or("");

    if added == "-" || deleted == "-" {
        return Ok((None, None, true));
    }

    let ins = added.parse::<i64>().ok();
    let del = deleted.parse::<i64>().ok();
    Ok((ins, del, false))
}

fn is_lockfile(path: &str) -> bool {
    let name = Path::new(path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    matches!(
        name,
        "yarn.lock"
            | "package-lock.json"
            | "pnpm-lock.yaml"
            | "bun.lockb"
            | "bun.lock"
            | "npm-shrinkwrap.json"
    )
}

fn git_string(args: &[&str]) -> anyhow::Result<String> {
    let output = Command::new("git")
        .args(args)
        .env("GIT_PAGER", "cat")
        .env("PAGER", "cat")
        .stdin(Stdio::null())
        .output()?;

    if !output.status.success() {
        return Err(anyhow::anyhow!(
            "error: git command failed: git {}\n{}{}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout)
        ));
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn git_string_ok(args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .args(args)
        .env("GIT_PAGER", "cat")
        .env("PAGER", "cat")
        .stdin(Stdio::null())
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let s = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

fn git_bytes(args: &[&str]) -> anyhow::Result<Vec<u8>> {
    let output = Command::new("git")
        .args(args)
        .env("GIT_PAGER", "cat")
        .env("PAGER", "cat")
        .stdin(Stdio::null())
        .output()?;

    if !output.status.success() {
        return Err(anyhow::anyhow!(
            "error: git command failed: git {}\n{}{}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout)
        ));
    }

    Ok(output.stdout)
}

fn print_usage_stdout() {
    print_usage(false);
}

fn print_usage_stderr() {
    print_usage(true);
}

fn print_usage(stderr: bool) {
    let out: &mut dyn std::io::Write = if stderr {
        &mut std::io::stderr()
    } else {
        &mut std::io::stdout()
    };

    let _ = writeln!(out, "Usage: semantic-commit staged-context");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "Print staged change context for commit message generation."
    );
    let _ = writeln!(out);
    let _ = writeln!(out, "Outputs:");
    let _ = writeln!(out, "  - Bundle: commit-context.json + staged.patch");
}
