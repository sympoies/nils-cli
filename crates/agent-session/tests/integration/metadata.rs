use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
use std::path::Path;
use std::sync::{Arc, Barrier};
use std::thread;

use nils_test_support::cmd::{CmdOptions, CmdOutput, run_resolved};
use pretty_assertions::assert_eq;
use serde_json::{Value, json};

/// Render each concurrent worker's outcome for an assertion message.
///
/// These two tests race real CLI invocations, and their assertions used to
/// throw away the only thing that explains a failure: which worker failed and
/// what it reported. That matters more here than in an ordinary test, because
/// the failures show up on the macOS lane and do not reproduce on Linux, so the
/// assertion text is usually the only evidence anyone gets.
fn describe_workers<'a>(outputs: impl IntoIterator<Item = &'a CmdOutput>) -> String {
    outputs
        .into_iter()
        .enumerate()
        .map(|(index, output)| {
            let body = serde_json::from_slice::<Value>(&output.stdout).ok();
            let field = |name: &str| {
                body.as_ref()
                    .and_then(|body| body["error"][name].as_str().map(str::to_owned))
                    .unwrap_or_else(|| "-".to_string())
            };
            format!(
                "  worker {index}: exit={} error={} message={:?} stderr={:?}",
                output.code,
                field("code"),
                field("message"),
                output.stderr_text().trim()
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn write_private(path: &Path, bytes: &[u8]) {
    fs::write(path, bytes).expect("write private fixture");
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).expect("set private fixture mode");
}

fn seed_session(state_dir: &Path, id: &str) {
    let state_root = state_dir;
    let sessions = state_root.join("sessions");
    let session = sessions.join(id);
    fs::create_dir_all(&session).expect("create session fixture");
    for directory in [state_root, sessions.as_path(), session.as_path()] {
        fs::set_permissions(directory, fs::Permissions::from_mode(0o700))
            .expect("set private directory mode");
    }
    write_private(
        &session.join("session.json"),
        serde_json::to_string_pretty(&json!({
            "schema_version": "agent-session.session.v1",
            "id": id,
            "agent": "codex",
            "mode": "interactive",
            "title": "metadata fixture",
            "cwd": "/tmp",
            "tmux_session": format!("hs-codex-{id}"),
            "prompt_file": "/private/prompt.md",
            "log_file": "/private/run.log",
            "created_at": "2030-01-01T00:00:00Z",
            "updated_at": "2030-01-01T00:00:00Z",
            "runtime": {
                "kind": "tmux",
                "tmux_session": format!("hs-codex-{id}"),
                "generation": 7,
                "started_at": "2030-01-01T00:00:00Z",
                "launch_id": "private-runtime-incarnation",
                "private_runtime_field": "must-stay-private"
            },
            "private_sentinel": "must-stay-private"
        }))
        .expect("render session")
        .as_bytes(),
    );
}

fn run(state_dir: &Path, args: &[&str]) -> nils_test_support::cmd::CmdOutput {
    run_with_env(state_dir, args, &[])
}

fn run_with_env(
    state_dir: &Path,
    args: &[&str],
    envs: &[(&str, &str)],
) -> nils_test_support::cmd::CmdOutput {
    let state = state_dir.to_string_lossy().into_owned();
    let mut command = vec!["--state-dir", state.as_str()];
    command.extend_from_slice(args);
    run_resolved(
        "agent-session",
        &command,
        &CmdOptions::new().with_envs(envs),
    )
}

fn data(value: &Value) -> &Value {
    value.get("data").expect("success data")
}

fn assert_untrusted_request(state_dir: &Path, id: &str, request: &Path) {
    seed_session(state_dir, id);
    let request_arg = request.to_string_lossy().into_owned();
    let output = run(
        state_dir,
        &[
            "metadata",
            "attach",
            id,
            "--request-file",
            &request_arg,
            "--if-revision",
            "0",
            "--idempotency-key",
            "untrusted-request-key",
            "--format",
            "json",
        ],
    );
    assert_eq!(output.code, 64, "stderr={}", output.stderr_text());
    assert_eq!(
        output.stdout_json()["error"]["code"],
        "metadata-request-untrusted"
    );
    let stored: Value = serde_json::from_slice(
        &fs::read(state_dir.join("sessions").join(id).join("session.json"))
            .expect("unchanged session"),
    )
    .expect("session JSON");
    assert!(stored.get("public_metadata").is_none());
}

#[test]
fn metadata_attach_is_revision_fenced_and_exactly_replayable() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let state_dir = tmp.path().join("state");
    let request = tmp.path().join("metadata.json");
    seed_session(&state_dir, "metadata-target");
    write_private(
        &request,
        br#"{"schema_version":"agent-session.metadata-attachment.request.v1","label":"acceptance.synthetic","value":"DSH-METADATA-731"}"#,
    );

    let request_arg = request.to_string_lossy().into_owned();
    let args = [
        "metadata",
        "attach",
        "metadata-target",
        "--request-file",
        request_arg.as_str(),
        "--if-revision",
        "0",
        "--idempotency-key",
        "metadata-acceptance-001",
        "--format",
        "json",
    ];
    let first = run(&state_dir, &args);
    assert_eq!(first.code, 0, "stderr={}", first.stderr_text());
    let first = first.stdout_json();
    assert_eq!(
        first["schema_version"],
        "cli.agent-session.metadata-attach.v1"
    );
    assert_eq!(data(&first)["id"], "metadata-target");
    assert_eq!(data(&first)["revision"], 1);
    assert_eq!(data(&first)["replayed"], false);
    assert_eq!(data(&first)["metadata"]["label"], "acceptance.synthetic");
    assert!(data(&first)["metadata"]["attachment_id"].is_string());
    assert!(data(&first)["metadata"].get("value_digest").is_none());
    assert!(!first.to_string().contains("DSH-METADATA-731"));

    let replay = run(&state_dir, &args);
    assert_eq!(replay.code, 0, "stderr={}", replay.stderr_text());
    let replay = replay.stdout_json();
    assert_eq!(data(&replay)["revision"], 1);
    assert_eq!(data(&replay)["replayed"], true);
    assert_eq!(
        data(&replay)["evidence_digest"],
        data(&first)["evidence_digest"]
    );

    let show = run(
        &state_dir,
        &[
            "metadata",
            "show",
            "metadata-target",
            "--label",
            "acceptance.synthetic",
            "--format",
            "json",
        ],
    );
    assert_eq!(show.code, 0, "stderr={}", show.stderr_text());
    let show = show.stdout_json();
    assert_eq!(show["schema_version"], "cli.agent-session.metadata-show.v1");
    assert_eq!(
        data(&show)["schema_version"],
        "agent-session.public-metadata-view.v1"
    );
    assert_eq!(data(&show)["id"], "metadata-target");
    assert_eq!(data(&show)["revision"], 1);
    assert_eq!(data(&show)["matching_count"], 1);
    assert_eq!(
        data(&show)["attachments"][0]["label"],
        "acceptance.synthetic"
    );
    assert_eq!(
        data(&show)["attachments"][0]["evidence_digest"],
        data(&first)["evidence_digest"]
    );
    assert_eq!(
        data(&show)["attachments"][0]["attachment_id"],
        data(&first)["metadata"]["attachment_id"]
    );
    let rendered = serde_json::to_string(&show).expect("render read-back");
    for private in [
        "must-stay-private",
        "private-runtime-incarnation",
        "/private/prompt.md",
        "/private/run.log",
        "metadata-acceptance-001",
    ] {
        assert!(!rendered.contains(private), "read-back leaked {private}");
    }

    let stored: Value = serde_json::from_slice(
        &fs::read(state_dir.join("sessions/metadata-target/session.json")).expect("stored session"),
    )
    .expect("stored session JSON");
    assert_eq!(stored["private_sentinel"], "must-stay-private");
    assert_eq!(
        stored["runtime"]["launch_id"],
        "private-runtime-incarnation"
    );
    assert_eq!(stored["runtime"]["generation"], 7);
    assert_eq!(
        stored["public_metadata"]["attachments"]
            .as_object()
            .unwrap()
            .len(),
        1
    );
    let stored_text = serde_json::to_string(&stored).expect("stored session text");
    assert!(!stored_text.contains("metadata-acceptance-001"));
    assert!(!stored_text.contains("DSH-METADATA-731"));
    assert_eq!(
        fs::metadata(state_dir.join("sessions/metadata-target/session.json"))
            .expect("stored session metadata")
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}

#[test]
fn metadata_attach_rejects_key_rebinding_and_stale_revisions_without_mutation() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let state_dir = tmp.path().join("state");
    let request = tmp.path().join("metadata.json");
    seed_session(&state_dir, "metadata-conflicts");
    write_private(
        &request,
        br#"{"schema_version":"agent-session.metadata-attachment.request.v1","label":"topic","value":"first-topic"}"#,
    );
    let request_arg = request.to_string_lossy().into_owned();
    let attach = |key: &str, revision: &str| {
        run(
            &state_dir,
            &[
                "metadata",
                "attach",
                "metadata-conflicts",
                "--request-file",
                request_arg.as_str(),
                "--if-revision",
                revision,
                "--idempotency-key",
                key,
                "--format",
                "json",
            ],
        )
    };
    assert_eq!(attach("stable-key", "0").code, 0);

    write_private(
        &request,
        br#"{"schema_version":"agent-session.metadata-attachment.request.v1","label":"acceptance.synthetic","value":"DSH-METADATA-CHANGED"}"#,
    );
    let key_conflict = attach("stable-key", "1");
    assert_eq!(key_conflict.code, 65);
    assert_eq!(
        key_conflict.stdout_json()["error"]["code"],
        "metadata-idempotency-conflict"
    );
    let stale = attach("fresh-key", "0");
    assert_eq!(stale.code, 65);
    let stale = stale.stdout_json();
    assert_eq!(stale["error"]["code"], "metadata-revision-conflict");
    assert_eq!(stale["error"]["details"]["expected_revision"], 0);
    assert_eq!(stale["error"]["details"]["current_revision"], 1);

    let show = run(
        &state_dir,
        &["metadata", "show", "metadata-conflicts", "--format", "json"],
    );
    assert_eq!(show.code, 0);
    let show = show.stdout_json();
    assert_eq!(data(&show)["revision"], 1);
    assert_eq!(data(&show)["matching_count"], 1);
    assert_eq!(data(&show)["attachments"][0]["label"], "topic");
}

#[test]
fn metadata_attach_rejects_unbounded_sensitive_or_executable_requests() {
    let cases = [
        (
            "unknown-field",
            br#"{"schema_version":"agent-session.metadata-attachment.request.v1","label":"topic","value":"okay-topic","opaque":true}"#.to_vec(),
            "metadata-request-invalid",
        ),
        (
            "nested-patch",
            br#"{"schema_version":"agent-session.metadata-attachment.request.v1","metadata":{"label":"topic","value":"okay-topic"}}"#.to_vec(),
            "metadata-request-invalid",
        ),
        (
            "credential-key",
            br#"{"schema_version":"agent-session.metadata-attachment.request.v1","label":"api_key","value":"public"}"#.to_vec(),
            "metadata-label-forbidden",
        ),
        (
            "credential-shaped-label",
            br#"{"schema_version":"agent-session.metadata-attachment.request.v1","label":"glpat-private-looking-value","value":"public-topic"}"#.to_vec(),
            "metadata-label-unsupported",
        ),
        (
            "credential-value",
            br#"{"schema_version":"agent-session.metadata-attachment.request.v1","label":"topic","value":"sk-proj-private-looking-value"}"#.to_vec(),
            "metadata-value-sensitive",
        ),
        (
            "gitlab-credential-value",
            br#"{"schema_version":"agent-session.metadata-attachment.request.v1","label":"topic","value":"glpat-private-looking-value"}"#.to_vec(),
            "metadata-value-sensitive",
        ),
        (
            "stripe-credential-value",
            br#"{"schema_version":"agent-session.metadata-attachment.request.v1","label":"topic","value":"sk_live_private-looking-value"}"#.to_vec(),
            "metadata-value-sensitive",
        ),
        (
            "password-shaped-value",
            br#"{"schema_version":"agent-session.metadata-attachment.request.v1","label":"topic","value":"Summer2026!"}"#.to_vec(),
            "metadata-value-invalid",
        ),
        (
            "execution-field",
            br#"{"schema_version":"agent-session.metadata-attachment.request.v1","command":"rm","label":"topic","value":"okay-topic"}"#.to_vec(),
            "metadata-request-invalid",
        ),
        (
            "path-value",
            br#"{"schema_version":"agent-session.metadata-attachment.request.v1","label":"topic","value":"/etc/passwd"}"#.to_vec(),
            "metadata-value-path-forbidden",
        ),
        (
            "drive-relative-path-value",
            br#"{"schema_version":"agent-session.metadata-attachment.request.v1","label":"topic","value":"C:private.txt"}"#.to_vec(),
            "metadata-value-path-forbidden",
        ),
        (
            "oversized",
            vec![b'x'; 1025],
            "metadata-request-too-large",
        ),
    ];
    for (name, bytes, code) in cases {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let state_dir = tmp.path().join("state");
        let request = tmp.path().join("metadata.json");
        seed_session(&state_dir, "metadata-invalid");
        write_private(&request, &bytes);
        let request_arg = request.to_string_lossy().into_owned();
        let output = run(
            &state_dir,
            &[
                "metadata",
                "attach",
                "metadata-invalid",
                "--request-file",
                request_arg.as_str(),
                "--if-revision",
                "0",
                "--idempotency-key",
                "invalid-request-key",
                "--format",
                "json",
            ],
        );
        assert_eq!(
            output.code,
            64,
            "case={name} stderr={}",
            output.stderr_text()
        );
        if name == "credential-shaped-label" {
            assert!(!output.stdout_text().contains("glpat-private-looking-value"));
        }
        assert_eq!(output.stdout_json()["error"]["code"], code, "case={name}");
        let stored: Value = serde_json::from_slice(
            &fs::read(state_dir.join("sessions/metadata-invalid/session.json"))
                .expect("unchanged session"),
        )
        .expect("session JSON");
        assert!(stored.get("public_metadata").is_none(), "case={name}");
    }
}

#[test]
fn metadata_attach_rejects_untrusted_request_files_without_mutation() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let request_bytes =
        br#"{"schema_version":"agent-session.metadata-attachment.request.v1","label":"topic","value":"public-topic"}"#;

    let symlink_source = tmp.path().join("symlink-source.json");
    let symlink_request = tmp.path().join("symlink-request.json");
    write_private(&symlink_source, request_bytes);
    symlink(&symlink_source, &symlink_request).expect("request symlink");
    assert_untrusted_request(
        &tmp.path().join("symlink-state"),
        "request-symlink",
        &symlink_request,
    );

    let public_request = tmp.path().join("public-request.json");
    write_private(&public_request, request_bytes);
    fs::set_permissions(&public_request, fs::Permissions::from_mode(0o644))
        .expect("set public request mode");
    assert_untrusted_request(
        &tmp.path().join("public-state"),
        "request-public",
        &public_request,
    );

    let hardlink_source = tmp.path().join("hardlink-source.json");
    let hardlink_request = tmp.path().join("hardlink-request.json");
    write_private(&hardlink_source, request_bytes);
    fs::hard_link(&hardlink_source, &hardlink_request).expect("request hardlink");
    assert_untrusted_request(
        &tmp.path().join("hardlink-state"),
        "request-hardlink",
        &hardlink_request,
    );
}

#[test]
fn metadata_exact_replay_survives_later_revision() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let state_dir = tmp.path().join("state");
    let first_request = tmp.path().join("first.json");
    let second_request = tmp.path().join("second.json");
    seed_session(&state_dir, "metadata-late-replay");
    write_private(
        &first_request,
        br#"{"schema_version":"agent-session.metadata-attachment.request.v1","label":"topic","value":"first-topic"}"#,
    );
    write_private(
        &second_request,
        br#"{"schema_version":"agent-session.metadata-attachment.request.v1","label":"acceptance.synthetic","value":"DSH-METADATA-SECOND"}"#,
    );
    let first_arg = first_request.to_string_lossy().into_owned();
    let second_arg = second_request.to_string_lossy().into_owned();
    let first_args = [
        "metadata",
        "attach",
        "metadata-late-replay",
        "--request-file",
        first_arg.as_str(),
        "--if-revision",
        "0",
        "--idempotency-key",
        "late-replay-key",
        "--format",
        "json",
    ];
    let first = run(&state_dir, &first_args);
    assert_eq!(first.code, 0, "stderr={}", first.stderr_text());
    let first = first.stdout_json();
    let second = run(
        &state_dir,
        &[
            "metadata",
            "attach",
            "metadata-late-replay",
            "--request-file",
            second_arg.as_str(),
            "--if-revision",
            "1",
            "--idempotency-key",
            "later-key",
            "--format",
            "json",
        ],
    );
    assert_eq!(second.code, 0, "stderr={}", second.stderr_text());
    assert_eq!(data(&second.stdout_json())["revision"], 2);

    let replay = run(&state_dir, &first_args);
    assert_eq!(replay.code, 0, "stderr={}", replay.stderr_text());
    let replay = replay.stdout_json();
    assert_eq!(data(&replay)["replayed"], true);
    assert_eq!(data(&replay)["revision"], 1);
    assert_eq!(
        data(&replay)["metadata"]["attachment_id"],
        data(&first)["metadata"]["attachment_id"]
    );
    assert_eq!(
        data(&replay)["evidence_digest"],
        data(&first)["evidence_digest"]
    );
    let show = run(
        &state_dir,
        &[
            "metadata",
            "show",
            "metadata-late-replay",
            "--format",
            "json",
        ],
    );
    assert_eq!(show.code, 0);
    assert_eq!(data(&show.stdout_json())["revision"], 2);
    assert_eq!(data(&show.stdout_json())["matching_count"], 2);
}

#[test]
fn metadata_attach_rejects_capacity_overflow_without_mutation() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let state_dir = tmp.path().join("state");
    let request = tmp.path().join("metadata.json");
    seed_session(&state_dir, "metadata-capacity");
    for (revision, label) in [
        "topic",
        "category",
        "status",
        "priority",
        "source",
        "workflow",
        "component",
        "stage",
    ]
    .into_iter()
    .enumerate()
    {
        write_private(
            &request,
            serde_json::to_string(&json!({
                "schema_version": "agent-session.metadata-attachment.request.v1",
                "label": label,
                "value": format!("{label}-public")
            }))
            .expect("request JSON")
            .as_bytes(),
        );
        let request_arg = request.to_string_lossy().into_owned();
        let output = run(
            &state_dir,
            &[
                "metadata",
                "attach",
                "metadata-capacity",
                "--request-file",
                &request_arg,
                "--if-revision",
                &revision.to_string(),
                "--idempotency-key",
                &format!("capacity-key-{revision}"),
                "--format",
                "json",
            ],
        );
        assert_eq!(
            output.code,
            0,
            "label={label} stderr={}",
            output.stderr_text()
        );
    }
    let record_path = state_dir.join("sessions/metadata-capacity/session.json");
    let before = fs::read(&record_path).expect("capacity record before overflow");
    write_private(
        &request,
        br#"{"schema_version":"agent-session.metadata-attachment.request.v1","label":"acceptance.synthetic","value":"DSH-METADATA-OVERFLOW"}"#,
    );
    let request_arg = request.to_string_lossy().into_owned();
    let overflow = run(
        &state_dir,
        &[
            "metadata",
            "attach",
            "metadata-capacity",
            "--request-file",
            &request_arg,
            "--if-revision",
            "8",
            "--idempotency-key",
            "capacity-overflow-key",
            "--format",
            "json",
        ],
    );
    assert_eq!(overflow.code, 65, "stderr={}", overflow.stderr_text());
    assert_eq!(
        overflow.stdout_json()["error"]["code"],
        "metadata-capacity-exceeded"
    );
    assert_eq!(
        fs::read(&record_path).expect("capacity record after overflow"),
        before
    );
}

#[test]
fn metadata_never_persists_or_projects_raw_values() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let state_dir = tmp.path().join("state");
    let request = tmp.path().join("metadata.json");
    let raw_value = "opaque-synthetic-value";
    seed_session(&state_dir, "metadata-private-value");
    write_private(
        &request,
        serde_json::to_string(&json!({
            "schema_version": "agent-session.metadata-attachment.request.v1",
            "label": "topic",
            "value": raw_value
        }))
        .expect("request JSON")
        .as_bytes(),
    );
    let request_arg = request.to_string_lossy().into_owned();
    let attach = run(
        &state_dir,
        &[
            "metadata",
            "attach",
            "metadata-private-value",
            "--request-file",
            &request_arg,
            "--if-revision",
            "0",
            "--idempotency-key",
            "private-value-key",
            "--format",
            "json",
        ],
    );
    assert_eq!(attach.code, 0, "stderr={}", attach.stderr_text());
    let attach_text = attach.stdout_text();
    assert!(!attach_text.contains(raw_value));
    assert!(
        data(&attach.stdout_json())["metadata"]
            .get("value_digest")
            .is_none()
    );

    let show = run(
        &state_dir,
        &[
            "metadata",
            "show",
            "metadata-private-value",
            "--format",
            "json",
        ],
    );
    assert_eq!(show.code, 0, "stderr={}", show.stderr_text());
    assert!(!show.stdout_text().contains(raw_value));
    assert!(
        !String::from_utf8(
            fs::read(state_dir.join("sessions/metadata-private-value/session.json"))
                .expect("stored session"),
        )
        .expect("stored session UTF-8")
        .contains(raw_value)
    );
}

#[test]
fn metadata_operations_reject_symlinked_state_and_untrusted_record_ownership() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let actual = tmp.path().join("actual-state");
    seed_session(&actual, "symlink-state");
    let alias = tmp.path().join("state-alias");
    symlink(&actual, &alias).expect("state root symlink");
    let symlinked = run(
        &alias,
        &["metadata", "show", "symlink-state", "--format", "json"],
    );
    assert_eq!(symlinked.code, 65);
    assert_eq!(
        symlinked.stdout_json()["error"]["code"],
        "session-state-ancestor-untrusted"
    );

    let state_dir = tmp.path().join("record-state");
    seed_session(&state_dir, "untrusted-record");
    let foreign_uid = fs::metadata(state_dir.join("sessions/untrusted-record/session.json"))
        .expect("record metadata")
        .uid()
        .wrapping_add(1)
        .to_string();
    let wrong_owner = run_with_env(
        &state_dir,
        &["metadata", "show", "untrusted-record", "--format", "json"],
        &[("NILS_AGENT_SESSION_TEST_METADATA_RECORD_UID", &foreign_uid)],
    );
    assert_eq!(wrong_owner.code, 65);
    assert_eq!(
        wrong_owner.stdout_json()["error"]["code"],
        "metadata-session-record-untrusted"
    );

    let target = tmp.path().join("outside-record.json");
    fs::copy(
        state_dir.join("sessions/untrusted-record/session.json"),
        &target,
    )
    .expect("copy target");
    fs::remove_file(state_dir.join("sessions/untrusted-record/session.json"))
        .expect("remove record");
    symlink(
        &target,
        state_dir.join("sessions/untrusted-record/session.json"),
    )
    .expect("record symlink");
    let record_symlink = run(
        &state_dir,
        &["metadata", "show", "untrusted-record", "--format", "json"],
    );
    assert_eq!(record_symlink.code, 65);
    assert_eq!(
        record_symlink.stdout_json()["error"]["code"],
        "metadata-session-record-untrusted"
    );
}

#[test]
fn concurrent_exact_replays_serialize_to_one_attachment() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let state_dir = tmp.path().join("state");
    let request = tmp.path().join("metadata.json");
    seed_session(&state_dir, "metadata-concurrent");
    write_private(
        &request,
        br#"{"schema_version":"agent-session.metadata-attachment.request.v1","label":"topic","value":"concurrent-once"}"#,
    );
    let state_dir = Arc::new(state_dir);
    let request = Arc::new(request.to_string_lossy().into_owned());
    let barrier = Arc::new(Barrier::new(2));
    let mut workers = Vec::new();
    for _ in 0..2 {
        let state_dir = Arc::clone(&state_dir);
        let request = Arc::clone(&request);
        let barrier = Arc::clone(&barrier);
        workers.push(thread::spawn(move || {
            barrier.wait();
            run(
                &state_dir,
                &[
                    "metadata",
                    "attach",
                    "metadata-concurrent",
                    "--request-file",
                    request.as_str(),
                    "--if-revision",
                    "0",
                    "--idempotency-key",
                    "concurrent-exact-key",
                    "--format",
                    "json",
                ],
            )
        }));
    }
    let results = workers
        .into_iter()
        .map(|worker| worker.join().expect("join worker"))
        .collect::<Vec<_>>();
    assert!(
        results.iter().all(|result| result.code == 0),
        "both workers must exit 0: the same idempotency key means the loser of \
         the race replays the winner's write instead of failing its revision \
         precondition\n{}",
        describe_workers(&results)
    );
    let replayed = results
        .iter()
        .map(|result| data(&result.stdout_json())["replayed"].as_bool().unwrap())
        .collect::<Vec<_>>();
    assert!(
        replayed.contains(&false) && replayed.contains(&true),
        "exactly one worker must write and the other replay, got replayed={replayed:?}\n{}",
        describe_workers(&results)
    );

    let show = run(
        &state_dir,
        &[
            "metadata",
            "show",
            "metadata-concurrent",
            "--format",
            "json",
        ],
    );
    assert_eq!(show.code, 0);
    let show = show.stdout_json();
    assert_eq!(data(&show)["revision"], 1);
    assert_eq!(data(&show)["matching_count"], 1);
}

#[test]
fn concurrent_distinct_mutations_conflict_then_retry_without_lost_updates() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let state_dir = tmp.path().join("state");
    seed_session(&state_dir, "metadata-distinct");
    let requests = [
        ("topic", "alpha-topic", "distinct-alpha-key"),
        (
            "acceptance.synthetic",
            "DSH-METADATA-BETA",
            "distinct-beta-key",
        ),
    ]
    .map(|(label, value, key)| {
        let path = tmp.path().join(format!("{value}.json"));
        write_private(
            &path,
            serde_json::to_string(&json!({
                "schema_version": "agent-session.metadata-attachment.request.v1",
                "label": label,
                "value": value
            }))
            .expect("request JSON")
            .as_bytes(),
        );
        (path.to_string_lossy().into_owned(), key)
    });
    let state_dir = Arc::new(state_dir);
    let barrier = Arc::new(Barrier::new(2));
    let mut workers = Vec::new();
    for (request, key) in requests.clone() {
        let state_dir = Arc::clone(&state_dir);
        let barrier = Arc::clone(&barrier);
        workers.push(thread::spawn(move || {
            barrier.wait();
            (
                request.clone(),
                key,
                run(
                    &state_dir,
                    &[
                        "metadata",
                        "attach",
                        "metadata-distinct",
                        "--request-file",
                        &request,
                        "--if-revision",
                        "0",
                        "--idempotency-key",
                        key,
                        "--format",
                        "json",
                    ],
                ),
            )
        }));
    }
    let results = workers
        .into_iter()
        .map(|worker| worker.join().expect("join worker"))
        .collect::<Vec<_>>();
    assert_eq!(
        results
            .iter()
            .filter(|(_, _, output)| output.code == 0)
            .count(),
        1,
        "distinct mutations at the same revision must leave exactly one winner\n{}",
        describe_workers(results.iter().map(|(_, _, output)| output))
    );
    let failed = results
        .iter()
        .find(|(_, _, output)| output.code != 0)
        .expect("one stale writer");
    assert_eq!(
        failed.2.stdout_json()["error"]["code"],
        "metadata-revision-conflict",
        "the stale writer must lose on the revision fence, not on storage\n{}",
        describe_workers(results.iter().map(|(_, _, output)| output))
    );
    let retry = run(
        &state_dir,
        &[
            "metadata",
            "attach",
            "metadata-distinct",
            "--request-file",
            &failed.0,
            "--if-revision",
            "1",
            "--idempotency-key",
            failed.1,
            "--format",
            "json",
        ],
    );
    assert_eq!(retry.code, 0, "stderr={}", retry.stderr_text());
    assert_eq!(data(&retry.stdout_json())["revision"], 2);

    let show = run(
        &state_dir,
        &["metadata", "show", "metadata-distinct", "--format", "json"],
    );
    assert_eq!(show.code, 0);
    let show = show.stdout_json();
    assert_eq!(data(&show)["revision"], 2);
    assert_eq!(data(&show)["matching_count"], 2);
    assert_eq!(
        data(&show)["attachments"][0]["label"],
        "acceptance.synthetic"
    );
    assert_eq!(data(&show)["attachments"][1]["label"], "topic");
}

/// The sympoies/nils-cli#1712 repair: a spurious `ENOENT` from the session
/// record lock open is retried, and nothing else about the open changes.
///
/// The defect is a kernel behavior on macOS and cannot be provoked from a
/// test, so these drive `NILS_AGENT_SESSION_TEST_LOCK_OPEN_FAULT`, which fails
/// that many opens with that errno in the child process. What they pin is the
/// retry policy — the boundary, the errno class, and that both lock-open paths
/// carry it — not the race itself. The race stays owned by
/// `concurrent_distinct_mutations_conflict_then_retry_without_lost_updates`,
/// which asserts the stale writer loses on the revision fence and not on
/// storage.
///
/// The budgets are derived from `SESSION_LOCK_OPEN_ATTEMPTS` rather than
/// written out, so the boundary is asserted instead of a range around it.
#[cfg(debug_assertions)]
mod lock_open_retry {
    use super::{CmdOutput, Path, data, run_with_env, seed_session, write_private};
    use agent_session::SESSION_LOCK_OPEN_ATTEMPTS;
    use pretty_assertions::assert_eq;

    const REQUEST: &[u8] = br#"{"schema_version":"agent-session.metadata-attachment.request.v1","label":"acceptance.synthetic","value":"DSH-METADATA-731"}"#;

    fn attach(state_dir: &Path, request: &Path, fault: &str) -> CmdOutput {
        let request_arg = request.to_string_lossy().into_owned();
        run_with_env(
            state_dir,
            &[
                "metadata",
                "attach",
                "metadata-faulted",
                "--request-file",
                request_arg.as_str(),
                "--if-revision",
                "0",
                "--idempotency-key",
                "metadata-lock-fault-001",
                "--format",
                "json",
            ],
            &[("NILS_AGENT_SESSION_TEST_LOCK_OPEN_FAULT", fault)],
        )
    }

    fn fixture(tmp: &Path) -> (std::path::PathBuf, std::path::PathBuf) {
        let state_dir = tmp.join("state");
        let request = tmp.join("metadata.json");
        seed_session(&state_dir, "metadata-faulted");
        write_private(&request, REQUEST);
        (state_dir, request)
    }

    #[test]
    fn a_burst_one_short_of_the_bound_is_retried_rather_than_reported() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let (state_dir, request) = fixture(tmp.path());

        let fault = format!("ENOENT:{}", SESSION_LOCK_OPEN_ATTEMPTS - 1);
        let output = attach(&state_dir, &request, &fault);
        assert_eq!(
            output.code,
            0,
            "a retried ENOENT must not surface: stdout={} stderr={}",
            output.stdout_text(),
            output.stderr_text()
        );
        assert_eq!(data(&output.stdout_json())["revision"], 1);
    }

    #[test]
    fn a_burst_that_reaches_the_bound_is_still_reported() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let (state_dir, request) = fixture(tmp.path());

        // One more fault than attempts: a parent directory that really is gone
        // must still fail, with the same code it failed with before the retry.
        let fault = format!("ENOENT:{SESSION_LOCK_OPEN_ATTEMPTS}");
        let output = attach(&state_dir, &request, &fault);
        assert_eq!(
            output.stdout_json()["error"]["code"],
            "session-record-lock-open-failed"
        );
    }

    #[test]
    fn a_non_enoent_failure_is_not_retried_at_all() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let (state_dir, request) = fixture(tmp.path());

        // Fewer faults than attempts. An implementation that retried every
        // error would spend them and succeed; only ENOENT is retried, so this
        // fails on the first one.
        let fault = format!("EACCES:{}", SESSION_LOCK_OPEN_ATTEMPTS - 1);
        let output = attach(&state_dir, &request, &fault);
        assert_eq!(
            output.stdout_json()["error"]["code"],
            "session-record-lock-open-failed",
            "stdout={}",
            output.stdout_text()
        );
    }

    #[test]
    fn the_bound_keeps_the_documented_worst_case() {
        // Deriving the budgets above from the constant pins the boundary but
        // not the constant, so a larger bound would pass every case here while
        // quietly falsifying the doc comment's promise. This is a lock path:
        // the delay is the part a reader accepts the retry on.
        let worst_case_micros: u64 = (0..SESSION_LOCK_OPEN_ATTEMPTS - 1)
            .map(|attempt| 200u64 << attempt)
            .sum();
        assert!(
            worst_case_micros < 26_000,
            "worst case is {worst_case_micros}us; retry_spurious_lock_enoent documents under 26ms"
        );
    }

    fn delete_with_fault(state_dir: &Path, fault: &str) -> CmdOutput {
        run_with_env(
            state_dir,
            &["delete", "lock-sibling", "--format", "json"],
            &[("NILS_AGENT_SESSION_TEST_LOCK_OPEN_FAULT", fault)],
        )
    }

    #[test]
    fn the_other_lock_open_path_carries_the_same_policy() {
        // `metadata attach` takes its lock through `openat`; everything else —
        // delete, coordination, the mailbox, `mutate_session_record` — opens
        // the same file through `OpenOptions`. The race is between processes,
        // not between call sites, so both have to retry.
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let state_dir = tmp.path().join("state");
        seed_session(&state_dir, "lock-sibling");

        let retried = delete_with_fault(
            &state_dir,
            &format!("ENOENT:{}", SESSION_LOCK_OPEN_ATTEMPTS - 1),
        );
        // The fixture has no live tmux runtime, so delete fails later for its
        // own reasons. What matters is that it got past the lock.
        assert_ne!(
            retried.stdout_json()["error"]["code"],
            "session-record-lock-open-failed",
            "stdout={}",
            retried.stdout_text()
        );

        let exhausted =
            delete_with_fault(&state_dir, &format!("ENOENT:{SESSION_LOCK_OPEN_ATTEMPTS}"));
        assert_eq!(
            exhausted.stdout_json()["error"]["code"],
            "session-record-lock-open-failed",
            "stdout={}",
            exhausted.stdout_text()
        );
    }
}
