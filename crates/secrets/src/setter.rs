//! Stdin single-key encrypted-write transaction; delivery is explicitly handed off.
use std::collections::BTreeSet;
use std::fs;
use std::io::{self, IsTerminal, Read, Write};
use std::path::{Component, Path};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use nils_common::cli_contract::OutputFormat;
use serde_json::{Value, json};

use crate::runtime::{
    self, AddSignalPhase, CmdError, Env, Outcome, PrivateTempFile, ensure_add_not_interrupted,
    enter_add_commit_phase, run_add_sops,
};

const VALUE_LIMIT: u64 = 1024 * 1024;
const DOCUMENT_LIMIT: u64 = 16 * 1024 * 1024;
const WORKTREE_GUIDANCE: &str = "select a clean governed non-default managed store worktree: use git-cli worktree add in the store repository, configure origin/HEAD locally, then set SECRETS_REPO to that worktree; inspect and commit any pending changes separately";

pub(crate) fn set(name: &str, key: &str, format: OutputFormat) -> i32 {
    runtime::dispatch(format, |env| {
        let env = Env {
            store_root: fs::canonicalize(&env.store_root)
                .map_err(|_| CmdError::unavailable(WORKTREE_GUIDANCE, "store-worktree-required"))?,
            cwd: env.cwd.clone(),
            selection_source: env.selection_source.clone(),
            selection_match: env.selection_match.clone(),
        };
        set_with(&env, name, key)
    })
}

fn set_with(env: &Env, name: &str, key: &str) -> Result<SetOutcome, CmdError> {
    if !identifier(key) || key.starts_with("sops_") {
        return Err(CmdError::runtime("invalid dotenv key (sops_ is reserved)"));
    }
    safe_relative(Path::new(name))?;
    env.ensure_store()?;
    let branch = governed_worktree(&env.store_root, None)?;
    let entry = runtime::resolve_entry(env, Some(name))?;
    safe_relative(Path::new(&entry.rel))?;
    reject_symlinks(&env.store_root, Path::new(&entry.rel))?;
    if io::stdin().is_terminal() {
        return Err(CmdError::runtime(
            "terminal stdin refused; pipe UTF-8 bytes through EOF using a producer that does not echo the value or put it in argv",
        ));
    }

    let _phase = AddSignalPhase::begin()?;
    let value = read_value()?;
    let original = if entry.exists {
        sops_capture(env, &entry.rel, false, &entry.path, None)?
    } else {
        Vec::new()
    };
    let document = replace_key(&original, key, &value)?;
    let ciphertext = sops_capture(
        env,
        &entry.rel,
        true,
        Path::new("/dev/stdin"),
        Some(document.as_bytes().to_vec()),
    )?;
    ensure_ciphertext(&ciphertext)?;
    ensure_add_not_interrupted()?;

    let parent = entry
        .path
        .parent()
        .ok_or_else(|| CmdError::runtime("invalid entry parent"))?;
    fs::create_dir_all(parent)
        .map_err(|_| CmdError::runtime("cannot create encrypted entry directory"))?;
    let candidate = PrivateTempFile::new(parent)?;
    candidate
        .reopen()?
        .write_all(&ciphertext)
        .map_err(|_| CmdError::runtime("cannot write encrypted candidate; target unchanged"))?;
    if sops_capture(env, &entry.rel, false, candidate.path(), None)? != document.as_bytes() {
        return Err(CmdError::runtime(
            "encrypted candidate failed complete SOPS validation; target unchanged",
        ));
    }
    // The selected worktree must still be clean immediately before installation.
    // Nothing has been staged; ignore only our exact owned ciphertext candidate.
    if governed_worktree(&env.store_root, Some(candidate.path()))? != branch {
        return Err(CmdError::runtime(
            "store branch changed before installation; target unchanged",
        ));
    }
    enter_add_commit_phase()?;
    candidate.persist(&entry.path)?;
    Ok(SetOutcome {
        entry: entry.rel,
        branch,
        store: fs::canonicalize(&env.store_root).unwrap_or_else(|_| env.store_root.clone()),
    })
}

fn identifier(key: &str) -> bool {
    let mut bytes = key.bytes();
    bytes
        .next()
        .is_some_and(|byte| byte.is_ascii_alphabetic() || byte == b'_')
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

fn safe_relative(path: &Path) -> Result<(), CmdError> {
    if path.as_os_str().is_empty()
        || path.components().any(|part| {
            !matches!(part, Component::Normal(_))
                || part.as_os_str() == ".git"
                || part.as_os_str() == ".sops.yaml"
        })
        || path.to_string_lossy().chars().any(char::is_control)
    {
        return Err(CmdError::runtime(
            "entry must be a safe store-relative path",
        ));
    }
    Ok(())
}

fn reject_symlinks(root: &Path, rel: &Path) -> Result<(), CmdError> {
    let mut path = root.to_path_buf();
    for part in rel.components() {
        path.push(part);
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(CmdError::runtime("symlink entry paths are refused"));
            }
            Ok(_) => {}
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(_) => return Err(CmdError::runtime("cannot inspect entry path")),
        }
    }
    Ok(())
}

fn metadata_command(root: &Path, program: &str, args: &[&str]) -> Result<Vec<u8>, CmdError> {
    let output = Command::new(program)
        .args(args)
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .map_err(|_| CmdError::unavailable(WORKTREE_GUIDANCE, "store-worktree-required"))?;
    if !output.status.success() {
        return Err(CmdError::unavailable(
            WORKTREE_GUIDANCE,
            "store-worktree-required",
        ));
    }
    Ok(output.stdout)
}

fn governed_worktree(root: &Path, owned: Option<&Path>) -> Result<String, CmdError> {
    let refuse = || CmdError::unavailable(WORKTREE_GUIDANCE, "store-worktree-required");
    if !root.join(".git").is_file() {
        return Err(refuse());
    }
    let canonical = fs::canonicalize(root).map_err(|_| refuse())?;
    let top = metadata_command(root, "git", &["rev-parse", "--show-toplevel"])?;
    if fs::canonicalize(String::from_utf8_lossy(&top).trim())
        .ok()
        .as_ref()
        != Some(&canonical)
    {
        return Err(refuse());
    }
    let branch = metadata_command(root, "git", &["symbolic-ref", "--quiet", "--short", "HEAD"])?;
    let branch = String::from_utf8(branch)
        .map_err(|_| refuse())?
        .trim()
        .to_string();
    let default = metadata_command(
        root,
        "git",
        &[
            "symbolic-ref",
            "--quiet",
            "--short",
            "refs/remotes/origin/HEAD",
        ],
    )?;
    let default = String::from_utf8(default).map_err(|_| refuse())?;
    let default = default
        .trim()
        .strip_prefix("origin/")
        .filter(|s| !s.is_empty())
        .ok_or_else(refuse)?;
    if branch.is_empty() || branch == default {
        return Err(refuse());
    }
    let listing = metadata_command(root, "git-cli", &["worktree", "list", "--format", "json"])?;
    let listing: Value = serde_json::from_slice(&listing).map_err(|_| refuse())?;
    let managed = listing["ok"] == true
        && listing["data"]["entries"]
            .as_array()
            .is_some_and(|entries| {
                entries.iter().any(|entry| {
                    entry["managed"] == true
                        && entry["detached"] == false
                        && entry["bare"] == false
                        && entry["prunable"].as_bool() != Some(true)
                        && entry["branch"].as_str() == Some(branch.as_str())
                        && entry["path"]
                            .as_str()
                            .and_then(|path| fs::canonicalize(path).ok())
                            .as_ref()
                            == Some(&canonical)
                })
            });
    if !managed {
        return Err(refuse());
    }
    check_clean(root, owned)?;
    Ok(branch)
}

fn check_clean(root: &Path, owned: Option<&Path>) -> Result<(), CmdError> {
    let status = metadata_command(
        root,
        "git",
        &["status", "--porcelain", "-z", "--untracked-files=all"],
    )?;
    let owned_line = owned
        .and_then(|path| path.strip_prefix(root).ok())
        .map(|path| format!("?? {}", path.display()).into_bytes());
    if status
        .split(|byte| *byte == 0)
        .filter(|line| !line.is_empty())
        .any(|line| Some(line) != owned_line.as_deref())
    {
        return Err(CmdError::unavailable(
            WORKTREE_GUIDANCE,
            "store-worktree-required",
        ));
    }
    Ok(())
}

/// Read on a worker so a handled termination signal can cancel even a stalled producer.
fn read_value() -> Result<String, CmdError> {
    let (send, receive) = mpsc::sync_channel(1);
    thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = io::stdin()
            .take(VALUE_LIMIT + 1)
            .read_to_end(&mut bytes)
            .map(|_| bytes);
        let _ = send.send(result);
    });
    loop {
        ensure_add_not_interrupted()?;
        match receive.recv_timeout(Duration::from_millis(10)) {
            Ok(Ok(bytes)) => {
                if bytes.len() as u64 > VALUE_LIMIT {
                    return Err(CmdError::runtime(
                        "stdin value exceeds 1 MiB; target unchanged",
                    ));
                }
                let value = String::from_utf8(bytes)
                    .map_err(|_| CmdError::runtime("stdin must be UTF-8; target unchanged"))?;
                if value
                    .chars()
                    .any(|c| c.is_control() && c != '\n' && c != '\t')
                {
                    return Err(CmdError::runtime(
                        "stdin contains unsupported control characters; target unchanged",
                    ));
                }
                return Ok(value);
            }
            Ok(Err(_)) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err(CmdError::runtime("cannot read stdin; target unchanged"));
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    }
}

fn replace_key(original: &[u8], key: &str, value: &str) -> Result<String, CmdError> {
    let document = std::str::from_utf8(original).map_err(|_| {
        CmdError::runtime("existing dotenv document is not UTF-8; target unchanged")
    })?;
    let mut escaped = String::new();
    for c in value.chars() {
        match c {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '$' => escaped.push_str("\\$"),
            '\n' => escaped.push_str("\\n"),
            '\t' => escaped.push_str("\\t"),
            _ => escaped.push(c),
        }
    }
    let replacement = format!("{key}=\"{escaped}\"\n");
    let mut seen = BTreeSet::new();
    let mut updated = false;
    let mut output = String::new();
    for line in document.split_inclusive('\n') {
        let content = line.strip_suffix('\n').unwrap_or(line);
        if !content.is_empty() && !content.starts_with('#') {
            let (existing, _) = content.split_once('=').ok_or_else(|| {
                CmdError::runtime("unsupported dotenv assignment; target unchanged")
            })?;
            if !identifier(existing)
                || existing.starts_with("sops_")
                || !seen.insert(existing)
                || content.contains(['\r', '\0'])
            {
                return Err(CmdError::runtime(
                    "ambiguous or unsupported dotenv assignment; target unchanged",
                ));
            }
            if existing == key {
                output.push_str(&replacement);
                updated = true;
                continue;
            }
        }
        output.push_str(line);
    }
    if !updated {
        if !output.is_empty() && !output.ends_with('\n') {
            output.push('\n');
        }
        output.push_str(&replacement);
    }
    Ok(output)
}

/// Capture only in memory; child diagnostics can contain plaintext and are discarded.
fn sops_capture(
    env: &Env,
    rel: &str,
    encrypt: bool,
    source: &Path,
    input: Option<Vec<u8>>,
) -> Result<Vec<u8>, CmdError> {
    let (reader, writer) =
        io::pipe().map_err(|_| CmdError::runtime("cannot create SOPS output pipe"))?;
    let read = thread::spawn(move || {
        let mut bytes = Vec::new();
        reader
            .take(DOCUMENT_LIMIT + 1)
            .read_to_end(&mut bytes)
            .map(|_| bytes)
    });
    let mut command = Command::new("sops");
    command
        .args([
            if encrypt { "-e" } else { "-d" },
            "--input-type",
            "dotenv",
            "--output-type",
            "dotenv",
            "--filename-override",
        ])
        .arg(rel)
        .current_dir(&env.store_root)
        .stdin(Stdio::null())
        .stdout(writer)
        .stderr(Stdio::null());
    command
        .arg("--config")
        .arg(env.store_root.join(".sops.yaml"))
        .env_remove("SOPS_CONFIG");
    if encrypt {
        // Recipient overrides would bypass the selected store's creation rules.
        for variable in [
            "SOPS_AGE_RECIPIENTS",
            "SOPS_PGP_FP",
            "SOPS_KMS_ARN",
            "SOPS_GCP_KMS_IDS",
            "SOPS_HUAWEICLOUD_KMS_IDS",
            "SOPS_AZURE_KEYVAULT_URLS",
            "SOPS_VAULT_URIS",
        ] {
            command.env_remove(variable);
        }
    }
    command.arg(source);
    let write = if let Some(input) = input {
        let (reader, mut writer) =
            io::pipe().map_err(|_| CmdError::runtime("cannot create SOPS input pipe"))?;
        command.stdin(reader);
        Some(thread::spawn(move || writer.write_all(&input)))
    } else {
        None
    };
    let result = run_add_sops(&mut command);
    // Close the command's parent pipe descriptors before joining workers.
    drop(command);
    let written = write
        .map(|handle| handle.join().ok().and_then(Result::ok).is_some())
        .unwrap_or(true);
    let bytes = read
        .join()
        .ok()
        .and_then(Result::ok)
        .ok_or_else(|| CmdError::runtime("cannot read SOPS result; target unchanged"))?;
    if !result?.success() || !written || bytes.len() as u64 > DOCUMENT_LIMIT {
        return Err(CmdError::runtime(
            "SOPS operation failed; target unchanged (diagnostics suppressed)",
        ));
    }
    Ok(bytes)
}

fn ensure_ciphertext(bytes: &[u8]) -> Result<(), CmdError> {
    let document = std::str::from_utf8(bytes)
        .map_err(|_| CmdError::runtime("SOPS produced invalid ciphertext"))?;
    let mut has_key = false;
    let mut has_mac = false;
    for line in document
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
    {
        let (key, value) = line
            .split_once('=')
            .ok_or_else(|| CmdError::runtime("SOPS produced invalid ciphertext"))?;
        if key == "sops_mac" {
            has_mac = true;
        }
        if !key.starts_with("sops_") {
            has_key = true;
            if !value.starts_with("ENC[AES256_GCM,") || !value.ends_with(']') {
                return Err(CmdError::runtime(
                    "SOPS rules left an unencrypted value; target unchanged",
                ));
            }
        }
    }
    if !has_key || !has_mac {
        return Err(CmdError::runtime(
            "SOPS produced incomplete ciphertext; target unchanged",
        ));
    }
    Ok(())
}

struct SetOutcome {
    entry: String,
    branch: String,
    store: std::path::PathBuf,
}

impl Outcome for SetOutcome {
    fn command(&self) -> &'static str {
        "set"
    }
    fn human(&self) -> String {
        format!(
            "Encrypted write completed for {}; delivery pending (not committed, no PR verified). In store worktree {} on {}, stage only this entry, use semantic-commit for a signed commit, git-cli push, then forge-cli pr deliver --no-merge. Inspect pending changes before retrying.",
            self.entry,
            self.store.display(),
            self.branch
        )
    }
    fn to_json(&self) -> Value {
        json!({"store": self.store, "entry": self.entry, "branch": self.branch, "encrypted_write_completed": true,
            "committed": false, "pr_created": false, "delivery_verified": false, "delivery_status": "pending",
            "next_actions": ["In the selected store worktree, inspect the diff and stage only the reported entry with git add -- <entry>.",
                "Use semantic-commit to create a signed commit; do not include unrelated changes.",
                "Publish the non-default branch with git-cli push, then use forge-cli pr deliver --no-merge with a metadata-only body.",
                "Verify the commit signature and exact PR head through the governed delivery workflow. Until then delivery remains pending; if signing/provider delivery fails, preserve this encrypted change and repair delivery without replaying set."]})
    }
}
