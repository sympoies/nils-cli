use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::sync::Barrier;

use nils_test_support::git::{InitRepoOptions, git, init_repo_at_with};
use pretty_assertions::assert_eq;
use serde_json::json;

use crate::CliContext;
use crate::cli::resume_move::{Binding, SignedRecord, destination, source};

use super::{
    Fixture, History, HistoryRole, Provider, Refusal, STOP_PROOF_SCHEMA, StopProof, TICKET_SCHEMA,
    sha256, sign,
};

/// A destination move prepared with its exact private history; `stage` records the proof.
pub(super) struct Staged {
    pub(super) context: CliContext,
    pub(super) cwd: std::path::PathBuf,
    pub(super) source_cwd: std::path::PathBuf,
    pub(super) binding: Binding,
    pub(super) proof: SignedRecord,
    pub(super) destination_key: std::path::PathBuf,
    pub(super) now: i64,
}

impl Staged {
    pub(super) fn prepare(fixture: &Fixture, state_name: &str) -> Self {
        let root = fs::canonicalize(fixture.directory.path()).unwrap();
        let context = CliContext {
            state_dir: root.join(state_name),
            host: Some("host-b".into()),
        };
        let cwd = root.join("workspace");
        fs::create_dir(&cwd).unwrap();
        init_repo_at_with(&cwd, InitRepoOptions::new().with_initial_commit());
        git(
            &cwd,
            &[
                "remote",
                "add",
                "origin",
                "https://example.invalid/fixture/workspace.git",
            ],
        );
        let source_cwd = root.join("source-layout").join("workspace");
        let checkout = sha256(source_cwd.as_os_str().as_encoded_bytes());
        let original = &fixture.binding;
        let relative_path = format!(
            "sessions/2026/01/01/rollout-{}.jsonl",
            original.provider_session_id
        );
        let rollout = json!({"type":"session_meta","payload":{
            "id":original.provider_session_id,"cwd":source_cwd,"source":"cli",
            "timestamp":"2026-01-01T00:00:00Z","history_mode":"legacy"
        }})
        .to_string();
        let plan = source::Plan {
            source_host: original.source_host.clone(),
            destination_host: original.destination_host.clone(),
            source_session_id: original.source_session_id.clone(),
            source_incarnation: original.source_incarnation.clone(),
            source_generation: 1,
            provider: original.provider,
            provider_session_id: original.provider_session_id.clone(),
            account: original.account.clone(),
            account_binding_revision: 1,
            source_checkout_sha256: checkout.clone(),
            git_checkout: source::git_checkout(&cwd),
            history: source::HistoryPlacement {
                role: HistoryRole::CodexRollout,
                source_relative_path: relative_path.clone(),
                relative_path: relative_path.clone(),
                bytes: rollout.len() as u64,
            },
        };
        let now = jiff::Timestamp::now().as_second();
        let destination_key = root.join("host-b");
        let verified_prepare = destination::prepare_fixture(
            &context,
            &plan,
            &cwd,
            &original.move_id,
            &fixture.verifier,
            &destination_key,
            now,
        )
        .unwrap();
        let ticket = fixture
            .verifier
            .verify_ticket(&verified_prepare.ticket, "host-b")
            .unwrap();
        assert_eq!(ticket.record().schema_version, TICKET_SCHEMA);
        let binding = ticket.record().binding.clone();
        let history = verified_prepare.provider_home.join(&relative_path);
        fs::create_dir_all(history.parent().unwrap()).unwrap();
        fs::write(&history, &rollout).unwrap();
        fs::set_permissions(&history, fs::Permissions::from_mode(0o600)).unwrap();
        let proof = sign(
            &fixture.directory,
            "host-a",
            STOP_PROOF_SCHEMA,
            &StopProof {
                schema_version: STOP_PROOF_SCHEMA.into(),
                ticket_sha256: ticket.sha256().into(),
                binding: binding.clone(),
                source_generation: 1,
                account_binding_revision: 1,
                source_checkout_sha256: checkout,
                stopped: true,
                issued_at: now,
                expires_at: ticket.record().expires_at,
                history: History {
                    role: HistoryRole::CodexRollout,
                    relative_path,
                    bytes: rollout.len() as u64,
                    sha256: sha256(rollout.as_bytes()),
                },
            },
        );
        Self {
            context,
            cwd,
            source_cwd,
            binding,
            proof,
            destination_key,
            now,
        }
    }

    pub(super) fn stage(&self, fixture: &Fixture) {
        destination::stage(
            &self.context,
            &self.binding.move_id,
            &self.proof,
            &fixture.verifier,
            &self.binding.destination_host,
            self.now,
        )
        .unwrap();
    }

    pub(super) fn request(&self) -> destination::AdmissionRequest<'_> {
        destination::AdmissionRequest {
            move_id: &self.binding.move_id,
            account: &self.binding.account,
            provider_session_id: &self.binding.provider_session_id,
            cwd: &self.cwd,
            destination_host: &self.binding.destination_host,
        }
    }

    pub(super) fn writer_path(&self) -> std::path::PathBuf {
        self.context.state_dir.join("coordination").join(format!(
            "provider-resume-{}.lock",
            sha256(format!("codex:{}", self.binding.provider_session_id).as_bytes())
        ))
    }
}

#[test]
fn destination_abort_and_writer_admission_have_one_winner() {
    let Some(fixture) = Fixture::new(Provider::Codex) else {
        return;
    };
    let staged = Staged::prepare(&fixture, "destination");
    staged.stage(&fixture);
    let Staged {
        context,
        cwd: _,
        source_cwd,
        binding,
        proof,
        destination_key,
        now,
    } = &staged;
    let (context, now) = (context, *now);
    let writer_path = staged.writer_path();
    let request = staged.request();
    destination::check(context, &fixture.verifier, request, now).unwrap();
    assert!(!source_cwd.exists());
    assert!(!writer_path.exists());
    assert!(!context.state_dir.join("sessions").exists());

    let barrier = Barrier::new(2);
    let (admitted, aborted) = std::thread::scope(|scope| {
        let abort = scope.spawn(|| {
            barrier.wait();
            destination::abort(
                context,
                &binding.move_id,
                &fixture.verifier,
                destination_key,
                &binding.destination_host,
                None,
            )
        });
        barrier.wait();
        let admission = destination::admit(context, &fixture.verifier, request, now);
        (admission, abort.join().unwrap())
    });
    match (admitted, aborted) {
        (Ok(_), Err(error)) => assert_eq!(error.code(), Refusal::DestinationWriterPresent.code()),
        (Err(error), Ok(receipt)) => {
            assert_eq!(error.code(), "resume-move-aborted");
            assert!(!writer_path.exists());
            assert!(!context.state_dir.join("sessions").exists());
            fixture
                .verifier
                .verify_no_writer_receipt(&receipt, "host-b")
                .unwrap()
                .check(&fixture.verifier.verify_stop_proof(proof, "host-a").unwrap())
                .unwrap();
            assert_eq!(
                destination::admit(context, &fixture.verifier, request, now)
                    .err()
                    .unwrap()
                    .code(),
                "resume-move-aborted"
            );
        }
        _ => panic!("admission and terminal no-writer abort must have exactly one winner"),
    }
}
