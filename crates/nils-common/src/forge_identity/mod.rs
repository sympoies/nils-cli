//! Optional, metadata-only forge identity policy shared by managed API and Git runners.
//! Configured sessions resolve the principal from authenticated immutable launch context.
mod git;
mod policy;
mod probe;
pub mod session;
pub use git::{
    authoring_remote, prepare_git, prepare_git_with_deadline, target_for_remote, verify_key,
};
pub use policy::{Credential, Operation, Policy, Profile, Selection, Target};
pub use probe::with_deadline as with_probe_deadline;

use serde::Serialize;
use sha2::{Digest, Sha256};
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::Duration;

pub const PRINCIPAL_ENV: &str = "FORGE_IDENTITY_PRINCIPAL";
pub const SESSION_ENV: &str = "FORGE_IDENTITY_SESSION";
pub type Result<T> = std::result::Result<T, Error>;
#[derive(Debug, Clone)]
pub struct Error {
    pub code: &'static str,
    detail: Option<String>,
    #[cfg_attr(not(test), allow(dead_code))]
    pub details: Option<serde_json::Value>,
}
impl Error {
    pub const fn new(code: &'static str) -> Self {
        Self {
            code,
            detail: None,
            details: None,
        }
    }
    fn with_detail(code: &'static str, detail: String) -> Self {
        Self {
            code,
            detail: Some(detail),
            details: None,
        }
    }
    fn with_details(mut self, details: serde_json::Value) -> Self {
        self.details = Some(details);
        self
    }
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code)?;
        if let Some(detail) = &self.detail {
            write!(f, ": {detail}")?;
        }
        Ok(())
    }
}
impl std::error::Error for Error {}
impl From<Error> for io::Error {
    fn from(e: Error) -> Self {
        io::Error::new(io::ErrorKind::PermissionDenied, e)
    }
}

pub struct LoadedPolicy {
    pub policy: Policy,
    pub digest: String,
}
fn config_home() -> Result<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .ok_or(Error::new("identity_config_unavailable"))
}
pub fn load() -> Result<Option<LoadedPolicy>> {
    let path = config_home()?.join("forge-cli/identity.toml");
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(Error::new("identity_policy_unreadable")),
    };
    let digest = Sha256::digest(text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    let policy = Policy::parse(&text)?;
    // Presence, including an empty/non-Unicode value, activates strict selection.
    // Parse first so an unreadable or malformed installed policy never bypasses.
    if !policy.require_session_binding
        && policy.activation == policy::Activation::AssertedOnly
        && std::env::var_os(PRINCIPAL_ENV).is_none()
    {
        return Ok(None);
    }
    Ok(Some(LoadedPolicy { policy, digest }))
}
pub fn principal() -> Result<String> {
    let p = std::env::var(PRINCIPAL_ENV).map_err(|_| Error::new("identity_principal_missing"))?;
    if !policy::identifier(&p) {
        return Err(Error::new("identity_principal_unknown"));
    }
    Ok(p)
}
impl LoadedPolicy {
    pub fn select(&self, target: &Target, path: Option<&Path>, op: Operation) -> Result<Selection> {
        if !self.policy.require_session_binding {
            return self.policy.resolve(&principal()?, target, path, op);
        }
        probe::with_deadline(None, || {
            let (principal, decision) = self.policy.session_decision(session::current()?)?;
            if let Some(asserted) = std::env::var_os(PRINCIPAL_ENV)
                && asserted != std::ffi::OsStr::new(&principal)
            {
                return Err(Error::new("identity_session_principal_mismatch"));
            }
            let mut selected = self.policy.resolve(&principal, target, path, op)?;
            selected.session_binding = Some(decision);
            Ok(selected)
        })
    }
    pub fn authorize(
        &self,
        target: &Target,
        path: Option<&Path>,
        op: Operation,
        gh: &std::ffi::OsStr,
    ) -> Result<Authorization> {
        probe::with_deadline(None, || self.authorize_inner(target, path, op, gh))
    }
    fn authorize_inner(
        &self,
        target: &Target,
        path: Option<&Path>,
        op: Operation,
        gh: &std::ffi::OsStr,
    ) -> Result<Authorization> {
        let selected = self.select(target, path, op);
        let result = selected
            .as_ref()
            .map_err(Clone::clone)
            .and_then(|s| self.verify(s, gh));
        match &result {
            Ok(auth) => self.audit(
                Some(&auth.selection),
                target,
                op,
                "authorized",
                Some(&auth.actor),
            )?,
            Err(e) => self.audit(selected.as_ref().ok(), target, op, e.code, None)?,
        }
        result
    }
    fn verify(&self, selection: &Selection, gh: &std::ffi::OsStr) -> Result<Authorization> {
        let credential = self
            .policy
            .credentials
            .get(&selection.profile.credential)
            .unwrap();
        let token = match credential {
            Credential::Env { name } => std::env::var(name).ok(),
            Credential::GhUser { user } => {
                let mut cmd = Command::new(gh);
                clean_gh(&mut cmd);
                cmd.args([
                    "auth",
                    "token",
                    "--hostname",
                    &selection.target.host,
                    "--user",
                    user,
                ]);
                let out = probe::run(&mut cmd).map_err(|e| {
                    if e.code == "identity_probe_unavailable" {
                        Error::new("identity_credential_missing")
                    } else {
                        e
                    }
                })?;
                if !out.status.success() {
                    return Err(Error::new("identity_credential_missing"));
                }
                String::from_utf8(out.stdout)
                    .ok()
                    .map(|s| s.trim().to_string())
            }
        }
        .filter(|v| {
            !v.is_empty()
                && v.len() <= 16384
                && !v
                    .bytes()
                    .any(|b| b.is_ascii_whitespace() || b.is_ascii_control())
        })
        .ok_or(Error::new("identity_credential_missing"))?;
        let mut auth = Authorization {
            selection: selection.clone(),
            actor: String::new(),
            token,
            policy_digest: self.digest.clone(),
        };
        let actor = if let Some(expected) = &selection.profile.expected_login {
            let data = auth.probe(gh, &["api", "user"])?;
            let login = data["login"]
                .as_str()
                .ok_or(Error::new("identity_actor_mismatch"))?;
            if !login.eq_ignore_ascii_case(expected) {
                return Err(Error::new("identity_actor_mismatch"));
            }
            expected.clone()
        } else {
            let slug = selection.profile.app_slug.as_ref().unwrap();
            let endpoint = format!("apps/{slug}");
            let app = auth.probe(gh, &["api", &endpoint])?;
            if app["id"].as_u64() != selection.profile.expected_app_id {
                return Err(Error::new("identity_actor_mismatch"));
            }
            let viewer = auth.probe(
                gh,
                &["api", "graphql", "-f", "query=query { viewer { login } }"],
            )?;
            let expected = format!("{slug}[bot]");
            if viewer["data"]["viewer"]["login"].as_str() != Some(&expected) {
                return Err(Error::new("identity_actor_mismatch"));
            }
            // Repo-scoped calls require installation coverage; cross-repository reads
            // have no individual repository to check.
            if !selection.target.repo.is_empty() {
                // An installation token must list this repository; a public repo GET alone proves no coverage.
                let pages = auth.probe(
                    gh,
                    &[
                        "api",
                        "installation/repositories?per_page=100",
                        "--paginate",
                    ],
                )?;
                let found = pages.as_array().is_some_and(|pages| {
                    pages.iter().any(|page| {
                        page["repositories"].as_array().is_some_and(|repos| {
                            repos.iter().any(|repo| {
                                repo["full_name"].as_str().is_some_and(|name| {
                                    name.eq_ignore_ascii_case(&selection.target.repo)
                                })
                            })
                        })
                    })
                });
                if !found {
                    return Err(Error::new("identity_app_repository_missing"));
                }
            }
            expected
        };
        auth.actor = actor;
        Ok(auth)
    }
    pub fn audit(
        &self,
        selection: Option<&Selection>,
        target: &Target,
        operation: Operation,
        outcome: &str,
        actor: Option<&str>,
    ) -> Result<()> {
        #[derive(Serialize)]
        struct Record<'a> {
            schema_version: &'static str,
            timestamp: String,
            policy_version: u32,
            policy_digest: &'a str,
            principal: Option<String>,
            session: Option<String>,
            target: &'a Target,
            operation: Operation,
            profile_id: Option<&'a str>,
            matched_rule: Option<&'a str>,
            signer: Option<&'a str>,
            actor: Option<&'a str>,
            outcome: &'a str,
            session_binding: Option<session::SessionBinding>,
            matched_launch_rule: Option<String>,
            asserted_principal: Option<String>,
        }
        let binding = selection
            .and_then(|s| s.session_binding.as_ref())
            .map(|d| d.binding.clone())
            .or_else(|| {
                self.policy
                    .require_session_binding
                    .then(|| session::current().ok())
                    .flatten()
            });
        let decision = binding
            .as_ref()
            .and_then(|b| self.policy.session_decision(b.clone()).ok());
        let record = Record {
            schema_version: "forge.identity.audit.v1",
            timestamp: jiff::Timestamp::now().to_string(),
            policy_version: 1,
            policy_digest: &self.digest,
            principal: selection
                .map(|s| s.principal.clone())
                .or_else(|| decision.as_ref().map(|(p, _)| p.clone()))
                .or_else(|| {
                    (!self.policy.require_session_binding)
                        .then(|| principal().ok())
                        .flatten()
                }),
            session: binding.as_ref().map(|b| b.session_id.clone()).or_else(|| {
                std::env::var(SESSION_ENV)
                    .ok()
                    .filter(|s| policy::identifier(s))
            }),
            target,
            operation,
            profile_id: selection.map(|s| s.profile_id.as_str()),
            matched_rule: selection.map(|s| s.matched_rule.as_str()),
            signer: selection
                .filter(|s| s.operation == Operation::Commit)
                .map(|s| s.profile.signing_fingerprint.as_str()),
            actor,
            outcome,
            session_binding: binding,
            matched_launch_rule: decision.map(|(_, d)| d.matched_launch_rule),
            asserted_principal: self
                .policy
                .require_session_binding
                .then(|| principal().ok())
                .flatten(),
        };
        append_audit(&record)
    }
}

fn append_audit(record: &impl Serialize) -> Result<()> {
    let state = std::env::var_os("XDG_STATE_HOME")
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))
        .ok_or(Error::new("identity_audit_unavailable"))?;
    let dir = state.join("forge-cli");
    fs::create_dir_all(&dir).map_err(|_| Error::new("identity_audit_unavailable"))?;
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let mut file = options
        .open(dir.join("identity-audit.jsonl"))
        .map_err(|_| Error::new("identity_audit_unavailable"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if file
            .metadata()
            .map_err(|_| Error::new("identity_audit_unavailable"))?
            .permissions()
            .mode()
            & 0o077
            != 0
        {
            return Err(Error::new("identity_audit_unavailable"));
        }
    }
    let mut bytes =
        serde_json::to_vec(record).map_err(|_| Error::new("identity_audit_unavailable"))?;
    bytes.push(b'\n');
    file.write_all(&bytes)
        .map_err(|_| Error::new("identity_audit_unavailable"))
}
/// Record a refusal whose repository or policy could not be resolved. No caller text is retained.
pub fn audit_refusal(code: &'static str) -> Result<()> {
    if let Some(loaded) = load().ok().flatten()
        && loaded.policy.require_session_binding
    {
        let binding = session::current().ok();
        let decision = binding
            .as_ref()
            .and_then(|b| loaded.policy.session_decision(b.clone()).ok());
        return append_audit(&serde_json::json!({
            "schema_version":"forge.identity.audit.v1", "timestamp":jiff::Timestamp::now().to_string(),
            "policy_version":1,"policy_digest":loaded.digest,
            "principal":decision.as_ref().map(|(p,_)|p),
            "session":binding.as_ref().map(|b|&b.session_id),
            "session_binding":binding,
            "matched_launch_rule":decision.map(|(_,d)|d.matched_launch_rule),
            "asserted_principal":principal().ok(),"outcome":code,"target":null
        }));
    }
    append_audit(
        &serde_json::json!({"schema_version":"forge.identity.audit.v1","timestamp":jiff::Timestamp::now().to_string(),"principal":principal().ok(),"outcome":code,"target":null}),
    )
}

/// Holds a credential only in memory. It deliberately has no Debug/Serialize implementation.
pub struct Authorization {
    pub selection: Selection,
    pub actor: String,
    token: String,
    policy_digest: String,
}
pub fn clean_gh(cmd: &mut Command) {
    for name in [
        "GH_TOKEN",
        "GITHUB_TOKEN",
        "GH_ENTERPRISE_TOKEN",
        "GITHUB_ENTERPRISE_TOKEN",
        "GH_HOST",
        "GH_DEBUG",
        "DEBUG",
    ] {
        cmd.env_remove(name);
    }
}
impl Authorization {
    pub fn finish(&self, success: bool, object: Option<&str>) -> Result<()> {
        append_audit(&serde_json::json!({
            "schema_version":"forge.identity.audit.v1","timestamp":jiff::Timestamp::now().to_string(),
            "policy_version":1,"policy_digest":self.policy_digest,"principal":self.selection.principal,
            "target":self.selection.target,"operation":self.selection.operation,"profile_id":self.selection.profile_id,
            "matched_rule":self.selection.matched_rule,"actor":self.actor,"session_binding":self.selection.session_binding,
            "signer":(self.selection.operation == Operation::Commit).then_some(&self.selection.profile.signing_fingerprint),
            "outcome":if success {"execution_succeeded"} else {"execution_failed"},"object":object
        })).map_err(|_| Error::new("identity_audit_failed_after_execution"))
    }

    pub fn apply_api(&self, cmd: &mut Command) {
        clean_gh(cmd);
        let key = if self.selection.target.host == "github.com" {
            "GH_TOKEN"
        } else {
            "GH_ENTERPRISE_TOKEN"
        };
        cmd.env(key, &self.token)
            .env("GH_HOST", &self.selection.target.host);
    }
    fn probe(&self, gh: &std::ffi::OsStr, args: &[&str]) -> Result<serde_json::Value> {
        actor_probe_with_retry(|| {
            let mut cmd = Command::new(gh);
            self.apply_api(&mut cmd);
            cmd.args(args)
                .args(["--hostname", &self.selection.target.host]);
            cmd.arg("--include");
            let output = probe::run(&mut cmd).map_err(|probe_error| {
                let classification = match probe_error.code {
                    "identity_probe_timeout" => "probe_timeout",
                    "identity_probe_output_limit" => "probe_output_limit",
                    _ => "probe_unavailable",
                };
                let mut error =
                    actor_probe_error("process_probe", classification, None, None, false);
                if matches!(
                    probe_error.code,
                    "identity_probe_timeout" | "identity_probe_output_limit"
                ) {
                    error.code = probe_error.code;
                }
                error
            })?;
            if args.contains(&"--paginate") {
                parse_paginated_actor_probe_response(&output)
            } else {
                parse_actor_probe_response(&output)
            }
        })
    }
    pub fn redact(&self, bytes: &mut Vec<u8>) {
        *bytes = String::from_utf8_lossy(bytes)
            .replace(&self.token, "[REDACTED]")
            .into_bytes();
    }
    pub fn redact_output(&self, output: &mut Output) {
        self.redact(&mut output.stdout);
        self.redact(&mut output.stderr);
    }
}

const ACTOR_PROBE_MAX_ATTEMPTS: u8 = 3;

fn actor_probe_error(
    stage: &'static str,
    classification: &'static str,
    http_status: Option<u16>,
    retry_after_seconds: Option<u64>,
    retryable: bool,
) -> Error {
    let mut cause = serde_json::json!({
        "operation": "github_actor_lookup",
        "stage": stage,
        "classification": classification,
    });
    if let Some(status) = http_status {
        cause["http_status"] = serde_json::json!(status);
    }
    if let Some(seconds) = retry_after_seconds {
        cause["retry_after_seconds"] = serde_json::json!(seconds);
    }
    Error::new("identity_actor_unavailable").with_details(serde_json::json!({
        "retryable": retryable,
        "next_action": if retryable { "retry_request" } else { "check_identity_configuration" },
        "recovery": { "kind": "github_actor_probe", "automatic": retryable },
        "cause": cause,
    }))
}

/// `gh` failed without an HTTP response, so nothing proves the credential or
/// configuration wrong; a transport failure is the retryable case.
fn no_response_failure() -> Error {
    actor_probe_error("process_exit", "nonzero_exit", None, None, true)
}

fn parse_actor_probe_response(output: &Output) -> Result<serde_json::Value> {
    let (status, retry_after, request_id, body) = parse_included_http_response(&output.stdout);
    if let Some(status) = status {
        if !(200..300).contains(&status) || !output.status.success() {
            let transient = status == 429 || (500..600).contains(&status);
            let mut error = actor_probe_error(
                "http_response",
                match status {
                    401 => "http_401",
                    403 => "http_403",
                    429 => "http_429",
                    500..=599 => "http_5xx",
                    _ => "http_error",
                },
                Some(status),
                retry_after,
                transient,
            );
            if let Some(request_id) = request_id
                && let Some(details) = error.details.as_mut()
            {
                details["cause"]["request_id"] = serde_json::json!(request_id);
            }
            return Err(error);
        }
        return serde_json::from_slice(body).map_err(|_| {
            actor_probe_error("response_decode", "invalid_json", Some(status), None, false)
        });
    }
    if !output.status.success() {
        return Err(no_response_failure());
    }
    serde_json::from_slice(&output.stdout)
        .map_err(|_| actor_probe_error("response_decode", "invalid_json", None, None, false))
}

fn parse_paginated_actor_probe_response(output: &Output) -> Result<serde_json::Value> {
    let mut remaining = output.stdout.as_slice();
    let mut pages = Vec::new();
    while !remaining.is_empty() {
        while matches!(remaining.first(), Some(b'\r' | b'\n' | b' ' | b'\t')) {
            remaining = &remaining[1..];
        }
        if remaining.is_empty() {
            break;
        }
        let (status, retry_after, request_id, body) = parse_included_http_response(remaining);
        let Some(status) = status else {
            return Err(if output.status.success() {
                actor_probe_error("process_exit", "invalid_response", None, None, false)
            } else {
                no_response_failure()
            });
        };
        if !(200..300).contains(&status) {
            let transient = status == 429 || (500..600).contains(&status);
            let mut error = actor_probe_error(
                "http_response",
                match status {
                    401 => "http_401",
                    403 => "http_403",
                    429 => "http_429",
                    500..=599 => "http_5xx",
                    _ => "http_error",
                },
                Some(status),
                retry_after,
                transient,
            );
            if let Some(request_id) = request_id
                && let Some(details) = error.details.as_mut()
            {
                details["cause"]["request_id"] = serde_json::json!(request_id);
            }
            return Err(error);
        }
        let header_len = remaining.len() - body.len();
        let mut stream =
            serde_json::Deserializer::from_slice(body).into_iter::<serde_json::Value>();
        let page = stream.next().and_then(|page| page.ok()).ok_or_else(|| {
            actor_probe_error("response_decode", "invalid_json", Some(status), None, false)
        })?;
        let consumed = stream.byte_offset();
        if consumed == 0 || consumed > body.len() {
            return Err(actor_probe_error(
                "response_decode",
                "invalid_json",
                Some(status),
                None,
                false,
            ));
        }
        pages.push(page);
        remaining = &remaining[header_len + consumed..];
    }
    if !output.status.success() {
        return Err(no_response_failure());
    }
    Ok(serde_json::Value::Array(pages))
}

fn parse_included_http_response(bytes: &[u8]) -> (Option<u16>, Option<u64>, Option<String>, &[u8]) {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return (None, None, None, bytes);
    };
    let split = text
        .find("\r\n\r\n")
        .map(|index| (index, 4))
        .or_else(|| text.find("\n\n").map(|index| (index, 2)));
    let Some((header_end, separator_len)) = split else {
        return (None, None, None, bytes);
    };
    let headers = &text[..header_end];
    let body = &text[header_end + separator_len..];
    let mut lines = headers.lines();
    let status = lines
        .next()
        .and_then(|line| line.split_ascii_whitespace().nth(1))
        .and_then(|value| value.parse::<u16>().ok());
    let mut retry_after = None;
    let mut request_id = None;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if name.eq_ignore_ascii_case("retry-after") {
            retry_after = value.trim().parse::<u64>().ok().filter(|v| *v <= 30);
        } else if name.eq_ignore_ascii_case("x-github-request-id") {
            let value = value.trim();
            if (1..=64).contains(&value.len())
                && value
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b':'))
            {
                request_id = Some(value.to_owned());
            }
        }
    }
    (status, retry_after, request_id, body.as_bytes())
}

fn actor_probe_with_retry(
    mut run: impl FnMut() -> Result<serde_json::Value>,
) -> Result<serde_json::Value> {
    for attempt in 1..=ACTOR_PROBE_MAX_ATTEMPTS {
        match run() {
            Ok(value) => return Ok(value),
            Err(mut error) => {
                if let Some(details) = error.details.as_mut()
                    && let Some(cause) = details.get_mut("cause")
                {
                    cause["attempt"] = serde_json::json!(attempt);
                }
                let retryable = error
                    .details
                    .as_ref()
                    .and_then(|details| details.get("retryable"))
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false);
                if !retryable || attempt == ACTOR_PROBE_MAX_ATTEMPTS {
                    return Err(error);
                }
                let retry_after = error
                    .details
                    .as_ref()
                    .and_then(|details| details.pointer("/cause/retry_after_seconds"))
                    .and_then(serde_json::Value::as_u64);
                let delay = Duration::from_millis(
                    retry_after.map_or(100 * 2u64.pow(attempt.into()), |s| s * 1000),
                );
                if !probe::sleep_within_deadline(delay) {
                    return Err(error);
                }
            }
        }
    }
    unreachable!("bounded attempt range always returns")
}

/// Canonical source checkout for a linked worktree's managed-path rules.
pub fn managed_path(cwd: Option<&Path>) -> Result<PathBuf> {
    let mut cmd = Command::new("git");
    if let Some(cwd) = cwd {
        cmd.current_dir(cwd);
    }
    cmd.args(["rev-parse", "--path-format=absolute", "--git-common-dir"]);
    let output = probe::run(&mut cmd)?;
    if !output.status.success() {
        return Err(Error::new("identity_target_unknown"));
    }
    let common = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
    let path = if common.file_name().is_some_and(|n| n == ".git") {
        common.parent().unwrap().to_path_buf()
    } else {
        common
    };
    fs::canonicalize(path).map_err(|_| Error::new("identity_target_unknown"))
}

/// Explicit API targets may have no checkout. Existing checkout metadata must
/// resolve successfully; an unreadable or damaged checkout is not absence.
pub fn managed_path_optional(cwd: Option<&Path>) -> Result<Option<PathBuf>> {
    let cwd = cwd
        .map(PathBuf::from)
        .map(Ok)
        .unwrap_or_else(std::env::current_dir)
        .map_err(|_| Error::new("identity_target_unknown"))?;
    if std::env::var_os("GIT_DIR").is_some_and(|v| !v.is_empty()) {
        return managed_path(Some(&cwd)).map(Some);
    }
    for dir in cwd.ancestors() {
        match fs::symlink_metadata(dir.join(".git")) {
            Ok(_) => return managed_path(Some(&cwd)).map(Some),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(_) => return Err(Error::new("identity_target_unknown")),
        }
        if dir.join("HEAD").is_file() && dir.join("objects").is_dir() {
            return managed_path(Some(&cwd)).map(Some);
        }
    }
    Ok(None)
}

#[cfg(test)]
mod actor_probe_tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::process::ExitStatusExt;

    fn output(status: i32, body: &[u8]) -> Output {
        Output {
            status: std::process::ExitStatus::from_raw(status << 8),
            stdout: body.to_vec(),
            stderr: b"FIXTURE_PRIVATE_RESPONSE_BODY".to_vec(),
        }
    }

    #[test]
    fn actor_probe_failure_keeps_only_bounded_typed_http_cause() {
        let error = parse_actor_probe_response(&output(
            1,
            b"HTTP/2 503 Service Unavailable\r\nRetry-After: 1\r\nX-GitHub-Request-Id: 8296:105D2D:3F1475:431BEF:6ACEF43B\r\n\r\n{\"message\":\"FIXTURE_PRIVATE_RESPONSE_BODY\"}",
        ))
        .unwrap_err();
        assert_eq!(error.code, "identity_actor_unavailable");
        let details = error.details.unwrap();
        assert_eq!(details["retryable"], true);
        assert_eq!(details["cause"]["stage"], "http_response");
        assert_eq!(details["cause"]["classification"], "http_5xx");
        assert_eq!(details["cause"]["http_status"], 503);
        assert_eq!(details["cause"]["retry_after_seconds"], 1);
        assert_eq!(
            details["cause"]["request_id"],
            "8296:105D2D:3F1475:431BEF:6ACEF43B"
        );
        assert!(serde_json::to_string(&details).unwrap().len() <= 512);
        assert!(
            !serde_json::to_string(&details)
                .unwrap()
                .contains("FIXTURE_PRIVATE_RESPONSE_BODY")
        );
    }

    #[test]
    fn actor_probe_authentication_and_invalid_json_are_not_retryable() {
        let included = parse_actor_probe_response(&output(
            0,
            b"HTTP/2.0 200 OK\nX-GitHub-Request-Id: AbC-123\n\n{\"login\":\"account-a\"}",
        ))
        .unwrap();
        assert_eq!(included["login"], "account-a");

        let unauthorized = parse_actor_probe_response(&output(
            1,
            b"HTTP/2 401 Unauthorized\r\n\r\n{\"message\":\"fixture\"}",
        ))
        .unwrap_err();
        assert_eq!(unauthorized.details.unwrap()["retryable"], false);

        let malformed = parse_actor_probe_response(&output(0, b"not-json")).unwrap_err();
        assert_eq!(
            malformed.details.unwrap()["cause"]["classification"],
            "invalid_json"
        );
    }

    #[test]
    fn actor_probe_failure_without_an_http_response_is_retryable_but_a_malformed_success_is_not() {
        let single = parse_actor_probe_response(&output(1, b"")).unwrap_err();
        let paginated = parse_paginated_actor_probe_response(&output(1, b"")).unwrap_err();
        let late_failure = parse_paginated_actor_probe_response(&output(
            1,
            b"HTTP/2 200 OK\r\n\r\n{\"repositories\":[]}",
        ))
        .unwrap_err();
        for error in [single, paginated, late_failure] {
            let details = error.details.unwrap();
            assert_eq!(details["cause"]["classification"], "nonzero_exit");
            assert_eq!(details["retryable"], true);
            assert_eq!(details["next_action"], "retry_request");
            assert_eq!(details["recovery"]["automatic"], true);
        }

        let invalid = parse_paginated_actor_probe_response(&output(0, b"not-http")).unwrap_err();
        let details = invalid.details.unwrap();
        assert_eq!(details["cause"]["classification"], "invalid_response");
        assert_eq!(details["retryable"], false);

        let mut calls = 0;
        let error = actor_probe_with_retry(|| {
            calls += 1;
            parse_actor_probe_response(&output(1, b""))
        })
        .unwrap_err();
        assert_eq!(calls, 3);
        assert_eq!(error.details.unwrap()["cause"]["attempt"], 3);
    }

    #[test]
    fn paginated_actor_probe_parses_each_included_response_and_retries_transient_status() {
        let success = output(
            0,
            b"HTTP/2 200 OK\r\n\r\n{\"repositories\":[{\"full_name\":\"org/repo\"}]}HTTP/2 200 OK\n\n{\"repositories\":[]}",
        );
        let pages = parse_paginated_actor_probe_response(&success).unwrap();
        assert_eq!(pages.as_array().unwrap().len(), 2);
        assert_eq!(pages[0]["repositories"][0]["full_name"], "org/repo");

        let mut calls = 0;
        let result = actor_probe_with_retry(|| {
            calls += 1;
            if calls == 1 {
                parse_paginated_actor_probe_response(&output(
                    1,
                    b"HTTP/2 503 Service Unavailable\r\nRetry-After: 0\r\n\r\n{}",
                ))
            } else {
                parse_paginated_actor_probe_response(&success)
            }
        })
        .unwrap();
        assert_eq!(calls, 2);
        assert_eq!(result[0]["repositories"][0]["full_name"], "org/repo");

        let mut persistent_calls = 0;
        let persistent = actor_probe_with_retry(|| {
            persistent_calls += 1;
            parse_paginated_actor_probe_response(&output(
                1,
                b"HTTP/2 503 Service Unavailable\r\n\r\n{}",
            ))
        })
        .unwrap_err();
        assert_eq!(persistent_calls, 3);
        let details = persistent.details.unwrap();
        assert_eq!(details["retryable"], true);
        assert_eq!(details["cause"]["http_status"], 503);
        assert_eq!(details["cause"]["attempt"], 3);

        let unauthorized =
            parse_paginated_actor_probe_response(&output(1, b"HTTP/2 403 Forbidden\r\n\r\n{}"))
                .unwrap_err();
        assert_eq!(unauthorized.details.unwrap()["retryable"], false);
    }

    #[test]
    fn paginated_authorization_probe_requires_headers_and_retries_503() {
        let dir = tempfile::tempdir().unwrap();
        let counter = dir.path().join("calls");
        let script = dir.path().join("gh");
        let body = r#"#!/bin/sh
case "$*" in
  *--paginate*)
    case "$*" in *--include*) ;; *) exit 9 ;; esac
    count=$(cat "__COUNTER__" 2>/dev/null || echo 0)
    count=$((count + 1))
    printf '%s' "$count" > "__COUNTER__"
    if [ "$count" -eq 1 ]; then
      printf '%b' 'HTTP/2 503 Service Unavailable\r\nRetry-After: 0\r\n\r\n{}'
      exit 1
    fi
    printf '%b' 'HTTP/2 200 OK\r\n\r\n{"repositories":[{"full_name":"org/repo"}]}'
    ;;
  *) exit 8 ;;
esac
"#
        .replace("__COUNTER__", &counter.display().to_string());
        std::fs::write(&script, body).unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
        let authorization = Authorization {
            selection: Selection {
                principal: "fixture".to_owned(),
                target: Target::new("github.com", "org/repo").unwrap(),
                operation: Operation::ApiRead,
                profile_id: "fixture".to_owned(),
                matched_rule: "fixture".to_owned(),
                profile: Profile {
                    expected_login: Some("account-a".to_owned()),
                    expected_app_id: None,
                    app_slug: None,
                    credential: "fixture".to_owned(),
                    commit_name: "Fixture".to_owned(),
                    commit_email: "fixture@example.invalid".to_owned(),
                    signing_fingerprint: "fixture".to_owned(),
                    operations: vec![Operation::ApiRead],
                },
                session_binding: None,
            },
            actor: "account-a".to_owned(),
            token: "fixture-token".to_owned(),
            policy_digest: "fixture".to_owned(),
        };
        let pages = authorization
            .probe(
                script.as_os_str(),
                &[
                    "api",
                    "installation/repositories?per_page=100",
                    "--paginate",
                ],
            )
            .unwrap();
        assert_eq!(pages[0]["repositories"][0]["full_name"], "org/repo");
        assert_eq!(std::fs::read_to_string(counter).unwrap(), "2");
    }

    #[test]
    fn actor_probe_retries_one_proven_transient_failure_then_succeeds() {
        let mut calls = 0;
        let result = actor_probe_with_retry(|| {
            calls += 1;
            if calls == 1 {
                Err(parse_actor_probe_response(&output(
                    1,
                    b"HTTP/2 503 Service Unavailable\r\n\r\n{}",
                ))
                .unwrap_err())
            } else {
                parse_actor_probe_response(&output(0, b"{\"login\":\"fixture\"}"))
            }
        })
        .unwrap();
        assert_eq!(calls, 2);
        assert_eq!(result["login"], "fixture");

        let mut persistent_calls = 0;
        let error = actor_probe_with_retry(|| {
            persistent_calls += 1;
            parse_actor_probe_response(&output(1, b"HTTP/2 503 Service Unavailable\r\n\r\n{}"))
        })
        .unwrap_err();
        assert_eq!(persistent_calls, 3);
        let details = error.details.unwrap();
        assert_eq!(details["retryable"], true);
        assert_eq!(details["cause"]["attempt"], 3);
    }
}
