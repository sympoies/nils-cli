//! Bounded host account-broker client shared by provider bindings.
//!
//! A broker is configured as a JSON argv array, never a shell command. Each
//! invocation runs in a fresh process group with a null stdin, bounded
//! stdout/stderr, and a hard deadline. [`BrokerClient`] layers the parts every
//! provider shares on top of that runner: configuration parsing, the stable
//! `<provider>-account-broker-*` error codes, envelope checks, and account-list
//! validation. Codex keeps its v1 protocol (`agent-session.codex-auth-broker.v1`,
//! no `--provider` argument); Claude speaks the provider-neutral v2 protocol
//! (`agent-session.account-broker.v2`). Provider modules own their verbs'
//! response shapes and binding state.

use std::env;
use std::io::Read;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::CliError;

pub(crate) const BROKER_OUTPUT_LIMIT: u64 = 1024 * 1024;
/// Default deadline for one broker call.
pub(crate) const BROKER_TIMEOUT: Duration = Duration::from_secs(10);
/// Longest public account label a broker may return.
pub(crate) const MAX_LABEL_BYTES: usize = 64;
/// Longest public plan name a broker may return.
pub(crate) const MAX_PLAN_BYTES: usize = 128;
const MAX_BROKER_ARGV: usize = 16;
const MAX_BROKER_ARG_BYTES: usize = 4096;

/// Wire protocol a provider's broker speaks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BrokerProtocol {
    /// `agent-session.codex-auth-broker.v1`: no `--provider` argument, and
    /// responses carry no `provider` field.
    CodexV1,
    /// `agent-session.account-broker.v2`: every call passes `--provider <id>`
    /// after the verb, and every response echoes the same `provider`.
    ProviderV2,
}

/// One provider's broker: where it is configured, which protocol it speaks,
/// and the prefix of its stable error codes.
pub(crate) struct BrokerClient {
    /// Provider id, also the error-code prefix (`<provider>-account-...`).
    pub(crate) provider: &'static str,
    /// Provider name used in error messages.
    pub(crate) display: &'static str,
    /// Environment variable holding the JSON argv array.
    pub(crate) env: &'static str,
    /// Required `schema_version` of every response.
    pub(crate) schema: &'static str,
    pub(crate) protocol: BrokerProtocol,
    /// Error returned when no broker is configured.
    pub(crate) unsupported: fn() -> CliError,
}

/// A public, token-free account entry from a broker list.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub(crate) struct AccountSummary {
    pub(crate) account: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) plan: Option<String>,
}

/// A validated broker account list.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct AccountInventory {
    pub(crate) accounts: Vec<AccountSummary>,
    pub(crate) selection_strategies: Vec<String>,
}

/// The `list` response. `A` is the provider's wire shape of one account.
#[derive(Deserialize)]
pub(crate) struct ListResponse<A> {
    schema_version: String,
    #[serde(default)]
    provider: Option<String>,
    accounts: Vec<A>,
    #[serde(default)]
    pub(crate) selection_strategies: Vec<String>,
}

impl BrokerClient {
    /// The configured argv, `None` when unset or blank.
    pub(crate) fn argv(&self) -> Result<Option<Vec<String>>, CliError> {
        parse_argv(env::var(self.env).ok()).map_err(|error| match error {
            BrokerArgvError::NotJsonArgv => self.error(
                "invalid-config",
                "configuration must be a JSON argv array".to_string(),
            ),
            BrokerArgvError::Invalid => {
                self.error("invalid-config", "configuration is invalid".to_string())
            }
        })
    }

    pub(crate) fn is_configured(&self) -> bool {
        matches!(self.argv(), Ok(Some(_)))
    }

    /// Runs `<verb> [--provider <id>] <args...> --format json` and decodes its
    /// stdout as one JSON document.
    pub(crate) fn call(
        &self,
        verb: &str,
        args: &[&str],
        timeout: Duration,
    ) -> Result<Value, CliError> {
        let argv = self.argv()?.ok_or_else(self.unsupported)?;
        let mut call = vec![verb];
        if self.protocol == BrokerProtocol::ProviderV2 {
            call.extend(["--provider", self.provider]);
        }
        call.extend_from_slice(args);
        call.extend(["--format", "json"]);
        run(&argv, &call, timeout).map_err(|error| match error {
            BrokerProcessError::SpawnFailed => {
                self.error("unavailable", "could not be started".to_string())
            }
            BrokerProcessError::StdoutUnavailable => {
                self.error("unavailable", "output was unavailable".to_string())
            }
            BrokerProcessError::StderrUnavailable => {
                self.error("unavailable", "error output was unavailable".to_string())
            }
            BrokerProcessError::WaitFailed => self.error("failed", "failed".to_string()),
            BrokerProcessError::Rejected => {
                self.error("rejected", "rejected the request".to_string())
            }
            BrokerProcessError::OutputTooLarge => self.error(
                "invalid-response",
                "output exceeded the size limit".to_string(),
            ),
            BrokerProcessError::MalformedJson => {
                self.error("invalid-response", "returned malformed JSON".to_string())
            }
            BrokerProcessError::Timeout => self.error("timeout", "timed out".to_string()),
        })
    }

    /// Decodes a verb's response and checks its schema (and, for v2, its
    /// provider). `what` names the payload in the invalid-response message.
    pub(crate) fn decode<T: DeserializeOwned>(
        &self,
        value: Value,
        what: &str,
        envelope: impl FnOnce(&T) -> (&str, Option<&str>),
    ) -> Result<T, CliError> {
        let response: T = serde_json::from_value(value).map_err(|_| self.invalid_response(what))?;
        let (schema, provider) = envelope(&response);
        self.ensure_envelope(schema, provider)?;
        Ok(response)
    }

    fn ensure_envelope(&self, schema: &str, provider: Option<&str>) -> Result<(), CliError> {
        match self.protocol {
            BrokerProtocol::CodexV1 if schema == self.schema => Ok(()),
            BrokerProtocol::CodexV1 => Err(self.invalid_response("an unsupported schema")),
            BrokerProtocol::ProviderV2
                if schema == self.schema && provider == Some(self.provider) =>
            {
                Ok(())
            }
            BrokerProtocol::ProviderV2 => {
                Err(self.invalid_response("an unsupported schema or provider"))
            }
        }
    }

    /// Lists accounts and checks the envelope without validating entries.
    pub(crate) fn list_response<A: DeserializeOwned>(
        &self,
        timeout: Duration,
    ) -> Result<ListResponse<A>, CliError> {
        let value = self.call("list", &[], timeout)?;
        self.decode(
            value,
            "an invalid account list",
            |response: &ListResponse<A>| {
                (
                    response.schema_version.as_str(),
                    response.provider.as_deref(),
                )
            },
        )
    }

    /// Lists accounts and validates every entry: a safe, unique nickname and
    /// bounded single-line public metadata. Blank metadata is dropped. Any
    /// violation is a broker fault (`<provider>-account-broker-invalid-response`).
    pub(crate) fn list<A>(&self, timeout: Duration) -> Result<AccountInventory, CliError>
    where
        A: DeserializeOwned + Into<AccountSummary>,
    {
        let response = self.list_response::<A>(timeout)?;
        let mut accounts: Vec<AccountSummary> = Vec::with_capacity(response.accounts.len());
        for account in response.accounts {
            let mut account: AccountSummary = account.into();
            self.ensure_broker_nickname(&account.account, "an unsafe account nickname")?;
            self.ensure_public_string(&account.label, MAX_LABEL_BYTES)?;
            self.ensure_public_string(&account.plan, MAX_PLAN_BYTES)?;
            if accounts.iter().any(|seen| seen.account == account.account) {
                return Err(self.invalid_response("duplicate account nicknames"));
            }
            account.label = account.label.filter(|value| !value.trim().is_empty());
            account.plan = account.plan.filter(|value| !value.trim().is_empty());
            accounts.push(account);
        }
        Ok(AccountInventory {
            accounts,
            selection_strategies: response.selection_strategies,
        })
    }

    /// A nickname returned by the broker must follow the shared nickname rule.
    pub(crate) fn ensure_broker_nickname(&self, account: &str, what: &str) -> Result<(), CliError> {
        if nils_common::provider_runtime::accounts::is_valid_account_nickname(account) {
            Ok(())
        } else {
            Err(self.invalid_response(what))
        }
    }

    /// Public broker metadata must be bounded and single-line.
    pub(crate) fn ensure_public_string(
        &self,
        value: &Option<String>,
        max: usize,
    ) -> Result<(), CliError> {
        if value
            .as_ref()
            .is_some_and(|value| value.len() > max || value.contains(['\n', '\r', '\0']))
        {
            return Err(self.invalid_response("invalid public metadata"));
        }
        Ok(())
    }

    pub(crate) fn invalid_response(&self, what: &str) -> CliError {
        self.error("invalid-response", format!("returned {what}"))
    }

    fn error(&self, suffix: &str, message: String) -> CliError {
        CliError::runtime(
            format!("{}-account-broker-{suffix}", self.provider),
            format!("{} account broker {message}", self.display),
            None,
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BrokerArgvError {
    NotJsonArgv,
    Invalid,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BrokerProcessError {
    SpawnFailed,
    StdoutUnavailable,
    StderrUnavailable,
    WaitFailed,
    Rejected,
    OutputTooLarge,
    MalformedJson,
    Timeout,
}

/// Parses a broker argv configuration value. Blank values mean "not configured".
pub(crate) fn parse_argv(raw: Option<String>) -> Result<Option<Vec<String>>, BrokerArgvError> {
    let Some(raw) = raw.filter(|value| !value.trim().is_empty()) else {
        return Ok(None);
    };
    let argv: Vec<String> = serde_json::from_str(&raw).map_err(|_| BrokerArgvError::NotJsonArgv)?;
    if !valid_argv(&argv) {
        return Err(BrokerArgvError::Invalid);
    }
    Ok(Some(argv))
}

/// Where serve records its effective broker argv, relative to the state dir.
const SERVE_BROKERS_RECORD: &str = "serve/account-brokers.json";
const SERVE_BROKERS_SCHEMA: &str = "agent-session.serve-account-brokers.v1";
const MAX_SERVE_BROKERS_BYTES: u64 = 64 * 1024;

fn serve_broker_clients() -> [&'static BrokerClient; 2] {
    [
        &crate::codex_account::BROKER,
        &crate::claude_account::BROKER,
    ]
}

/// Record serve's effective broker argv in the owner-private state dir, or
/// retract it when serve has none. serve resolves brokers from its own
/// environment and `--config`, which owner-run CLI commands do not inherit.
pub(crate) fn publish_serve_brokers(state_dir: &Path) -> std::io::Result<()> {
    let record = state_dir.join(SERVE_BROKERS_RECORD);
    let brokers: serde_json::Map<String, Value> = serve_broker_clients()
        .into_iter()
        .filter_map(|client| {
            let argv = client.argv().ok().flatten()?;
            Some((client.env.to_string(), Value::from(argv)))
        })
        .collect();
    if brokers.is_empty() {
        return match std::fs::remove_file(&record) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error),
            _ => Ok(()),
        };
    }
    let document = serde_json::json!({
        "schema_version": SERVE_BROKERS_SCHEMA,
        "brokers": brokers,
    });
    nils_common::fs::write_atomic(&record, document.to_string().as_bytes(), 0o600)
        .map_err(std::io::Error::other)
}

/// For an owner-run CLI command, adopt the broker argv serve recorded for
/// each provider this process's environment leaves unset. The caller's own
/// environment always wins, and a record that is not a private, owner-owned
/// file with valid argv is ignored, leaving the provider unconfigured.
pub(crate) fn adopt_serve_brokers(state_dir: &Path) {
    let unset: Vec<&BrokerClient> = serve_broker_clients()
        .into_iter()
        .filter(|client| {
            env::var(client.env)
                .ok()
                .is_none_or(|value| value.trim().is_empty())
        })
        .collect();
    if unset.is_empty() {
        return;
    }
    let Some(brokers) = read_serve_brokers(&state_dir.join(SERVE_BROKERS_RECORD)) else {
        return;
    };
    for client in unset {
        let Some(argv) = brokers
            .get(client.env)
            .and_then(|value| serde_json::from_value::<Vec<String>>(value.clone()).ok())
            .filter(|argv| valid_argv(argv))
        else {
            continue;
        };
        let Ok(encoded) = serde_json::to_string(&argv) else {
            continue;
        };
        // SAFETY: CLI commands adopt brokers on the main thread before they
        // start any helper thread, so nothing reads the environment concurrently.
        unsafe { env::set_var(client.env, encoded) };
    }
}

fn read_serve_brokers(path: &Path) -> Option<serde_json::Map<String, Value>> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .ok()?;
    let metadata = file.metadata().ok()?;
    if !metadata.is_file()
        || metadata.len() > MAX_SERVE_BROKERS_BYTES
        || metadata.mode() & 0o077 != 0
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.nlink() != 1
    {
        return None;
    }
    let mut bytes = Vec::new();
    file.by_ref()
        .take(MAX_SERVE_BROKERS_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    let document: Value = serde_json::from_slice(&bytes).ok()?;
    if document.get("schema_version").and_then(Value::as_str) != Some(SERVE_BROKERS_SCHEMA) {
        return None;
    }
    document.get("brokers")?.as_object().cloned()
}

/// Count and per-argument bounds shared by every broker configuration source
/// (the environment variables and the `serve --config` broker tables).
pub(crate) fn valid_argv(argv: &[String]) -> bool {
    !argv.is_empty()
        && argv.len() <= MAX_BROKER_ARGV
        && argv
            .iter()
            .all(|arg| !arg.is_empty() && arg.len() <= MAX_BROKER_ARG_BYTES && !arg.contains('\0'))
}

/// Runs `argv + args` and decodes its stdout as one JSON document.
pub(crate) fn run(
    argv: &[String],
    args: &[&str],
    timeout: Duration,
) -> Result<Value, BrokerProcessError> {
    let mut child = Command::new(&argv[0])
        .args(&argv[1..])
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn()
        .map_err(|_| BrokerProcessError::SpawnFailed)?;
    let mut stdout_pipe = child
        .stdout
        .take()
        .ok_or(BrokerProcessError::StdoutUnavailable)?;
    let mut stderr_pipe = child
        .stderr
        .take()
        .ok_or(BrokerProcessError::StderrUnavailable)?;
    let (output_tx, output_rx) = std::sync::mpsc::channel();
    let stdout_tx = output_tx.clone();
    thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stdout_pipe
            .by_ref()
            .take(BROKER_OUTPUT_LIMIT + 1)
            .read_to_end(&mut bytes);
        let _ = stdout_tx.send((true, bytes));
    });
    thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stderr_pipe
            .by_ref()
            .take(BROKER_OUTPUT_LIMIT + 1)
            .read_to_end(&mut bytes);
        let _ = output_tx.send((false, bytes));
    });

    let deadline = Instant::now() + timeout;
    let mut status = None;
    let mut stdout = None;
    let mut stderr_drained = false;
    loop {
        if status.is_none() {
            match child.try_wait() {
                Ok(Some(exit)) => status = Some(exit),
                Ok(None) => {}
                Err(_) => {
                    terminate(&mut child);
                    return Err(BrokerProcessError::WaitFailed);
                }
            }
        }
        while let Ok((is_stdout, bytes)) = output_rx.try_recv() {
            if is_stdout {
                stdout = Some(bytes);
            } else {
                stderr_drained = true;
            }
        }
        if let (Some(status), Some(stdout)) = (status.as_ref(), stdout.as_ref())
            && stderr_drained
        {
            if !status.success() {
                terminate(&mut child);
                return Err(BrokerProcessError::Rejected);
            }
            if stdout.len() as u64 > BROKER_OUTPUT_LIMIT {
                return Err(BrokerProcessError::OutputTooLarge);
            }
            let decoded =
                serde_json::from_slice(stdout).map_err(|_| BrokerProcessError::MalformedJson);
            terminate(&mut child);
            return decoded;
        }
        if Instant::now() >= deadline {
            terminate(&mut child);
            return Err(BrokerProcessError::Timeout);
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn terminate(child: &mut std::process::Child) {
    let pid = child.id();
    // SAFETY: the broker is launched as the leader of a fresh process group.
    unsafe {
        libc::kill(-(pid as i32), libc::SIGKILL);
    }
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(test)]
mod tests {
    use super::*;
    use nils_test_support::{EnvGuard, GlobalStateLock};
    use pretty_assertions::assert_eq;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};

    /// Every provider client runs through the same bounded runner.
    fn clients() -> [&'static BrokerClient; 2] {
        [
            &crate::codex_account::BROKER,
            &crate::claude_account::BROKER,
        ]
    }

    fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
        let path = dir.join(name);
        fs::write(&path, body).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        path
    }

    fn configure(lock: &GlobalStateLock, client: &BrokerClient, argv: &[&str]) -> EnvGuard {
        EnvGuard::set(lock, client.env, &serde_json::to_string(argv).unwrap())
    }

    /// A broker that prints its first argument as the response.
    fn echo_broker(dir: &Path) -> String {
        script(dir, "echo-broker", "#!/bin/sh\nprintf '%s\\n' \"$1\"\n")
            .to_string_lossy()
            .into_owned()
    }

    fn code(client: &BrokerClient, suffix: &str) -> String {
        format!("{}-account-broker-{suffix}", client.provider)
    }

    /// A list response in `client`'s own envelope.
    fn list_json(client: &BrokerClient, accounts: Value) -> String {
        let mut value = serde_json::json!({
            "schema_version": client.schema,
            "accounts": accounts,
        });
        if client.protocol == BrokerProtocol::ProviderV2 {
            value["provider"] = Value::from(client.provider);
        }
        value.to_string()
    }

    #[test]
    fn missing_or_malformed_configuration_fails_closed() {
        let lock = GlobalStateLock::new();
        for client in clients() {
            let unset = EnvGuard::remove(&lock, client.env);
            assert!(!client.is_configured());
            assert_eq!(
                client.call("list", &[], BROKER_TIMEOUT).unwrap_err().code(),
                format!("{}-account-unsupported", client.provider)
            );
            drop(unset);
            let _broker = EnvGuard::set(&lock, client.env, "not-json");
            assert_eq!(
                client.argv().unwrap_err().code(),
                code(client, "invalid-config")
            );
        }
    }

    #[test]
    fn process_failures_map_to_each_providers_stable_codes() {
        let lock = GlobalStateLock::new();
        let tmp = tempfile::TempDir::new().unwrap();
        let broker = script(
            tmp.path(),
            "failing-broker",
            r#"#!/bin/sh
case "$1" in
  malformed) printf '{' ;;
  oversized) dd if=/dev/zero bs=1048577 count=1 2>/dev/null | tr '\000' x ;;
  rejected) printf '%s\n' 'private broker failure' >&2; exit 7 ;;
  *) exit 2 ;;
esac
"#,
        );
        let broker = broker.to_string_lossy();
        for client in clients() {
            for (mode, suffix) in [
                ("malformed", "invalid-response"),
                ("oversized", "invalid-response"),
                ("rejected", "rejected"),
            ] {
                let _broker = configure(&lock, client, &[&broker, mode]);
                let error = client
                    .call("list", &[], Duration::from_secs(2))
                    .unwrap_err();
                assert_eq!(error.code(), code(client, suffix), "{mode}");
                assert!(
                    !error.message().contains("private broker failure"),
                    "broker stderr must stay private"
                );
            }
            let _missing = configure(&lock, client, &["/nonexistent/account-broker"]);
            assert_eq!(
                client.call("list", &[], BROKER_TIMEOUT).unwrap_err().code(),
                code(client, "unavailable")
            );
        }
    }

    #[test]
    fn timeout_terminates_the_broker_process_group() {
        let lock = GlobalStateLock::new();
        let tmp = tempfile::TempDir::new().unwrap();
        let broker = script(
            tmp.path(),
            "hanging-broker",
            r#"#!/bin/sh
sleep 60 &
child=$!
printf '%s\n' "$child" > "$1"
wait "$child"
"#,
        );
        for client in clients() {
            let child_pid_file = tmp.path().join(format!("{}-child-pid", client.provider));
            let _broker = configure(
                &lock,
                client,
                &[&broker.to_string_lossy(), &child_pid_file.to_string_lossy()],
            );
            // The budget must cover the helper's startup phase (spawn the
            // child, publish its PID) with margin, so the timeout fires while
            // the helper is still hanging instead of before it published the
            // child (sympoies/nils-cli#2131).
            let error = client
                .call("list", &[], Duration::from_secs(2))
                .unwrap_err();
            assert_eq!(error.code(), code(client, "timeout"));
            // The group kill happened at the timeout, so wait deterministically
            // for the published child PID instead of racy unwrapping a read.
            let deadline = Instant::now() + Duration::from_secs(2);
            let raw = loop {
                if let Ok(raw) = fs::read_to_string(&child_pid_file)
                    && raw.trim().parse::<i32>().is_ok()
                {
                    break raw;
                }
                assert!(
                    Instant::now() < deadline,
                    "{}: broker never published the child PID file",
                    client.provider
                );
                thread::sleep(Duration::from_millis(10));
            };
            let child_pid: i32 = raw.trim().parse().unwrap();
            let deadline = Instant::now() + Duration::from_secs(1);
            // SAFETY: signal 0 only probes whether the pid still exists.
            while unsafe { libc::kill(child_pid, 0) } == 0 && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(10));
            }
            assert_eq!(
                unsafe { libc::kill(child_pid, 0) },
                -1,
                "{}",
                client.provider
            );
        }
    }

    #[test]
    fn only_the_v2_protocol_passes_the_provider_argument() {
        let lock = GlobalStateLock::new();
        let tmp = tempfile::TempDir::new().unwrap();
        let calls = tmp.path().join("calls");
        let broker = script(
            tmp.path(),
            "recording-broker",
            r#"#!/bin/sh
calls=$1
shift
printf '%s\n' "$*" >> "$calls"
printf '%s\n' '{}'
"#,
        );
        for client in clients() {
            let _broker = configure(
                &lock,
                client,
                &[&broker.to_string_lossy(), &calls.to_string_lossy()],
            );
            client
                .call("select", &["--strategy", "current_default"], BROKER_TIMEOUT)
                .unwrap();
        }
        assert_eq!(
            fs::read_to_string(calls)
                .unwrap()
                .lines()
                .collect::<Vec<_>>(),
            vec![
                "select --strategy current_default --format json",
                "select --provider claude --strategy current_default --format json",
            ]
        );
    }

    /// Owner-run CLI commands do not inherit serve's environment, so they adopt
    /// the broker argv serve recorded in the private state dir, but only for a
    /// provider their own environment leaves unset (sympoies/nils-cli#2040).
    #[test]
    fn cli_adopts_the_serve_recorded_broker_only_where_its_environment_is_unset() {
        let lock = GlobalStateLock::new();
        let tmp = tempfile::TempDir::new().unwrap();
        let state = tmp.path().join("state");
        let claude_argv = r#"["/opt/brokers/claude-broker","--host","sym"]"#;
        let codex_argv = r#"["/opt/brokers/codex-broker"]"#;
        let record = state.join("serve/account-brokers.json");
        {
            let _claude = EnvGuard::set(&lock, "AGENT_SESSION_CLAUDE_ACCOUNT_BROKER", claude_argv);
            let _codex = EnvGuard::set(&lock, "AGENT_SESSION_CODEX_ACCOUNT_BROKER", codex_argv);
            publish_serve_brokers(&state).unwrap();
        }
        assert_eq!(
            fs::metadata(&record).unwrap().permissions().mode() & 0o777,
            0o600
        );

        {
            let _claude = EnvGuard::remove(&lock, "AGENT_SESSION_CLAUDE_ACCOUNT_BROKER");
            let _codex = EnvGuard::set(&lock, "AGENT_SESSION_CODEX_ACCOUNT_BROKER", r#"["/own"]"#);
            adopt_serve_brokers(&state);
            assert_eq!(
                crate::claude_account::BROKER.argv().unwrap(),
                Some(vec![
                    "/opt/brokers/claude-broker".to_string(),
                    "--host".to_string(),
                    "sym".to_string()
                ])
            );
            assert_eq!(
                crate::codex_account::BROKER.argv().unwrap(),
                Some(vec!["/own".to_string()]),
                "the caller's own broker always wins"
            );
        }

        // A record anyone else could have written is never adopted.
        fs::set_permissions(&record, fs::Permissions::from_mode(0o620)).unwrap();
        {
            let _claude = EnvGuard::remove(&lock, "AGENT_SESSION_CLAUDE_ACCOUNT_BROKER");
            adopt_serve_brokers(&state);
            assert!(!crate::claude_account::BROKER.is_configured());
        }

        // A serve without brokers retracts the record.
        {
            let _claude = EnvGuard::remove(&lock, "AGENT_SESSION_CLAUDE_ACCOUNT_BROKER");
            let _codex = EnvGuard::remove(&lock, "AGENT_SESSION_CODEX_ACCOUNT_BROKER");
            publish_serve_brokers(&state).unwrap();
            assert!(!record.exists());
        }
    }

    #[test]
    fn envelopes_must_carry_the_clients_schema_and_provider() {
        let lock = GlobalStateLock::new();
        let tmp = tempfile::TempDir::new().unwrap();
        let broker = echo_broker(tmp.path());
        let codex = &crate::codex_account::BROKER;
        let claude = &crate::claude_account::BROKER;
        let v1 = r#"{"schema_version":"agent-session.codex-auth-broker.v1","accounts":[]}"#;
        let v2 = |provider: &str| {
            format!(
                r#"{{"schema_version":"agent-session.account-broker.v2","provider":"{provider}","accounts":[]}}"#
            )
        };
        let v2_without_provider =
            r#"{"schema_version":"agent-session.account-broker.v2","accounts":[]}"#;
        for (client, response, accepted) in [
            (codex, v1.to_string(), true),
            (codex, v2("codex"), false),
            (claude, v2("claude"), true),
            (claude, v2("codex"), false),
            (claude, v2_without_provider.to_string(), false),
            (claude, v1.to_string(), false),
        ] {
            let _broker = configure(&lock, client, &[&broker, &response]);
            let result = client.list::<AccountSummary>(BROKER_TIMEOUT);
            if accepted {
                assert_eq!(result.unwrap().accounts, vec![], "{response}");
            } else {
                assert_eq!(
                    result.unwrap_err().code(),
                    code(client, "invalid-response"),
                    "{}: {response}",
                    client.provider
                );
            }
        }
    }

    #[test]
    fn listed_accounts_are_validated_as_broker_faults() {
        let lock = GlobalStateLock::new();
        let tmp = tempfile::TempDir::new().unwrap();
        let broker = echo_broker(tmp.path());
        for client in clients() {
            let valid = list_json(
                client,
                serde_json::json!([{"account":"alpha","label":" ","plan":"team"}]),
            );
            let _broker = configure(&lock, client, &[&broker, &valid]);
            assert_eq!(
                client
                    .list::<AccountSummary>(BROKER_TIMEOUT)
                    .unwrap()
                    .accounts,
                vec![AccountSummary {
                    account: "alpha".to_string(),
                    label: None,
                    plan: Some("team".to_string()),
                }]
            );
            for accounts in [
                serde_json::json!([{"account":"--format"}]),
                serde_json::json!([{"account":".."}]),
                serde_json::json!([{"account":"alpha"},{"account":"alpha"}]),
                serde_json::json!([{"account":"alpha","label":"a".repeat(MAX_LABEL_BYTES + 1)}]),
                serde_json::json!([{"account":"alpha","plan":"team\nplan"}]),
            ] {
                let response = list_json(client, accounts);
                let _broker = configure(&lock, client, &[&broker, &response]);
                assert_eq!(
                    client
                        .list::<AccountSummary>(BROKER_TIMEOUT)
                        .unwrap_err()
                        .code(),
                    code(client, "invalid-response"),
                    "{response}"
                );
            }
        }
    }
}
