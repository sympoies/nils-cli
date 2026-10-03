use super::{Authorization, Error, Operation, Profile, Result, Target, load, managed_path};
use std::path::Path;
use std::process::Command;

fn raw(cwd: Option<&Path>, args: &[&str]) -> Result<String> {
    let mut command = Command::new("git");
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    let output = command
        .args(args)
        .output()
        .map_err(|_| Error::new("identity_target_unknown"))?;
    if !output.status.success() {
        return Err(Error::new("identity_target_unknown"));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}
fn optional(cwd: Option<&Path>, key: &str) -> Option<String> {
    raw(cwd, &["config", "--get", key])
        .ok()
        .filter(|s| !s.is_empty())
}

fn command_tail<'a>(args: &'a [&'a str]) -> Result<&'a [&'a str]> {
    let mut i = 0;
    while i < args.len() {
        match args[i] {
            "-c" => {
                let value = *args.get(i + 1).ok_or(Error::new("identity_git_override"))?;
                if !(value.starts_with("core.quotepath=") || value.starts_with("core.pager=")) {
                    return Err(Error::new("identity_git_override"));
                }
                i += 2;
            }
            arg if arg.starts_with('-') => return Err(Error::new("identity_git_override")),
            _ => return Ok(&args[i..]),
        }
    }
    Ok(&[])
}
fn operation(tail: &[&str]) -> Result<Option<Operation>> {
    match tail.first().copied() {
        Some("merge") if tail.contains(&"--ff-only") => Ok(None),
        Some("commit" | "merge" | "cherry-pick" | "rebase" | "am") => Ok(Some(Operation::Commit)),
        Some("push") => Ok(Some(Operation::GitPush)),
        Some("fetch" | "ls-remote") => Ok(Some(Operation::GitRead)),
        Some("remote") if tail.contains(&"--auto") => Ok(Some(Operation::GitRead)),
        Some("clone" | "pull" | "submodule" | "stash" | "tag") => {
            Err(Error::new("identity_git_operation_unsupported"))
        }
        _ => Ok(None),
    }
}
fn default_remote(cwd: Option<&Path>) -> Result<String> {
    let branch = raw(cwd, &["symbolic-ref", "--quiet", "--short", "HEAD"])?;
    let selected = optional(cwd, &format!("branch.{branch}.pushRemote"))
        .or_else(|| optional(cwd, "remote.pushDefault"))
        .or_else(|| optional(cwd, &format!("branch.{branch}.remote")));
    if let Some(remote) = selected
        && remote != "."
    {
        return Ok(remote);
    }
    let remotes = raw(cwd, &["remote"])?;
    let names: Vec<_> = remotes.lines().collect();
    if names.len() != 1 {
        return Err(Error::new("identity_target_ambiguous"));
    }
    Ok(names[0].to_string())
}
fn selected_remote(cwd: Option<&Path>, tail: &[&str], op: Operation) -> Result<String> {
    if op == Operation::Commit {
        return default_remote(cwd);
    }
    if tail.first() == Some(&"remote") {
        return tail
            .get(2)
            .map(|s| s.to_string())
            .ok_or(Error::new("identity_target_ambiguous"));
    }
    // Managed callers use options with attached values or flags only. Refuse options
    // consuming a separate argument instead of guessing which positional is the remote.
    let mut positional = Vec::new();
    for arg in &tail[1..] {
        if arg.starts_with('-') {
            if matches!(
                arg.split('=').next().unwrap(),
                "--repo"
                    | "--receive-pack"
                    | "--upload-pack"
                    | "--exec"
                    | "-o"
                    | "--push-option"
                    | "--config-env"
                    | "--multiple"
            ) {
                return Err(Error::new("identity_git_override"));
            }
        } else {
            positional.push(*arg);
        }
    }
    positional
        .first()
        .map(|s| s.to_string())
        .ok_or(Error::new("identity_target_ambiguous"))
}
pub fn target_for_remote(cwd: Option<&Path>, remote: &str, push: bool) -> Result<(Target, String)> {
    // URL rewrite rules could send a verified credential to an unrelated destination.
    if raw(
        cwd,
        &[
            "config",
            "--get-regexp",
            "^url\\..*\\.(insteadOf|pushInsteadOf)$",
        ],
    )
    .is_ok()
    {
        return Err(Error::new("identity_transport_override"));
    }
    let urls = if remote.contains(":") || remote.contains('/') {
        remote.to_string()
    } else {
        let mut args = vec!["remote", "get-url"];
        if push {
            args.push("--push");
        }
        args.extend(["--all", remote]);
        raw(cwd, &args)?
    };
    let urls: Vec<_> = urls.lines().collect();
    if urls.len() != 1 {
        return Err(Error::new("identity_target_ambiguous"));
    }
    let url = urls[0];
    // GitHub HTTPS and SSH repository shapes only; SSH is routed through pinned HTTPS
    // in this child process so a shared SSH agent cannot select another actor.
    if !(url.starts_with("https://") || url.starts_with("ssh://git@") || url.starts_with("git@")) {
        return Err(Error::new("identity_transport_unsupported"));
    }
    if url.starts_with("https://")
        && url
            .trim_start_matches("https://")
            .split('/')
            .next()
            .is_some_and(|authority| authority.contains('@') || authority.contains(':'))
    {
        return Err(Error::new("identity_transport_override"));
    }
    if url.starts_with("ssh://")
        && url
            .trim_start_matches("ssh://git@")
            .split('/')
            .next()
            .is_some_and(|host| host.contains(':'))
    {
        return Err(Error::new("identity_transport_unsupported"));
    }
    let parsed =
        crate::git::parse_git_remote_url(url).ok_or(Error::new("identity_target_unknown"))?;
    let target = Target::new(&parsed.host, &parsed.path)?;
    if url.contains(['\n', '\r', '?', '#']) {
        return Err(Error::new("identity_transport_override"));
    }
    Ok((target, url.to_string()))
}

pub fn verify_key(profile: &Profile) -> Result<()> {
    let out = Command::new("gpg")
        .args([
            "--batch",
            "--with-colons",
            "--with-fingerprint",
            "--list-secret-keys",
            &profile.signing_fingerprint,
        ])
        .output()
        .map_err(|_| Error::new("identity_signing_key_missing"))?;
    if !out.status.success() {
        return Err(Error::new("identity_signing_key_missing"));
    }
    let mut usable = false;
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let fields: Vec<_> = line.split(':').collect();
        match fields.first().copied() {
            Some("sec" | "ssb") => {
                usable = fields
                    .get(1)
                    .is_some_and(|v| !matches!(*v, "r" | "e" | "d"))
                    && fields.get(11).is_some_and(|c| c.contains('s'))
                    && fields.get(14).is_some_and(|c| *c != "#");
            }
            Some("fpr")
                if usable
                    && fields
                        .get(9)
                        .is_some_and(|f| f.eq_ignore_ascii_case(&profile.signing_fingerprint)) =>
            {
                return Ok(());
            }
            _ => {}
        }
    }
    Err(Error::new("identity_signing_key_missing"))
}
fn verify_local_identity(cwd: Option<&Path>, profile: &Profile) -> Result<()> {
    for (key, expected) in [
        ("user.name", &profile.commit_name),
        ("user.email", &profile.commit_email),
        ("user.signingkey", &profile.signing_fingerprint),
    ] {
        if let Ok(actual) = raw(cwd, &["config", "--local", "--get", key]) {
            let matches = if key == "user.signingkey" {
                actual.trim_end_matches('!').eq_ignore_ascii_case(expected)
            } else {
                actual == *expected
            };
            if !matches {
                return Err(Error::new("identity_commit_mismatch"));
            }
        }
    }
    for (env, expected) in [
        ("GIT_AUTHOR_NAME", &profile.commit_name),
        ("GIT_AUTHOR_EMAIL", &profile.commit_email),
        ("GIT_COMMITTER_NAME", &profile.commit_name),
        ("GIT_COMMITTER_EMAIL", &profile.commit_email),
    ] {
        if std::env::var(env).is_ok_and(|actual| actual != *expected) {
            return Err(Error::new("identity_commit_mismatch"));
        }
    }
    Ok(())
}
const HELPER: &str = "!f() { test \"$1\" = get || exit 0; p= h= r=; while IFS= read -r line; do case \"$line\" in protocol=*) p=${line#protocol=};; host=*) h=${line#host=};; path=*) r=${line#path=};; esac; done; test \"$p\" = https && test \"$h\" = \"$FORGE_IDENTITY_HOST\" && test \"$r\" = \"$FORGE_IDENTITY_REPO.git\" || exit 1; printf 'username=x-access-token\\npassword=%s\\n' \"$FORGE_IDENTITY_TOKEN\"; }; f";

/// Configure only this command. The shared runners call it after caller environments
/// are applied, and capture protected child output before redacting it.
fn prepare_git_inner(
    command: &mut Command,
    cwd: Option<&Path>,
    args: &[&str],
) -> Result<Option<Authorization>> {
    // Local inspections do not load policy or credentials.
    let mut i = 0;
    while i < args.len() && args[i].starts_with('-') {
        i += if args[i] == "-c" || args[i] == "-C" {
            2
        } else {
            1
        };
    }
    let preliminary = operation(args.get(i..).unwrap_or_default());
    if matches!(preliminary, Ok(None)) {
        return Ok(None);
    }
    let Some(policy) = load()? else {
        return Ok(None);
    };
    preliminary?;
    let tail = command_tail(args)?;
    let Some(op) = operation(tail)? else {
        return Ok(None);
    };
    if [
        "GIT_CONFIG_COUNT",
        "GIT_CONFIG_PARAMETERS",
        "GIT_CONFIG",
        "GIT_DIR",
        "GIT_WORK_TREE",
    ]
    .iter()
    .any(|k| std::env::var_os(k).is_some())
    {
        return Err(Error::new("identity_git_override"));
    }
    let remote = selected_remote(cwd, tail, op)?;
    let (target, url) = target_for_remote(cwd, &remote, op != Operation::GitRead)?;
    let path = managed_path(cwd)?;
    let auth = policy.authorize(&target, Some(&path), op, std::ffi::OsStr::new("gh"))?;
    let mut config: Vec<(String, String)> = Vec::new();
    if op == Operation::Commit {
        // History-producing commands require dedicated contracts; do not let rebase,
        // cherry-pick, merge or am preserve a different author silently.
        if tail.first() != Some(&"commit")
            || tail.iter().any(|a| {
                a.starts_with("--author")
                    || a.starts_with("--no-gpg-sign")
                    || *a == "--amend"
                    || a.starts_with("-S") && *a != "-S"
            })
        {
            policy.audit(
                Some(&auth.selection),
                &target,
                op,
                "identity_commit_override",
                Some(&auth.actor),
            )?;
            return Err(Error::new("identity_commit_override"));
        }
        if let Err(e) = verify_local_identity(cwd, &auth.selection.profile)
            .and_then(|_| verify_key(&auth.selection.profile))
        {
            policy.audit(
                Some(&auth.selection),
                &target,
                op,
                e.code,
                Some(&auth.actor),
            )?;
            return Err(e);
        }
        let profile = &auth.selection.profile;
        config.extend([
            (String::from("user.name"), profile.commit_name.clone()),
            (String::from("user.email"), profile.commit_email.clone()),
            (
                String::from("user.signingkey"),
                format!("{}!", profile.signing_fingerprint),
            ),
            (String::from("commit.gpgsign"), String::from("true")),
            (String::from("gpg.format"), String::from("openpgp")),
            (String::from("gpg.program"), String::from("gpg")),
        ]);
        command
            .env("GIT_AUTHOR_NAME", &profile.commit_name)
            .env("GIT_AUTHOR_EMAIL", &profile.commit_email)
            .env("GIT_COMMITTER_NAME", &profile.commit_name)
            .env("GIT_COMMITTER_EMAIL", &profile.commit_email);
    } else {
        if raw(
            cwd,
            &["config", "--get-regexp", "^http\\..*extra[Hh]eader$"],
        )
        .is_ok()
        {
            return Err(Error::new("identity_transport_override"));
        }
        let https = format!("https://{}/{}.git", target.host, target.repo);
        config.extend([
            (format!("url.{https}.insteadOf"), url),
            (String::from("credential.helper"), String::new()),
            (String::from("credential.helper"), HELPER.to_string()),
            (String::from("credential.useHttpPath"), String::from("true")),
            (String::from("http.followRedirects"), String::from("false")),
            (String::from("http.extraHeader"), String::new()),
        ]);
        command
            .env("FORGE_IDENTITY_HOST", &target.host)
            .env("FORGE_IDENTITY_REPO", &target.repo)
            .env("FORGE_IDENTITY_TOKEN", &auth.token)
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_ASKPASS", "false");
        for (key, _) in std::env::vars_os() {
            if key.to_string_lossy().starts_with("GIT_TRACE") {
                command.env_remove(key);
            }
        }
        command.env_remove("GIT_CURL_VERBOSE");
    }
    command.env("GIT_CONFIG_COUNT", config.len().to_string());
    for (i, (key, value)) in config.iter().enumerate() {
        command
            .env(format!("GIT_CONFIG_KEY_{i}"), key)
            .env(format!("GIT_CONFIG_VALUE_{i}"), value);
    }
    Ok(Some(auth))
}

pub fn prepare_git(
    command: &mut Command,
    cwd: Option<&Path>,
    args: &[&str],
) -> Result<Option<Authorization>> {
    let result = prepare_git_inner(command, cwd, args);
    if let Err(error) = &result {
        super::audit_refusal(error.code)?;
    }
    result
}
