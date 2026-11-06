//! Resolve the configured HTTPS helper once, verify its actor, then pin its token.
use super::{Authorization, Error, LoadedPolicy, Operation, Result, Target, probe};
use std::ffi::OsStr;
use std::path::Path;
use std::process::Command;

impl LoadedPolicy {
    pub(super) fn authorize_https(
        &self,
        target: &Target,
        path: &Path,
        cwd: Option<&Path>,
        url: &str,
        operation: Operation,
    ) -> Result<Authorization> {
        // Resolve session/principal/profile/operation before invoking any helper.
        let selected = self.select(target, Some(path), operation);
        let result = selected.as_ref().map_err(Clone::clone).and_then(|selection| {
            let token = credential(cwd, url, selection.profile.expected_login.as_deref())?;
            self.verify_token(selection, OsStr::new("gh"), token)
        }).map_err(|error| match error.code {
            "identity_actor_mismatch" => Error::with_detail(error.code,
                "HTTPS credential helper account does not match the selected identity profile; configure the credential helper for the expected account or repair the identity profile".into())
                .with_details(serde_json::json!({"retryable":false,"next_action":"repair_https_credential"})),
            "identity_credential_missing" => Error::with_detail(error.code,
                "configure a forge credential helper for the selected HTTPS remote and authenticate the expected account".into()),
            "identity_credential_helper_untrusted" => Error::with_detail(error.code,
                "configure the HTTPS credential helper at system/global scope; repository/worktree credential helper entries are not trusted".into()),
            "identity_credential_helper_unavailable" => Error::with_detail(error.code,
                "cannot inspect credential helper scopes; repair Git configuration before retrying".into()),
            _ => error,
        });
        match &result {
            Ok(auth) => self.audit(
                Some(&auth.selection),
                target,
                operation,
                "authorized",
                Some(&auth.actor),
            )?,
            Err(error) => {
                self.audit(selected.as_ref().ok(), target, operation, error.code, None)?
            }
        }
        result
    }
}

fn credential(cwd: Option<&Path>, url: &str, username: Option<&str>) -> Result<String> {
    reject_local_config(
        cwd,
        "^credential(\\..*)?\\.helper$",
        "identity_credential_helper_untrusted",
        "identity_credential_helper_unavailable",
    )?;
    // Keep lookup out of repository discovery even if local config changes after
    // inspection. The directory never contains a credential or helper script.
    let isolated = tempfile::Builder::new()
        .prefix("forge-https-credential-")
        .tempdir()
        .map_err(|_| Error::new("identity_probe_unavailable"))?;
    // Git's credential protocol takes input on stdin. This fixed pipeline passes
    // only the already-validated public URL and login as arguments; the token is captured
    // privately by the bounded probe, never placed in argv or a file.
    let mut command = Command::new("sh");
    command.args([
        "-c",
        "{ printf 'url=%s\\n' \"$1\"; if [ -n \"$2\" ]; then printf 'username=%s\\n' \"$2\"; fi; printf '\\n'; } | git -c credential.interactive=false credential fill",
        "forge-https-credential",
        url,
        username.unwrap_or(""),
    ]);
    command
        .current_dir(isolated.path())
        .env("GIT_CEILING_DIRECTORIES", isolated.path());
    protect(&mut command);
    command
        .env_remove("GH_DEBUG")
        .env_remove("DEBUG")
        .env("GCM_INTERACTIVE", "never");
    let output = probe::run(&mut command)?;
    if !output.status.success() {
        return Err(Error::new("identity_credential_missing"));
    }
    let text =
        String::from_utf8(output.stdout).map_err(|_| Error::new("identity_credential_missing"))?;
    let mut passwords = text
        .lines()
        .filter_map(|line| line.strip_prefix("password="));
    let token = passwords
        .next()
        .ok_or(Error::new("identity_credential_missing"))?;
    if passwords.next().is_some() {
        return Err(Error::new("identity_credential_missing"));
    }
    Ok(token.to_owned())
}

pub(super) fn reject_local_transport(cwd: Option<&Path>) -> Result<()> {
    reject_local_config(
        cwd,
        "^http(\\..*)?\\.(proxy|sslcainfo|sslcapath)$",
        "identity_transport_config_untrusted",
        "identity_transport_config_unavailable",
    )
    .map_err(|error| {
        let detail = if error.code == "identity_transport_config_untrusted" {
            "configure HTTP proxy and CA settings at system/global scope; repository/worktree http.proxy, http.sslCAInfo, and http.sslCAPath entries (including URL-specific forms) are not trusted"
        } else {
            "cannot inspect HTTP proxy and CA configuration scopes; repair Git configuration before retrying"
        };
        Error::with_detail(error.code, detail.into())
    })
}

fn reject_local_config(
    cwd: Option<&Path>,
    pattern: &str,
    untrusted: &'static str,
    unavailable: &'static str,
) -> Result<()> {
    let mut command = Command::new("git");
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    command.args([
        "config",
        "--includes",
        "--null",
        "--show-scope",
        "--get-regexp",
        pattern,
    ]);
    protect(&mut command);
    let output = probe::run(&mut command)?;
    match output.status.code() {
        Some(1) => return Ok(()), // Git's documented no-match result.
        Some(0) => {}
        _ => return Err(Error::new(unavailable)),
    }
    // With --null, Git emits alternating scope and key/value records. Values
    // may contain newlines, so never interpret a value as another scope.
    let mut fields = output.stdout.split(|byte| *byte == 0);
    while let Some(scope) = fields.next() {
        if scope.is_empty() && fields.next().is_none() {
            break;
        }
        if fields.next().is_none() {
            return Err(Error::new(unavailable));
        }
        match scope {
            b"local" | b"worktree" => {
                return Err(Error::new(untrusted));
            }
            b"global" | b"system" => {}
            _ => return Err(Error::new(unavailable)),
        }
    }
    Ok(())
}

pub(super) fn protect(command: &mut Command) {
    command
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_ASKPASS", "false")
        .env_remove("GIT_SSL_NO_VERIFY")
        .env_remove("GIT_CURL_VERBOSE");
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("GIT_TRACE") {
            command.env_remove(key);
        }
    }
}
