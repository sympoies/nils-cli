//! Ed25519 SSHSIG verification against a configured, host-local trust file.

use std::fs;
use std::io::{Read, Seek, Write};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use base64::{Engine, engine::general_purpose::STANDARD};

use super::{MAX_PAYLOAD_BYTES, MAX_SIGNATURE_BYTES, Refusal, SignedRecord};

pub struct Verifier {
    executable: PathBuf,
    allowed_signers: PathBuf,
}

impl Verifier {
    /// Both paths come from local configuration. No trust is taken from a proof.
    pub fn new(executable: &Path, allowed_signers: &Path) -> Result<Self, Refusal> {
        if !executable.is_absolute() {
            return Err(Refusal::SshKeygenUnavailable);
        }
        let executable = fs::canonicalize(executable).map_err(|_| Refusal::SshKeygenUnavailable)?;
        let metadata = fs::metadata(&executable).map_err(|_| Refusal::SshKeygenUnavailable)?;
        if !metadata.is_file() || metadata.permissions().mode() & 0o111 == 0 {
            return Err(Refusal::SshKeygenUnavailable);
        }
        check_allowed_signers(allowed_signers)?;
        let verifier = Self {
            executable,
            allowed_signers: allowed_signers.to_path_buf(),
        };
        verifier.check_capability()?;
        Ok(verifier)
    }

    fn command(&self) -> Command {
        let mut command = Command::new(&self.executable);
        command
            .env_clear()
            .env("LC_ALL", "C")
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        command
    }

    fn check_capability(&self) -> Result<(), Refusal> {
        let diagnostic = || Refusal::SshKeygenUnavailable;
        let output = tempfile::tempfile().map_err(|_| diagnostic())?;
        let stdio = output.try_clone().map_err(|_| diagnostic())?;
        let child = self
            .command()
            .args(["-Y", "check-novalidate"])
            .stdin(Stdio::null())
            .stderr(stdio)
            .spawn()
            .map_err(|_| diagnostic())?;
        wait(child).map_err(|_| diagnostic())?;
        let mut bytes = String::new();
        let mut output = output;
        output.rewind().map_err(|_| diagnostic())?;
        output
            .take(256)
            .read_to_string(&mut bytes)
            .map_err(|_| diagnostic())?;
        // OpenSSH emits the operation-specific argument check only when -Y is supported.
        if !bytes.starts_with("Too few arguments for check-novalidate:") {
            return Err(diagnostic());
        }
        Ok(())
    }

    pub(super) fn verify_signature(
        &self,
        record: &SignedRecord,
        principal: &str,
        namespace: &'static str,
    ) -> Result<(), Refusal> {
        if record.payload.is_empty()
            || record.payload.len() > MAX_PAYLOAD_BYTES
            || record.signature.len() > MAX_SIGNATURE_BYTES
            || !is_ed25519_sshsig(&record.signature)
        {
            return Err(Refusal::ProofInvalid);
        }
        check_allowed_signers(&self.allowed_signers)?;
        let unavailable = |_| Refusal::SshKeygenUnavailable;
        let mut payload = tempfile::tempfile().map_err(unavailable)?;
        payload
            .write_all(record.payload.as_bytes())
            .map_err(unavailable)?;
        payload.rewind().map_err(unavailable)?;
        let mut signature = tempfile::NamedTempFile::new().map_err(unavailable)?;
        signature
            .write_all(record.signature.as_bytes())
            .map_err(unavailable)?;
        let child = self
            .command()
            .arg("-Y")
            .arg("verify")
            .arg("-f")
            .arg(&self.allowed_signers)
            .arg("-I")
            .arg(principal)
            .arg("-n")
            .arg(namespace)
            .arg("-s")
            .arg(signature.path())
            .stdin(payload)
            .spawn()
            .map_err(unavailable)?;
        if !wait(child).map_err(unavailable)?.success() {
            return Err(Refusal::ProofInvalid);
        }
        Ok(())
    }
}

fn check_allowed_signers(path: &Path) -> Result<(), Refusal> {
    if !path.is_absolute() {
        return Err(Refusal::TrustUnavailable);
    }
    let metadata = fs::symlink_metadata(path).map_err(|_| Refusal::TrustUnavailable)?;
    // SAFETY: geteuid takes no arguments and cannot fail.
    let uid = unsafe { libc::geteuid() };
    if !metadata.is_file()
        || (metadata.uid() != 0 && metadata.uid() != uid)
        || metadata.permissions().mode() & 0o022 != 0
        || metadata.len() == 0
        || metadata.len() > 64 * 1024
    {
        return Err(Refusal::TrustUnavailable);
    }
    Ok(())
}

fn is_ed25519_sshsig(signature: &str) -> bool {
    let Some(body) = signature
        .strip_prefix("-----BEGIN SSH SIGNATURE-----\n")
        .and_then(|value| {
            value
                .trim_end_matches('\n')
                .strip_suffix("\n-----END SSH SIGNATURE-----")
        })
    else {
        return false;
    };
    let body = body.replace('\n', "");
    // SSHSIG version 1, followed by the Ed25519 public-key blob header.
    // OpenSSH validates the remainder of the structure and the signature.
    const HEADER: &[u8] =
        b"SSHSIG\x00\x00\x00\x01\x00\x00\x00\x33\x00\x00\x00\x0bssh-ed25519\x00\x00\x00\x20";
    STANDARD
        .decode(body)
        .is_ok_and(|bytes| bytes.starts_with(HEADER))
}

fn wait(mut child: Child) -> std::io::Result<ExitStatus> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let error = match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
                continue;
            }
            Ok(None) => std::io::ErrorKind::TimedOut.into(),
            Err(error) => error,
        };
        let _ = child.kill();
        let _ = child.wait();
        return Err(error);
    }
}
