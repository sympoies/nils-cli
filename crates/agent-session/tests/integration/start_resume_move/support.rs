use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use agent_session::cli::resume_move::{STOP_PROOF_SCHEMA, SignedRecord, StopProof};

/// Two synthetic host signers and the allowlist that trusts both.
pub(super) struct Signers {
    ssh_keygen: PathBuf,
    pub(super) a_key: PathBuf,
    pub(super) b_key: PathBuf,
    pub(super) allowlist: PathBuf,
}

impl Signers {
    pub(super) fn new(root: &Path, ssh_keygen: &Path) -> Self {
        let a_key = root.join("host-a");
        let b_key = root.join("host-b");
        for key in [&a_key, &b_key] {
            assert!(
                Command::new(ssh_keygen)
                    .args(["-q", "-t", "ed25519", "-N", "", "-f"])
                    .arg(key)
                    .status()
                    .unwrap()
                    .success()
            );
        }
        let allowlist = root.join("allowed-signers");
        let signers = format!(
            "host-a {}host-b {}",
            fs::read_to_string(a_key.with_extension("pub")).unwrap(),
            fs::read_to_string(b_key.with_extension("pub")).unwrap()
        );
        write_private(&allowlist, signers.as_bytes());
        Self {
            ssh_keygen: ssh_keygen.to_path_buf(),
            a_key,
            b_key,
            allowlist,
        }
    }

    /// Sign the stop proof as the source host and write it as a private record file.
    pub(super) fn sign_proof(&self, root: &Path, proof: &StopProof) -> PathBuf {
        let signed_path = root.join("proof.json");
        let payload_path = root.join("payload");
        let payload = serde_json::to_string(proof).unwrap();
        write_private(&payload_path, payload.as_bytes());
        assert!(
            Command::new(&self.ssh_keygen)
                .args(["-Y", "sign", "-n", STOP_PROOF_SCHEMA, "-f"])
                .arg(&self.a_key)
                .arg(&payload_path)
                .output()
                .unwrap()
                .status
                .success()
        );
        let signature = fs::read_to_string(payload_path.with_extension("sig")).unwrap();
        write_private(
            &signed_path,
            &serde_json::to_vec(&SignedRecord { payload, signature }).unwrap(),
        );
        signed_path
    }
}

pub(super) fn write_private(path: &Path, bytes: &[u8]) {
    fs::write(path, bytes).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
}
