//! Create a signed candidate without moving HEAD, then conditionally install it.
use std::ffi::{OsStr, OsString};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Output, Stdio};

use nils_common::forge_identity::{self, Authorization};

use super::preflight::{RepositoryState, verify_identity_after};

#[path = "install_message.rs"]
mod message;

pub(crate) struct Installation<'a> {
    state: &'a RepositoryState,
    pub(crate) installed: Option<String>,
}

impl<'a> Installation<'a> {
    pub(crate) fn new(state: &'a RepositoryState) -> Self {
        Self {
            state,
            installed: None,
        }
    }

    pub(crate) fn commit(
        &mut self,
        message: &Path,
        signoff: bool,
        trailers: &[String],
    ) -> anyhow::Result<ExitStatus> {
        let git = CommitGit::new(&self.state.root)?;
        let result = self.create_and_install(&git, message, signoff, trailers);
        if let Some(auth) = git.authorization.as_ref() {
            auth.finish(result.is_ok(), self.installed.as_deref())?;
        }
        result
    }

    fn create_and_install(
        &mut self,
        git: &CommitGit,
        message: &Path,
        signoff: bool,
        trailers: &[String],
    ) -> anyhow::Result<ExitStatus> {
        let cleanup_mode = message::cleanup_mode(git)?;
        // Match native commit admission: a busy index refuses before hooks.
        git.checked(["update-index", "--refresh"])?;
        git.checked(["hook", "run", "--ignore-missing", "pre-commit"])?;
        let message = message::prepare(git, message, signoff, trailers, &cleanup_mode)?;
        let tree = git.text(["write-tree"])?;
        let old_tree = git.text(["rev-parse", &format!("{}^{{tree}}", self.state.head)])?;
        anyhow::ensure!(tree != old_tree, "no staged changes after commit hooks");
        let candidate = git.text([
            OsStr::new("commit-tree"),
            OsStr::new("-S"),
            tree.as_ref(),
            OsStr::new("-p"),
            self.state.head.as_ref(),
            OsStr::new("-F"),
            message.as_os_str(),
        ])?;
        git.checked(["verify-commit", &candidate])?;
        anyhow::ensure!(
            git.text(["log", "-1", "--format=%G?", &candidate])? == "G",
            "candidate commit signature is not locally verified-good"
        );
        // Recheck identity and staged content before touching the branch. The
        // branch's old value is checked atomically by update-ref, not here.
        verify_identity_after(self.state).map_err(anyhow::Error::msg)?;
        anyhow::ensure!(
            git.text(["write-tree"])? == tree,
            "staged tree changed during candidate creation"
        );
        let branch = format!("refs/heads/{}", self.state.default_branch);
        let subject = git.text(["log", "-1", "--format=%s", &candidate])?;
        let output = git.output(
            [
                "update-ref",
                "-m",
                &format!("commit: {subject}"),
                &branch,
                &candidate,
                &self.state.head,
            ],
            None,
        )?;
        if !output.status.success() {
            anyhow::bail!(
                "conditional default-branch installation refused: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        // Never recover the identity from HEAD: a post-commit hook or another
        // writer may advance it after this successful conditional installation.
        self.installed = Some(candidate);
        // Git's post-commit hook is advisory, just as with ordinary git commit.
        let post = git.output(["hook", "run", "--ignore-missing", "post-commit"], None)?;
        std::io::stderr().write_all(&post.stderr)?;
        Ok(output.status)
    }
}

struct CommitGit {
    root: PathBuf,
    index: PathBuf,
    environment: Vec<(OsString, Option<OsString>)>,
    authorization: Option<Authorization>,
}

impl CommitGit {
    fn new(root: &Path) -> anyhow::Result<Self> {
        let mut template = Command::new("git");
        // Authorize the logical commit once, preserving the same managed
        // identity and signing configuration as the ordinary commit path.
        let authorization = forge_identity::prepare_git(&mut template, Some(root), &["commit"])?;
        let environment = template
            .get_envs()
            .map(|(k, v)| (k.to_owned(), v.map(OsStr::to_owned)))
            .collect();
        Ok(Self {
            root: root.to_path_buf(),
            index: super::git::Git::at(root.to_path_buf())
                .stdout(["rev-parse", "--path-format=absolute", "--git-path", "index"])
                .map(PathBuf::from)
                .map_err(anyhow::Error::msg)?,
            environment,
            authorization,
        })
    }

    fn output<I, S>(&self, args: I, input: Option<&[u8]>) -> anyhow::Result<Output>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let mut command = Command::new("git");
        command
            .current_dir(&self.root)
            .args(args)
            .env("GIT_PAGER", "cat")
            .env("PAGER", "cat")
            .env("GIT_EDITOR", ":")
            .env("GIT_INDEX_FILE", &self.index)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (key, value) in &self.environment {
            match value {
                Some(value) => {
                    command.env(key, value);
                }
                None => {
                    command.env_remove(key);
                }
            }
        }
        command.stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        });
        let mut child = command.spawn()?;
        if let Some(input) = input {
            child.stdin.take().expect("piped stdin").write_all(input)?;
        }
        let mut output = child.wait_with_output()?;
        if let Some(auth) = self.authorization.as_ref() {
            auth.redact_output(&mut output);
        }
        Ok(output)
    }

    fn checked<I, S>(&self, args: I) -> anyhow::Result<Output>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let output = self.output(args, None)?;
        std::io::stderr().write_all(&output.stderr)?;
        anyhow::ensure!(
            output.status.success(),
            "git command failed (exit code: {})",
            output.status.code().unwrap_or(1)
        );
        Ok(output)
    }

    fn text<I, S>(&self, args: I) -> anyhow::Result<String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let output = self.checked(args)?;
        let value = String::from_utf8(output.stdout)?.trim().to_owned();
        anyhow::ensure!(!value.is_empty(), "git command returned empty output");
        Ok(value)
    }
}
