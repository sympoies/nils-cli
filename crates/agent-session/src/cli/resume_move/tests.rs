use super::*;
use pretty_assertions::assert_eq;
use std::fs;
use std::path::Path;
use std::process::Command;

const SSH_KEYGEN: &str = "/usr/bin/ssh-keygen";
const NOW: i64 = 1_800_000_000;

struct Fixture {
    directory: tempfile::TempDir,
    verifier: Verifier,
    binding: Binding,
    ticket: Verified<Ticket>,
    proof: StopProof,
}

impl Fixture {
    fn new(provider: Provider) -> Option<Self> {
        if !Path::new(SSH_KEYGEN).is_file() {
            eprintln!("skipped move-proof fixture: ssh-keygen is unavailable");
            return None;
        }
        let directory = tempfile::tempdir().unwrap();
        let mut allowed_signers = String::new();
        for host in ["host-a", "host-b", "untrusted"] {
            let key = directory.path().join(host);
            assert!(
                Command::new(SSH_KEYGEN)
                    .args(["-q", "-t", "ed25519", "-N", "", "-C", "fixture", "-f"])
                    .arg(&key)
                    .status()
                    .unwrap()
                    .success()
            );
            if host != "untrusted" {
                let public = fs::read_to_string(key.with_extension("pub")).unwrap();
                allowed_signers.push_str(&format!("{host} {public}"));
            }
        }
        let allowed_path = directory.path().join("allowed-signers");
        fs::write(&allowed_path, allowed_signers).unwrap();
        let verifier = Verifier::new(Path::new(SSH_KEYGEN), &allowed_path).unwrap();
        let binding = Binding {
            move_id: "00000000-0000-4000-8000-000000000001".into(),
            nonce: "00000000-0000-4000-8000-000000000002".into(),
            source_host: "host-a".into(),
            destination_host: "host-b".into(),
            source_session_id: "source".into(),
            source_incarnation: "00000000-0000-4000-8000-000000000003".into(),
            provider,
            provider_session_id: "00000000-0000-4000-8000-000000000004".into(),
            account: "fixture-account".into(),
            destination_checkout_sha256: sha256(b"destination checkout binding"),
        };
        let raw = sign(
            &directory,
            "host-b",
            TICKET_SCHEMA,
            &Ticket {
                schema_version: TICKET_SCHEMA.into(),
                binding: binding.clone(),
                issued_at: NOW - 10,
                expires_at: NOW + 60,
            },
        );
        let ticket = verifier.verify_ticket(&raw, "host-b").unwrap();
        let history = match provider {
            Provider::Codex => History {
                role: HistoryRole::CodexRollout,
                relative_path: format!(
                    "sessions/2026/01/01/rollout-{}.jsonl",
                    binding.provider_session_id
                ),
                bytes: 42,
                sha256: sha256(b"rollout"),
            },
            Provider::Claude => History {
                role: HistoryRole::ClaudeTranscript,
                relative_path: format!("{}.jsonl", binding.provider_session_id),
                bytes: 32,
                sha256: sha256(b"transcript"),
            },
        };
        let proof = StopProof {
            schema_version: STOP_PROOF_SCHEMA.into(),
            ticket_sha256: ticket.sha256().into(),
            binding: binding.clone(),
            source_generation: 1,
            account_binding_revision: 1,
            source_checkout_sha256: sha256(b"source checkout binding"),
            stopped: true,
            issued_at: NOW - 5,
            expires_at: NOW + 60,
            history,
        };
        Some(Self {
            directory,
            verifier,
            binding,
            ticket,
            proof,
        })
    }

    fn signed_proof(&self, signer: &str) -> SignedRecord {
        sign(&self.directory, signer, STOP_PROOF_SCHEMA, &self.proof)
    }

    fn verify(&self, record: &SignedRecord, observed: Observation<'_>) -> Result<(), Refusal> {
        self.verifier
            .verify_stop_proof(record, &self.binding.source_host)?
            .check(&self.ticket, observed)
    }

    fn observation(&self) -> Observation<'_> {
        Observation {
            binding: &self.binding,
            history: &self.proof.history,
            recorded_source_checkout_sha256: &self.proof.source_checkout_sha256,
            destination_writer_present: false,
            now: NOW,
        }
    }
}

fn sign(
    directory: &tempfile::TempDir,
    signer: &str,
    namespace: &str,
    value: &impl Serialize,
) -> SignedRecord {
    let path = directory.path().join("payload.json");
    let payload = serde_json::to_string(value).unwrap();
    fs::write(&path, &payload).unwrap();
    let output = Command::new(SSH_KEYGEN)
        .env_remove("SSH_AUTH_SOCK")
        .args(["-Y", "sign", "-n", namespace, "-f"])
        .arg(directory.path().join(signer))
        .arg(&path)
        .output()
        .unwrap();
    assert!(output.status.success());
    let signature = fs::read_to_string(path.with_extension("json.sig")).unwrap();
    fs::remove_file(path.with_extension("json.sig")).unwrap();
    SignedRecord { payload, signature }
}

#[test]
fn verified_stop_proof_is_bound_to_the_ticket_and_byte_snapshot() {
    for provider in [Provider::Codex, Provider::Claude] {
        let Some(fixture) = Fixture::new(provider) else {
            return;
        };
        let proof = fixture.signed_proof("host-a");
        assert_eq!(fixture.verify(&proof, fixture.observation()), Ok(()));

        let mut other = fixture.binding.clone();
        other.account = "other-account".into();
        assert_eq!(
            fixture.verify(
                &proof,
                Observation {
                    binding: &other,
                    ..fixture.observation()
                }
            ),
            Err(Refusal::AccountMismatch)
        );

        other = fixture.binding.clone();
        other.provider_session_id = "00000000-0000-4000-8000-000000000099".into();
        assert_eq!(
            fixture.verify(
                &proof,
                Observation {
                    binding: &other,
                    ..fixture.observation()
                }
            ),
            Err(Refusal::SessionMismatch)
        );

        other = fixture.binding.clone();
        other.destination_host = "other-host".into();
        assert_eq!(
            fixture.verify(
                &proof,
                Observation {
                    binding: &other,
                    ..fixture.observation()
                }
            ),
            Err(Refusal::TargetMismatch)
        );

        other = fixture.binding.clone();
        other.destination_checkout_sha256 = sha256(b"other checkout");
        assert_eq!(
            fixture.verify(
                &proof,
                Observation {
                    binding: &other,
                    ..fixture.observation()
                }
            ),
            Err(Refusal::PathMismatch)
        );

        let mut advanced = fixture.proof.history.clone();
        advanced.bytes += 1;
        advanced.sha256 = sha256(b"later content");
        assert_eq!(
            fixture.verify(
                &proof,
                Observation {
                    history: &advanced,
                    ..fixture.observation()
                }
            ),
            Err(Refusal::HistoryStale)
        );
        assert_eq!(
            fixture.verify(
                &proof,
                Observation {
                    now: NOW + 61,
                    ..fixture.observation()
                }
            ),
            Err(Refusal::ProofExpired)
        );
        assert_eq!(
            fixture.verify(
                &proof,
                Observation {
                    destination_writer_present: true,
                    ..fixture.observation()
                }
            ),
            Err(Refusal::DestinationWriterPresent)
        );
    }
}

#[test]
fn forged_modified_or_live_proofs_refuse() {
    let Some(mut fixture) = Fixture::new(Provider::Codex) else {
        return;
    };
    let forged = fixture.signed_proof("untrusted");
    assert_eq!(
        fixture.verify(&forged, fixture.observation()),
        Err(Refusal::ProofInvalid)
    );
    let mut tampered = fixture.signed_proof("host-a");
    tampered.payload = tampered
        .payload
        .replace("fixture-account", "tampered-account");
    assert_eq!(
        fixture.verify(&tampered, fixture.observation()),
        Err(Refusal::ProofInvalid)
    );

    let other_namespace = sign(
        &fixture.directory,
        "host-a",
        NO_WRITER_SCHEMA,
        &fixture.proof,
    );
    assert_eq!(
        fixture.verify(&other_namespace, fixture.observation()),
        Err(Refusal::ProofInvalid)
    );

    fixture.proof.stopped = false;
    let live = fixture.signed_proof("host-a");
    assert_eq!(
        fixture.verify(&live, fixture.observation()),
        Err(Refusal::SourceLive)
    );
}

#[test]
fn signed_history_role_must_match_the_provider() {
    let Some(mut fixture) = Fixture::new(Provider::Claude) else {
        return;
    };
    fixture.proof.history.role = HistoryRole::CodexRollout;
    let wrong_role = fixture.signed_proof("host-a");
    assert_eq!(
        fixture.verify(&wrong_role, fixture.observation()),
        Err(Refusal::HistoryUnsupported)
    );
}

#[test]
fn abort_receipt_is_bound_to_the_exact_authenticated_stop_proof() {
    let Some(fixture) = Fixture::new(Provider::Codex) else {
        return;
    };
    let proof = fixture
        .verifier
        .verify_stop_proof(&fixture.signed_proof("host-a"), "host-a")
        .unwrap();
    let mut receipt = NoWriterReceipt {
        schema_version: NO_WRITER_SCHEMA.into(),
        binding: fixture.binding.clone(),
        stop_proof_sha256: proof.sha256().into(),
        verdict: AbortVerdict::AbortedNoWriter,
    };
    let signed = sign(&fixture.directory, "host-b", NO_WRITER_SCHEMA, &receipt);
    let verified = fixture
        .verifier
        .verify_no_writer_receipt(&signed, "host-b")
        .unwrap();
    assert_eq!(verified.check(&proof), Ok(()));

    receipt.stop_proof_sha256 = sha256(b"another export");
    let signed = sign(&fixture.directory, "host-b", NO_WRITER_SCHEMA, &receipt);
    let verified = fixture
        .verifier
        .verify_no_writer_receipt(&signed, "host-b")
        .unwrap();
    assert_eq!(verified.check(&proof), Err(Refusal::MoveMismatch));
}
