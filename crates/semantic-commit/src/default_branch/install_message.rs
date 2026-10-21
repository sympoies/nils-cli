use std::fs;
use std::path::{Path, PathBuf};

use super::CommitGit;

pub(super) fn cleanup_mode(git: &CommitGit) -> anyhow::Result<String> {
    let output = git.output(["config", "--get", "commit.cleanup"], None)?;
    let mode = if output.status.code() == Some(1) {
        "whitespace"
    } else {
        anyhow::ensure!(output.status.success(), "failed to resolve commit.cleanup");
        std::str::from_utf8(&output.stdout)?.trim()
    };
    anyhow::ensure!(
        matches!(
            mode,
            "verbatim" | "strip" | "default" | "whitespace" | "scissors"
        ),
        "invalid commit.cleanup mode"
    );
    Ok(mode.to_owned())
}

pub(super) fn prepare(
    git: &CommitGit,
    source: &Path,
    signoff: bool,
    trailers: &[String],
    mode: &str,
) -> anyhow::Result<PathBuf> {
    let path = PathBuf::from(git.text([
        "rev-parse",
        "--path-format=absolute",
        "--git-path",
        "COMMIT_EDITMSG",
    ])?);
    let mut contents = fs::read(source)?;
    // Native commit cleans input whitespace before preparing hooks, except in
    // verbatim mode, which preserves the exact message-file bytes.
    if mode != "verbatim" {
        let initial = git.output(["stripspace"], Some(&contents))?;
        anyhow::ensure!(
            initial.status.success(),
            "initial commit message cleanup failed"
        );
        contents = initial.stdout;
    } else if (signoff || !trailers.is_empty()) && !contents.ends_with(b"\n") {
        // interpret-trailers needs a terminated subject paragraph when adding
        // trailers; this must not alter verbatim input without those options.
        contents.push(b'\n');
    }
    fs::write(&path, contents)?;
    if signoff {
        let ident = git.text(["var", "GIT_COMMITTER_IDENT"])?;
        let identity = ident
            .rsplit_once('>')
            .map(|(value, _)| format!("{value}>"))
            .ok_or_else(|| anyhow::anyhow!("invalid committer identity"))?;
        git.checked([
            "interpret-trailers",
            "--in-place",
            "--if-exists",
            "addIfDifferent",
            "--trailer",
            &format!("Signed-off-by: {identity}"),
            path.to_str()
                .ok_or_else(|| anyhow::anyhow!("invalid message path"))?,
        ])?;
    }
    for trailer in trailers {
        git.checked([
            std::ffi::OsStr::new("interpret-trailers"),
            std::ffi::OsStr::new("--in-place"),
            std::ffi::OsStr::new("--trailer"),
            trailer.as_ref(),
            path.as_os_str(),
        ])?;
    }
    git.checked([
        std::ffi::OsStr::new("hook"),
        std::ffi::OsStr::new("run"),
        std::ffi::OsStr::new("--ignore-missing"),
        std::ffi::OsStr::new("prepare-commit-msg"),
        std::ffi::OsStr::new("--"),
        path.as_os_str(),
        std::ffi::OsStr::new("message"),
    ])?;
    git.checked([
        std::ffi::OsStr::new("hook"),
        std::ffi::OsStr::new("run"),
        std::ffi::OsStr::new("--ignore-missing"),
        std::ffi::OsStr::new("commit-msg"),
        std::ffi::OsStr::new("--"),
        path.as_os_str(),
    ])?;
    let args = match mode {
        "verbatim" => return Ok(path),
        "strip" => vec!["stripspace", "--strip-comments"],
        "default" | "whitespace" | "scissors" => vec!["stripspace"],
        _ => anyhow::bail!("invalid commit.cleanup mode"),
    };
    let cleaned = git.output(args, Some(&fs::read(&path)?))?;
    anyhow::ensure!(cleaned.status.success(), "commit message cleanup failed");
    anyhow::ensure!(
        !String::from_utf8_lossy(&cleaned.stdout).trim().is_empty(),
        "empty commit message after hooks"
    );
    fs::write(&path, cleaned.stdout)?;
    Ok(path)
}
