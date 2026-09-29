//! macOS login Keychain storage for the active Claude Code login.
//!
//! On macOS Claude Code reads its login from the generic-password item
//! `Claude Code-credentials` (plus `-<sha256(config dir)[..8]>` when
//! `CLAUDE_CONFIG_DIR` is set) before `.credentials.json`, so a projected
//! login must reach the item too. The secret is passed to `security -i` on
//! stdin as hex (`-X`), never on argv.

use nils_common::env as shared_env;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::io::Write;
use std::process::{Command, Stdio};

use super::store::{self, AuthError, AuthResult};

const KEYCHAIN_ENV: &str = "CLAUDE_AUTH_KEYCHAIN";
const SECURITY_BIN_ENV: &str = "CLAUDE_AUTH_SECURITY_BIN";
const SERVICE: &str = "Claude Code-credentials";
const FALLBACK_ACCOUNT: &str = "claude-code-user";
/// `security find-generic-password` exit status for a missing item.
const ITEM_NOT_FOUND: i32 = 44;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Never touch the Keychain.
    Off,
    /// Write when the Keychain is available; report failures without failing.
    Auto,
    /// Fail when the item cannot be written.
    Required,
}

/// Whether this host keeps Claude Code logins in the Keychain.
///
/// `CLAUDE_AUTH_KEYCHAIN=on|off` overrides the macOS default.
pub fn enabled() -> bool {
    match shared_env::env_non_empty(KEYCHAIN_ENV).as_deref() {
        Some("off") => false,
        Some("on") => true,
        _ => cfg!(target_os = "macos"),
    }
}

fn security_bin() -> String {
    shared_env::env_non_empty(SECURITY_BIN_ENV).unwrap_or_else(|| "security".to_string())
}

/// The item name Claude Code uses for the current config dir.
fn service() -> String {
    match shared_env::env_non_empty(store::CONFIG_DIR_ENV) {
        Some(dir) => {
            let digest = Sha256::digest(dir.as_bytes());
            let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
            format!("{SERVICE}-{}", &hex[..8])
        }
        None => SERVICE.to_string(),
    }
}

/// The item account Claude Code uses: `$USER` when it is a safe name.
fn account() -> String {
    shared_env::env_non_empty("USER")
        .filter(|user| {
            user.chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'))
        })
        .unwrap_or_else(|| FALLBACK_ACCOUNT.to_string())
}

/// Read the stored credentials object, or `None` when there is no item.
pub fn read_item() -> AuthResult<Option<Map<String, Value>>> {
    let output = Command::new(security_bin())
        .args([
            "find-generic-password",
            "-a",
            &account(),
            "-s",
            &service(),
            "-w",
        ])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map_err(|err| AuthError::runtime("keychain-unavailable", format!("security: {err}")))?;
    if output.status.code() == Some(ITEM_NOT_FOUND) {
        return Ok(None);
    }
    if !output.status.success() {
        return Err(AuthError::runtime(
            "keychain-unavailable",
            "the login Keychain could not be read",
        ));
    }
    match serde_json::from_slice::<Value>(&output.stdout) {
        Ok(Value::Object(map)) => Ok(Some(map)),
        _ => Err(AuthError::data(
            "keychain-item-invalid",
            "the Claude Code Keychain item is not a JSON object",
        )),
    }
}

fn write_item(value: &Value) -> AuthResult<()> {
    let bytes = serde_json::to_vec(value)
        .map_err(|err| AuthError::runtime("keychain-write-failed", err.to_string()))?;
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    let command = format!(
        "add-generic-password -U -a \"{}\" -s \"{}\" -X {hex}\n",
        account(),
        service()
    );
    let mut child = Command::new(security_bin())
        .arg("-i")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|err| AuthError::runtime("keychain-unavailable", format!("security: {err}")))?;
    let written = child
        .stdin
        .take()
        .map(|mut stdin| stdin.write_all(command.as_bytes()))
        .transpose();
    let status = child
        .wait()
        .map_err(|err| AuthError::runtime("keychain-write-failed", err.to_string()))?;
    if written.is_err() || !status.success() {
        return Err(AuthError::runtime(
            "keychain-write-failed",
            "the login Keychain item could not be written",
        ));
    }
    Ok(())
}

/// Replace `claudeAiOauth` in the Keychain item, keeping its other entries.
///
/// Returns `off`, `written`, or (in [`Mode::Auto`]) `unavailable`.
pub fn project(access: &Map<String, Value>, mode: Mode) -> AuthResult<&'static str> {
    if mode == Mode::Off || !enabled() {
        if mode == Mode::Required {
            return Err(AuthError::runtime(
                "keychain-unavailable",
                "--keychain required needs a Keychain host",
            ));
        }
        return Ok("off");
    }
    let result = read_item().and_then(|item| {
        let mut item = item.unwrap_or_default();
        item.insert("claudeAiOauth".to_string(), Value::Object(access.clone()));
        write_item(&Value::Object(item))
    });
    match (result, mode) {
        (Ok(()), _) => Ok("written"),
        (Err(err), Mode::Required) => Err(err),
        (Err(_), _) => Ok("unavailable"),
    }
}
