//! Optional, metadata-only forge identity policy shared by managed API and Git runners.
//! The launcher supplies the starting principal; authenticated session binding is a separate layer.
mod git;
mod policy;
mod probe;
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

pub const PRINCIPAL_ENV: &str = "FORGE_IDENTITY_PRINCIPAL";
pub const SESSION_ENV: &str = "FORGE_IDENTITY_SESSION";
pub type Result<T> = std::result::Result<T, Error>;
#[derive(Debug, Clone, Copy)]
pub struct Error {
    pub code: &'static str,
}
impl Error {
    pub const fn new(code: &'static str) -> Self {
        Self { code }
    }
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code)
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
    if policy.activation == policy::Activation::AssertedOnly
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
        self.policy.resolve(&principal()?, target, path, op)
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
            .map_err(|e| *e)
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
            // An installation token must list this repository; a public repo GET alone proves no coverage.
            let pages = auth.probe(
                gh,
                &[
                    "api",
                    "installation/repositories?per_page=100",
                    "--paginate",
                    "--slurp",
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
        }
        let record = Record {
            schema_version: "forge.identity.audit.v1",
            timestamp: jiff::Timestamp::now().to_string(),
            policy_version: 1,
            policy_digest: &self.digest,
            principal: principal().ok(),
            session: std::env::var(SESSION_ENV)
                .ok()
                .filter(|s| policy::identifier(s)),
            target,
            operation,
            profile_id: selection.map(|s| s.profile_id.as_str()),
            matched_rule: selection.map(|s| s.matched_rule.as_str()),
            signer: selection
                .filter(|s| s.operation == Operation::Commit)
                .map(|s| s.profile.signing_fingerprint.as_str()),
            actor,
            outcome,
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
            "matched_rule":self.selection.matched_rule,"actor":self.actor,
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
        let mut cmd = Command::new(gh);
        self.apply_api(&mut cmd);
        cmd.args(args)
            .args(["--hostname", &self.selection.target.host]);
        let output = probe::run(&mut cmd).map_err(|e| {
            if e.code == "identity_probe_unavailable" {
                Error::new("identity_actor_unavailable")
            } else {
                e
            }
        })?;
        if !output.status.success() {
            return Err(Error::new("identity_actor_unavailable"));
        }
        serde_json::from_slice(&output.stdout).map_err(|_| Error::new("identity_actor_unavailable"))
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
