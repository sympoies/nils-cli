#[path = "tests/origin_tests.rs"]
mod origin_tests;
include!("../serve_claude_completion_tests.rs");
include!("../serve_codex_idle_notification_tests.rs");
include!("../serve_retitle_tests.rs");
#[path = "tests/interrupt_tests.rs"]
mod interrupt_tests;
use axum::body::Body;
use axum::http::Request;
use nils_test_support::{EnvGuard, GlobalStateLock};
use std::fs;
use std::io::Write;
#[cfg(target_os = "linux")]
use std::os::linux::fs::MetadataExt as LinuxMetadataExt;
use std::os::unix::fs::{FileTypeExt, PermissionsExt, symlink};
use std::os::unix::process::CommandExt;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::{Message as ClientMessage, client::IntoClientRequest};
use tower::ServiceExt;
const MACHINE: &str = "test-machine";
const TOKEN: &str = "s3cr3t-token";

/// A Homebrew upgrade without cleanup repoints the `bin/` link at the new
/// release and leaves the old one installed (sympoies/nils-cli#1817).
#[test]
fn serve_binary_identity_detects_a_repointed_invocation_link() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let release = |version: &str| {
        let dir = tmp.path().join("Cellar/nils-cli").join(version).join("bin");
        fs::create_dir_all(&dir).expect("release dir");
        let binary = dir.join("agent-session");
        fs::write(&binary, version).expect("release binary");
        binary
    };
    let old = release("1.0.0");
    let bin = tmp.path().join("bin");
    fs::create_dir_all(&bin).expect("bin dir");
    let link = bin.join("agent-session");
    symlink(&old, &link).expect("bin link");
    let identity = ServeBinaryIdentity::from_invocation(link.clone()).expect("identity");
    assert!(!identity.changed());

    let staged = bin.join("agent-session.new");
    symlink(release("1.0.1"), &staged).expect("staged link");
    fs::rename(&staged, &link).expect("repoint bin link");

    assert!(old.exists());
    assert!(identity.changed());
}

#[tokio::test]
async fn lifecycle_http_refusal_keeps_exact_error_and_records_pre_engine_failure() {
    let tmp = tempfile::tempdir().unwrap();
    let st = state(tmp.path(), Some(TOKEN), PathBuf::from("/unavailable/tmux"));
    let (status, body) = call(
        router(st.clone()),
        post_json("/sessions/missing/resume", Some(TOKEN), json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let cause = crate::load_session_record(&st.context, "missing").unwrap_err();
    assert_eq!(body["error"]["code"], cause.code());
    assert_eq!(body["error"]["message"], cause.message());
    let journal = crate::lifecycle::read(&st.context, "missing", 100).unwrap();
    assert_eq!(journal["records"].as_array().unwrap().len(), 1);
    assert_eq!(journal["records"][0]["caller"]["kind"], "serve");
}

#[tokio::test]
async fn retired_group_routes_are_absent() {
    let tmp = tempfile::TempDir::new().unwrap();
    let st = state(tmp.path(), Some(TOKEN), minimal_tmux(tmp.path()));
    for route in ["group-cleanup", "group-archive"] {
        let uri = format!("/sessions/retired/orchestration/{route}");
        for method in ["GET", "POST"] {
            let request = Request::builder()
                .method(method)
                .uri(&uri)
                .header("authorization", format!("Bearer {TOKEN}"))
                .body(Body::empty())
                .unwrap();
            let response = router(st.clone()).oneshot(request).await.unwrap();
            assert_eq!(response.status(), StatusCode::NOT_FOUND);
        }
    }
}

#[test]
fn attachment_limit_environment_is_bounded() {
    let lock = GlobalStateLock::new();
    let valid = EnvGuard::set(&lock, MAX_ATTACHMENT_BYTES_ENV, "1073741824");
    assert_eq!(resolve_max_attachment_bytes().unwrap(), 1024 * 1024 * 1024);
    drop(valid);

    let zero = EnvGuard::set(&lock, MAX_ATTACHMENT_BYTES_ENV, "0");
    assert!(resolve_max_attachment_bytes().is_err());
    drop(zero);

    let over_max = (MAX_CONFIGURED_ATTACHMENT_BYTES + 1).to_string();
    let _over_max = EnvGuard::set(&lock, MAX_ATTACHMENT_BYTES_ENV, &over_max);
    assert!(resolve_max_attachment_bytes().is_err());
}

#[test]
fn launch_profiles_advertise_only_ready_safe_summaries() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config_dir = tmp.path().join("claude-gpt");
    fs::create_dir(&config_dir).unwrap();
    let launcher = tmp.path().join("claude-gpt-launcher");
    fs::write(&launcher, "#!/usr/bin/env sh\nexit 0\n").unwrap();
    let mut permissions = fs::metadata(&launcher).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&launcher, permissions).unwrap();
    let profiles = AgentLaunchProfiles::from_json(
        &json!([{
            "id": "claude-gpt",
            "label": "Claude GPT",
            "agent": "claude",
            "agent_bin": launcher,
            "provider_config_dir": config_dir,
            "readiness_args": ["--check"],
            "auto_resume_supported": false,
        }])
        .to_string(),
    )
    .unwrap();

    let summaries = profiles.ready_summaries();
    assert_eq!(
        summaries,
        vec![AgentLaunchProfileSummary {
            id: "claude-gpt".to_string(),
            label: "Claude GPT".to_string(),
            agent: "claude".to_string(),
            provider_resume_import_supported: true,
            history_resume_supported: true,
        }]
    );
    assert_eq!(
        serde_json::to_value(&summaries).unwrap(),
        json!([{"id":"claude-gpt","label":"Claude GPT","agent":"claude","provider_resume_import_supported":true,"history_resume_supported":true}])
    );

    fs::write(&launcher, "#!/usr/bin/env sh\nexit 1\n").unwrap();
    assert!(profiles.probe_ready_summaries().is_empty());
}

#[test]
fn launch_profiles_accept_a_bounded_double_ctrl_c_graceful_shutdown_contract() {
    let tmp = tempfile::TempDir::new().unwrap();
    let launcher = executable(
        &tmp.path().join("claude-workbench-launcher"),
        "#!/usr/bin/env sh\nexit 0\n",
    );

    let profiles = AgentLaunchProfiles::from_json(
        &json!([{
            "id": "claude-workbench",
            "label": "Claude",
            "agent": "claude",
            "agent_bin": launcher,
            "graceful_shutdown": "double-ctrl-c",
        }])
        .to_string(),
    )
    .expect("the fixed graceful-shutdown mode is valid server-owned profile metadata");

    assert_eq!(profiles.entries.len(), 1);
}

#[test]
fn launch_profiles_reject_the_retired_hermes_base_agent() {
    let tmp = tempfile::TempDir::new().unwrap();
    let launcher = executable(&tmp.path().join("launcher"), "#!/bin/sh\nexit 0\n");
    let raw = json!([{ "id": "dsh-workbench", "label": "DSH", "agent": "hermes", "agent_bin": launcher }])
            .to_string();
    let error = AgentLaunchProfiles::from_json(&raw).expect_err("hermes is no longer an alias");
    assert_eq!(error.code(), "invalid-agent-launch-profiles");
    assert!(validate_launch_profiles_json(&raw).is_err());
}

#[test]
fn environment_launch_profiles_reject_retired_dsh_profiles() {
    let tmp = tempfile::TempDir::new().unwrap();
    let profiles = json!([{
        "id": "dsh-workbench",
        "label": "DSH",
        "agent": "dsh",
        "agent_bin": tmp.path().join("dsh-launcher")
    }])
    .to_string();
    let lock = GlobalStateLock::new();
    let _profiles = EnvGuard::set(&lock, "AGENT_SESSION_LAUNCH_PROFILES", &profiles);

    let error = AgentLaunchProfiles::from_environment()
        .expect_err("the live environment must not re-enable retired DSH launches");

    assert_eq!(error.code(), "invalid-agent-launch-profiles");
    assert!(error.message().contains("DSH launch profiles are retired"));
}

#[test]
fn launch_profiles_project_a_valid_dsh_history_adapter_source() {
    let tmp = tempfile::TempDir::new().unwrap();
    let launcher = executable(&tmp.path().join("dsh-launcher"), "#!/bin/sh\nexit 0\n");
    let adapter = executable(&tmp.path().join("dsh-history"), "#!/bin/sh\nexit 0\n");
    let sessions = tmp.path().join("sessions");
    fs::create_dir(&sessions).unwrap();
    let profiles = AgentLaunchProfiles::from_json(
        &json!([{
            "id": "dsh-workbench",
            "label": "DSH",
            "agent": "dsh",
            "agent_bin": launcher,
            "dsh_history": {
                "command": adapter,
                "root": sessions,
                "compression": "zstd"
            }
        }])
        .to_string(),
    )
    .unwrap();

    let sources = profiles.dsh_history_sources();
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0].agent_profile, "dsh-workbench");
    assert_eq!(sources[0].compression, "zstd");
}

#[test]
fn launch_profiles_advertise_dsh_history_resume_only_when_exact_id_is_explicit() {
    let tmp = tempfile::TempDir::new().unwrap();
    let launcher = executable(&tmp.path().join("dsh-launcher"), "#!/bin/sh\nexit 0\n");
    let adapter = executable(&tmp.path().join("dsh-history"), "#!/bin/sh\nexit 0\n");
    let sessions = tmp.path().join("sessions");
    fs::create_dir(&sessions).unwrap();

    let without_resume = AgentLaunchProfiles::from_json(
        &json!([{
            "id": "dsh-read-only",
            "label": "DSH read only",
            "agent": "dsh",
            "agent_bin": launcher,
            "dsh_history": {
                "command": adapter,
                "root": sessions
            }
        }])
        .to_string(),
    )
    .unwrap();
    assert!(!without_resume.entries[0].summary().history_resume_supported);

    let exact_resume = AgentLaunchProfiles::from_json(
        &json!([{
            "id": "dsh-exact-resume",
            "label": "DSH exact resume",
            "agent": "dsh",
            "agent_bin": launcher,
            "dsh_history": {
                "command": adapter,
                "root": sessions,
                "resume": "exact-id"
            }
        }])
        .to_string(),
    )
    .unwrap();
    let summary = exact_resume.entries[0].summary();
    assert!(!summary.provider_resume_import_supported);
    assert!(summary.history_resume_supported);
}

#[test]
fn unavailable_dsh_history_does_not_block_the_launch_profile() {
    let tmp = tempfile::TempDir::new().unwrap();
    let launcher = executable(&tmp.path().join("dsh-launcher"), "#!/bin/sh\nexit 0\n");
    let profiles = AgentLaunchProfiles::from_json(
        &json!([{
            "id": "dsh-workbench",
            "label": "DSH",
            "agent": "dsh",
            "agent_bin": launcher,
            "dsh_history": {
                "command": tmp.path().join("missing-history-adapter"),
                "root": tmp.path().join("missing-sessions")
            }
        }])
        .to_string(),
    )
    .unwrap();

    assert!(profiles.entries[0].is_ready());
}

#[test]
fn launch_profiles_reject_unknown_graceful_shutdown_contracts() {
    let tmp = tempfile::TempDir::new().unwrap();
    let launcher = executable(
        &tmp.path().join("claude-workbench-launcher"),
        "#!/usr/bin/env sh\nexit 0\n",
    );

    let error = AgentLaunchProfiles::from_json(
        &json!([{
            "id": "claude-workbench",
            "label": "Claude",
            "agent": "claude",
            "agent_bin": launcher,
            "graceful_shutdown": "force-kill",
        }])
        .to_string(),
    )
    .expect_err("unknown shutdown behavior must fail the server-owned profile parser closed");

    assert_eq!(error.code(), "invalid-agent-launch-profiles");
}

#[test]
fn launch_profile_readiness_timeout_kills_its_process_group() {
    let tmp = tempfile::TempDir::new().unwrap();
    let pid_file = tmp.path().join("readiness.pid");
    let mut command = ProcessCommand::new("/bin/sh");
    command.arg("-c").arg(format!(
        "echo $$ > '{}'; sleep 30 & wait",
        pid_file.display()
    ));

    let started_at = Instant::now();
    assert!(!run_agent_profile_readiness(
        command,
        Duration::from_secs(2)
    ));
    assert!(started_at.elapsed() < Duration::from_secs(20));

    let process_group = fs::read_to_string(pid_file)
        .unwrap()
        .trim()
        .parse::<libc::pid_t>()
        .unwrap();
    let cleanup_deadline = Instant::now() + Duration::from_secs(10);
    let (result, error) = loop {
        // SAFETY: signal 0 only probes whether the dedicated process group remains.
        let result = unsafe { libc::kill(-process_group, 0) };
        let error = io::Error::last_os_error().raw_os_error();
        // macOS can report EPERM while the group's zombies are being reaped.
        // Only ESRCH proves disappearance within the existing cleanup budget.
        if (result == -1 && error == Some(libc::ESRCH)) || Instant::now() >= cleanup_deadline {
            break (result, error);
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(result, -1);
    assert_eq!(error, Some(libc::ESRCH));
}

#[test]
fn launch_profile_readiness_is_single_flight_across_parallel_reads() {
    let tmp = tempfile::TempDir::new().unwrap();
    let calls = tmp.path().join("readiness.calls");
    let launcher = executable(
        &tmp.path().join("profile-readiness"),
        &format!(
            "#!/usr/bin/env sh\nprintf x >> {}\nsleep 0.2\n",
            shell_words::quote(&calls.to_string_lossy())
        ),
    );
    let profiles = AgentLaunchProfiles::from_json(
        &json!([{
            "id":"claude-gpt",
            "label":"Claude GPT",
            "agent":"claude",
            "agent_bin":launcher,
            "readiness_args":["--check"],
        }])
        .to_string(),
    )
    .unwrap();
    let barrier = Arc::new(std::sync::Barrier::new(16));
    let threads = (0..16)
        .map(|_| {
            let barrier = barrier.clone();
            let profiles = profiles.clone();
            std::thread::spawn(move || {
                barrier.wait();
                profiles.ready_summaries()
            })
        })
        .collect::<Vec<_>>();
    for thread in threads {
        assert_eq!(
            thread.join().unwrap(),
            vec![AgentLaunchProfileSummary {
                id: "claude-gpt".to_string(),
                label: "Claude GPT".to_string(),
                agent: "claude".to_string(),
                provider_resume_import_supported: false,
                history_resume_supported: false,
            }]
        );
    }

    assert_eq!(fs::read_to_string(calls).unwrap(), "x");
}

#[test]
fn launch_profile_readiness_probes_profiles_in_parallel_and_preserves_order() {
    let tmp = tempfile::TempDir::new().unwrap();
    let launcher = executable(
        &tmp.path().join("profile-readiness"),
        "#!/usr/bin/env sh\nsleep 0.25\n",
    );
    let configs = (0..4)
        .map(|index| {
            json!({
                "id": format!("profile-{index}"),
                "label": format!("Profile {index}"),
                "agent": "claude",
                "agent_bin": launcher,
                "readiness_args": ["--check"],
            })
        })
        .collect::<Vec<_>>();
    let profiles =
        AgentLaunchProfiles::from_json(&serde_json::to_string(&configs).unwrap()).unwrap();

    let started_at = Instant::now();
    let summaries = profiles.probe_ready_summaries();
    assert!(
        started_at.elapsed() < Duration::from_millis(700),
        "bounded profiles must be probed concurrently: {:?}",
        started_at.elapsed()
    );
    assert_eq!(
        summaries
            .iter()
            .map(|summary| summary.id.as_str())
            .collect::<Vec<_>>(),
        vec!["profile-0", "profile-1", "profile-2", "profile-3"]
    );
}

#[test]
fn launch_profile_readiness_unwind_releases_single_flight_owner() {
    let tmp = tempfile::TempDir::new().unwrap();
    let launcher = executable(
        &tmp.path().join("profile-readiness"),
        "#!/usr/bin/env sh\nexit 0\n",
    );
    let profiles = AgentLaunchProfiles::from_json(
        &json!([{
            "id":"claude-gpt",
            "label":"Claude GPT",
            "agent":"claude",
            "agent_bin":launcher,
        }])
        .to_string(),
    )
    .unwrap();
    assert_eq!(profiles.ready_summaries().len(), 1);

    let (owner_claimed_tx, owner_claimed_rx) = std::sync::mpsc::sync_channel(0);
    let (release_owner_tx, release_owner_rx) = std::sync::mpsc::sync_channel(0);
    let owner_profiles = profiles.clone();
    let owner = std::thread::spawn(move || {
        PANIC_AGENT_LAUNCH_PROFILE_READINESS_PROBE.with(|panic_next| panic_next.set(true));
        BLOCK_AGENT_LAUNCH_PROFILE_READINESS_PROBE.with(|block| {
            *block.borrow_mut() = Some((owner_claimed_tx, release_owner_rx));
        });
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                owner_profiles.ready_summaries()
            }))
            .is_err()
        );
    });
    owner_claimed_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("readiness owner should claim the single-flight refresh");

    let (waiter_waiting_tx, waiter_waiting_rx) = std::sync::mpsc::sync_channel(0);
    let (waiter_done_tx, waiter_done_rx) = std::sync::mpsc::sync_channel(0);
    let waiter_profiles = profiles.clone();
    let waiter = std::thread::spawn(move || {
        SIGNAL_AGENT_LAUNCH_PROFILE_READINESS_WAITER.with(|signal| {
            *signal.borrow_mut() = Some(waiter_waiting_tx);
        });
        waiter_done_tx
            .send(waiter_profiles.ready_summaries())
            .expect("readiness waiter result should remain connected");
    });
    waiter_waiting_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("a second caller should wait behind the refresh owner");
    release_owner_tx
        .send(())
        .expect("readiness owner should remain connected");

    let summaries = waiter_done_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("the unwind guard should wake the readiness waiter");
    owner.join().unwrap();
    waiter.join().unwrap();
    assert!(
        summaries.is_empty(),
        "a waiter released after unwind must not receive stale readiness"
    );
    let (readiness, _) = &*profiles.readiness_probe;
    let state = readiness
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    assert!(
        !state.refreshing,
        "a panicking probe must release ownership"
    );
    assert!(
        state.summaries.is_empty(),
        "unwind must clear cached readiness"
    );
    drop(state);
    assert_eq!(profiles.ready_summaries().len(), 1);
}

#[test]
fn launch_profiles_reject_browser_like_or_ambiguous_configuration() {
    let invalid = json!([{
        "id": "../claude-gpt",
        "label": "Claude GPT",
        "agent": "claude",
        "agent_bin": "/bin/false",
    }])
    .to_string();
    assert!(AgentLaunchProfiles::from_json(&invalid).is_err());

    let duplicate = json!([
        {"id":"same","label":"One","agent":"claude","agent_bin":"/bin/false"},
        {"id":"same","label":"Two","agent":"claude","agent_bin":"/bin/false"},
    ])
    .to_string();
    assert!(AgentLaunchProfiles::from_json(&duplicate).is_err());
}

#[test]
fn launch_profile_config_round_trips_codex_usage_account() {
    let with_account = AgentLaunchProfiles::from_json(
        &json!([{
            "id":"claude-gpt",
            "label":"Claude GPT",
            "agent":"claude",
            "agent_bin":"/bin/false",
            "codex_usage_account":"gpt-team",
        }])
        .to_string(),
    )
    .unwrap();
    assert_eq!(
        with_account.get("claude-gpt").unwrap().codex_usage_account,
        Some("gpt-team".to_string())
    );

    // Absent leaves behavior unchanged.
    let without = AgentLaunchProfiles::from_json(
        &json!([{
            "id":"claude-gpt",
            "label":"Claude GPT",
            "agent":"claude",
            "agent_bin":"/bin/false",
        }])
        .to_string(),
    )
    .unwrap();
    assert_eq!(without.get("claude-gpt").unwrap().codex_usage_account, None);

    // Present-but-empty is rejected like the other optional fields.
    let empty = json!([{
        "id":"claude-gpt",
        "label":"Claude GPT",
        "agent":"claude",
        "agent_bin":"/bin/false",
        "codex_usage_account":"  ",
    }])
    .to_string();
    assert!(AgentLaunchProfiles::from_json(&empty).is_err());
}

fn codex_diag_all_output(stdout: &[u8], timed_out: bool) -> UsageHelperOutput {
    UsageHelperOutput {
        status_code: Some(if timed_out { 1 } else { 0 }),
        stdout: stdout.to_vec(),
        stderr: Vec::new(),
        timed_out,
    }
}

#[test]
fn codex_account_usage_snapshot_keys_off_the_named_account() {
    // Two accounts; `gpt-team` is exhausted, `other` is healthy. The
    // snapshot must reflect ONLY the requested account and derive its
    // authority/exhaustion from the Codex diag source, not native Claude.
    let output = codex_diag_all_output(
            br#"{
  "schema_version": "codex-cli.diag.rate-limits.v1",
  "command": "diag rate-limits",
  "mode": "all",
  "ok": true,
  "results": [
    {
      "name": "other",
      "provider": "codex",
      "status": "ok",
      "ok": true,
      "windows": [
        {"label": "5h", "used_percent": 10, "remaining_percent": 90, "reset_at_epoch": 1893456000}
      ]
    },
    {
      "name": "gpt-team",
      "provider": "codex",
      "status": "ok",
      "ok": true,
      "windows": [
        {"label": "5h", "used_percent": 100, "remaining_percent": 0, "reset_at_epoch": 1893456300},
        {"label": "Weekly", "used_percent": 100, "remaining_percent": 0, "reset_at_epoch": 1893456900},
        {"label": "Month", "used_percent": 40, "remaining_percent": 60, "reset_at_epoch": 1893460000}
      ]
    }
  ]
}"#,
            false,
        );

    let exhausted = codex_account_usage_snapshot(&output, "gpt-team");
    assert!(
        exhausted.authoritative,
        "successful named read is authoritative"
    );
    assert!(exhausted.has_exhausted_windows);
    let mut resets = exhausted.exhausted_reset_epochs.clone();
    resets.sort_unstable();
    assert_eq!(resets, vec![1_893_456_300, 1_893_456_900]);

    let healthy = codex_account_usage_snapshot(&output, "other");
    assert!(healthy.authoritative);
    assert!(!healthy.has_exhausted_windows);
    assert!(healthy.exhausted_reset_epochs.is_empty());
}

#[test]
fn codex_account_usage_snapshot_supports_summary_only_accounts() {
    let output = codex_diag_all_output(
            br#"{
  "schema_version": "codex-cli.diag.rate-limits.v1",
  "command": "diag rate-limits",
  "mode": "all",
  "ok": true,
  "results": [
    {
      "name": "gpt-team",
      "provider": "codex",
      "status": "ok",
      "ok": true,
      "summary": {"non_weekly_label": "5h", "non_weekly_remaining": 0, "non_weekly_reset_epoch": 1893456300, "weekly_remaining": 30, "weekly_reset_epoch": 1893460000}
    }
  ]
}"#,
            false,
        );
    let snapshot = codex_account_usage_snapshot(&output, "gpt-team");
    assert!(snapshot.authoritative);
    assert!(snapshot.has_exhausted_windows);
    assert_eq!(snapshot.exhausted_reset_epochs, vec![1_893_456_300]);
}

#[test]
fn codex_account_usage_snapshot_fails_safe_without_native_fallback() {
    let ok_body = br#"{
  "schema_version": "codex-cli.diag.rate-limits.v1",
  "command": "diag rate-limits",
  "mode": "all",
  "ok": true,
  "results": [
    {"name": "gpt-team", "provider": "codex", "status": "ok", "ok": true,
     "windows": [{"label": "5h", "used_percent": 100, "remaining_percent": 0, "reset_at_epoch": 1893456300}]}
  ]
}"#;

    // Missing nickname -> non-authoritative (never native-Claude fallback).
    let missing = codex_account_usage_snapshot(&codex_diag_all_output(ok_body, false), "absent");
    assert!(!missing.authoritative);
    assert!(!missing.has_exhausted_windows);
    assert!(missing.exhausted_reset_epochs.is_empty());

    // Timed-out read -> non-authoritative.
    let timed_out = codex_account_usage_snapshot(&codex_diag_all_output(ok_body, true), "gpt-team");
    assert!(!timed_out.authoritative);

    // Invalid JSON -> non-authoritative.
    let invalid =
        codex_account_usage_snapshot(&codex_diag_all_output(b"not json", false), "gpt-team");
    assert!(!invalid.authoritative);

    // Named entry present but errored -> non-authoritative.
    let errored = codex_account_usage_snapshot(
            &codex_diag_all_output(
                br#"{"results": [{"name": "gpt-team", "status": "error", "ok": false, "reason_code": "auth_required"}]}"#,
                false,
            ),
            "gpt-team",
        );
    assert!(!errored.authoritative);

    // Named entry ok but no windows -> non-authoritative (cannot verify state).
    let no_windows = codex_account_usage_snapshot(
        &codex_diag_all_output(
            br#"{"results": [{"name": "gpt-team", "status": "ok", "ok": true, "windows": []}]}"#,
            false,
        ),
        "gpt-team",
    );
    assert!(!no_windows.authoritative);
}

#[tokio::test]
async fn delete_tombstone_cleanup_starts_after_bind_and_is_bounded() {
    let tmp = tempfile::TempDir::new().unwrap();
    let context = CliContext {
        state_dir: tmp.path().to_path_buf(),
        host: None,
    };
    let root = tmp.path().join(crate::SESSION_DELETE_TOMBSTONES_DIR);
    std::fs::create_dir(&root).unwrap();
    for index in 0..65 {
        let tombstone = root.join(format!("session-{index}"));
        std::fs::create_dir(&tombstone).unwrap();
        std::fs::write(tombstone.join("session.json"), b"retired").unwrap();
    }

    let occupied = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = occupied.local_addr().unwrap();
    assert!(
        bind_listener_and_start_delete_tombstone_cleanup(&context, address)
            .await
            .is_err()
    );
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 65);

    drop(occupied);
    let listener = bind_listener_and_start_delete_tombstone_cleanup(&context, address)
        .await
        .unwrap();
    let connected = tokio::net::TcpStream::connect(listener.local_addr().unwrap())
        .await
        .unwrap();
    drop(connected);

    let started_at = Instant::now();
    loop {
        let remaining = std::fs::read_dir(&root).unwrap().count();
        if remaining == 1 {
            break;
        }
        assert!(
            started_at.elapsed() < Duration::from_secs(2),
            "post-bind cleanup did not finish its bounded sweep: remaining={remaining}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn delete_tombstone_cleanup_failure_does_not_block_listener_availability() {
    let tmp = tempfile::TempDir::new().unwrap();
    let context = CliContext {
        state_dir: tmp.path().to_path_buf(),
        host: None,
    };
    std::fs::write(
        tmp.path().join(crate::SESSION_DELETE_TOMBSTONES_DIR),
        b"invalid cleanup root",
    )
    .unwrap();

    let listener =
        bind_listener_and_start_delete_tombstone_cleanup(&context, "127.0.0.1:0".parse().unwrap())
            .await
            .unwrap();
    let connected = tokio::net::TcpStream::connect(listener.local_addr().unwrap())
        .await
        .unwrap();
    drop(connected);
}

struct TestProcessGroup {
    child: Option<std::process::Child>,
    process_group_id: libc::pid_t,
}

impl TestProcessGroup {
    fn spawn() -> Self {
        let mut command = std::process::Command::new("sleep");
        command.arg("30");
        // SAFETY: this test-only child must own a dedicated process session.
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() < 0 {
                    Err(io::Error::last_os_error())
                } else {
                    Ok(())
                }
            });
        }
        let child = command.spawn().expect("spawn test process session");
        let process_group_id = child.id() as libc::pid_t;
        Self {
            child: Some(child),
            process_group_id,
        }
    }

    fn pid(&self) -> u32 {
        self.child.as_ref().expect("live test child").id()
    }

    fn into_background_reaper(mut self) -> TestProcessReaper {
        let mut child = self.child.take().expect("live test child");
        let process_group_id = self.process_group_id;
        let (cleanup, cleanup_requested) = std::sync::mpsc::channel();
        TestProcessReaper {
            cleanup,
            reaper: Some(std::thread::spawn(move || {
                loop {
                    match child.try_wait() {
                        Ok(Some(status)) => return Ok(status),
                        Err(error) => return Err(error),
                        Ok(None) => {}
                    }
                    match cleanup_requested.recv_timeout(Duration::from_millis(10)) {
                        Ok(()) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                            if let Some(status) = child.try_wait()? {
                                return Ok(status);
                            }
                            // SAFETY: this thread still owns the live leader of
                            // the dedicated test-only process group.
                            unsafe {
                                libc::kill(-process_group_id, libc::SIGKILL);
                            }
                            return child.wait();
                        }
                        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                    }
                }
            })),
        }
    }
}

impl Drop for TestProcessGroup {
    fn drop(&mut self) {
        let Some(mut child) = self.child.take() else {
            return;
        };
        // SAFETY: the child is the leader of a dedicated test-only process group.
        unsafe {
            libc::kill(-self.process_group_id, libc::SIGKILL);
        }
        let _ = child.wait();
    }
}

struct TestProcessReaper {
    cleanup: std::sync::mpsc::Sender<()>,
    reaper: Option<std::thread::JoinHandle<io::Result<std::process::ExitStatus>>>,
}

impl TestProcessReaper {
    fn wait_for_exit(&mut self, timeout: Duration) -> bool {
        let started_at = Instant::now();
        while self
            .reaper
            .as_ref()
            .is_some_and(|reaper| !reaper.is_finished())
            && started_at.elapsed() < timeout
        {
            std::thread::sleep(Duration::from_millis(10));
        }
        let Some(reaper) = self.reaper.take_if(|reaper| reaper.is_finished()) else {
            return false;
        };
        matches!(reaper.join(), Ok(Ok(_)))
    }
}

impl Drop for TestProcessReaper {
    fn drop(&mut self) {
        if let Some(reaper) = self.reaper.take() {
            let _ = self.cleanup.send(());
            let _ = reaper.join();
        }
    }
}

#[test]
fn provider_prompt_subscription_requires_the_versioned_capability() {
    assert!(provider_prompt_subscription_requested(
        r#"{"subscribe":["provider-prompt.v1"]}"#
    ));
    assert!(provider_prompt_subscription_requested(
        r#"{"subscribe":["other.v1","provider-prompt.v1"]}"#
    ));
    assert!(!provider_prompt_subscription_requested(
        r#"{"subscribe":["provider-prompt.v2"]}"#
    ));
    assert!(!provider_prompt_subscription_requested(
        r#"{"text":"provider-prompt.v1"}"#
    ));
    assert!(!provider_prompt_subscription_requested("not json"));
}

#[test]
fn provider_prompt_frames_use_the_versioned_text_contract() {
    let capability = provider_prompt_capability_frame(ProviderPromptCapabilityState {
        supported: true,
        provider: Some(ProviderKind::Codex),
    });
    let capability: Value = serde_json::from_str(&capability).expect("capability json");
    assert_eq!(
        capability,
        json!({
            "schema_version": ATTACH_SCHEMA_VERSION,
            "type": "capability",
            "capability": PROVIDER_PROMPT_CAPABILITY,
            "supported": true,
            "provider": "codex",
            "prompt_max_bytes": MAX_PROVIDER_PROMPT_BYTES,
        })
    );
    let unsupported: Value = serde_json::from_str(&provider_prompt_capability_frame(
        ProviderPromptCapabilityState {
            supported: false,
            provider: None,
        },
    ))
    .expect("unsupported capability json");
    assert_eq!(
        unsupported,
        json!({
            "schema_version": ATTACH_SCHEMA_VERSION,
            "type": "capability",
            "capability": PROVIDER_PROMPT_CAPABILITY,
            "supported": false,
            "provider": null,
            "prompt_max_bytes": MAX_PROVIDER_PROMPT_BYTES,
        })
    );

    let event = provider_prompt_event_frame(
        ProviderKind::Claude,
        ProviderPromptEvent {
            id: "pp-opaque".to_string(),
            prompt: "submitted prompt".to_string(),
            submitted_at: "2099-01-01T00:00:00Z".to_string(),
            truncated: false,
        },
    );
    let event: Value = serde_json::from_str(&event).expect("event json");
    assert_eq!(
        event,
        json!({
            "schema_version": ATTACH_EVENT_SCHEMA_VERSION,
            "type": "prompt_submitted",
            "event_id": "pp-opaque",
            "provider": "claude",
            "submitted_at": "2099-01-01T00:00:00Z",
            "text": "submitted prompt",
            "truncated": false,
        })
    );
    assert!(!event.to_string().contains("/home/"));
}

#[test]
fn terminal_input_rejection_frame_is_versioned_and_secret_free() {
    let error = CliError::runtime(
        "codex-account-next-pending",
        "sensitive provider detail",
        Some(json!({ "prompt": "must not cross the attach boundary" })),
    );

    let frame: Value = serde_json::from_str(&terminal_input_rejected_frame(&error)).unwrap();

    assert_eq!(frame["schema_version"], ATTACH_EVENT_SCHEMA_VERSION);
    assert_eq!(frame["type"], "input_rejected");
    assert_eq!(frame["code"], "codex-account-next-pending");
    assert_eq!(frame["retryable"], true);
    assert!(frame.get("message").is_none());
    assert!(frame.get("details").is_none());
}

#[test]
fn terminal_input_error_recovery_preserves_runtime_failure_closure() {
    for code in [
        "codex-account-next-pending",
        "codex-account-not-bound",
        "codex-input-section-unavailable",
        "session-record-lock-timeout",
    ] {
        let error = CliError::runtime(code, "retryable input policy", None);
        assert!(
            terminal_input_error_is_recoverable(&error),
            "{code} should keep the attach socket open"
        );
    }
    for code in [
        "session-not-running",
        "attach-input-worker-failed",
        "tmux-send-failed",
        "session-identity-conflict",
        "codex-account-runtime-changed",
        "codex-account-session-incarnation-conflict",
        "codex-account-binding-invalid",
    ] {
        let error = CliError::runtime(code, "terminal failure", None);
        assert!(
            !terminal_input_error_is_recoverable(&error),
            "{code} should retain the attach close/reconnect path"
        );
    }
}

#[test]
fn pending_provider_prompt_runtime_guard_rejects_replacement_identity() {
    let mut expected = test_record("pending-runtime", "hs-pending-runtime");
    expected.runtime = Some(crate::RuntimeInfo {
        kind: "tmux".to_string(),
        tmux_session: "hs-pending-runtime".to_string(),
        generation: 1,
        started_at: "2026-07-11T00:00:00Z".to_string(),
        launch_id: "launch-1".to_string(),
        extra: std::collections::BTreeMap::new(),
    });
    let mut current = expected.clone();
    assert!(same_provider_prompt_runtime(&expected, &current));

    current.runtime.as_mut().expect("runtime").launch_id = "launch-2".to_string();
    assert!(!same_provider_prompt_runtime(&expected, &current));
    current = expected.clone();
    current.runtime.as_mut().expect("runtime").generation = 2;
    assert!(!same_provider_prompt_runtime(&expected, &current));
    current = expected.clone();
    current.runtime.as_mut().expect("runtime").tmux_session = "hs-replaced".to_string();
    assert!(!same_provider_prompt_runtime(&expected, &current));
}

#[test]
fn fresh_provider_prompt_byte_zero_requires_authoritative_provenance() {
    let mut record = provider_discovery_record(
        "prompt-provenance",
        "hs-prompt-provenance",
        "launch-prompt-provenance",
        1,
    );
    assert!(provider_prompt_new_runtime_source_authorized(&record));

    record.agent = "codex".to_string();
    let resume = record.provider_resume.as_mut().expect("provider resume");
    resume.provider = "codex".to_string();
    resume.capture_method = "codex-session-meta".to_string();
    assert!(!provider_prompt_new_runtime_source_authorized(&record));
    record
        .provider_resume
        .as_mut()
        .expect("provider resume")
        .capture_method = "codex-user-prompt-submit-hook".to_string();
    assert!(provider_prompt_new_runtime_source_authorized(&record));
    record
        .provider_resume
        .as_mut()
        .expect("provider resume")
        .capture_method = "codex-app-server-thread-binding".to_string();
    assert!(provider_prompt_new_runtime_source_authorized(&record));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pending_provider_prompt_resolver_rejects_runtime_replacement_without_replay() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let state_dir = tmp.path().join("state");
    let cwd = tmp.path().join("repo");
    std::fs::create_dir_all(&cwd).expect("cwd");
    seed_fresh_provider_session(
        &state_dir,
        "resolver-runtime-replaced",
        "codex",
        "hs-resolver-runtime-replaced",
        &cwd,
        None,
    );
    let context = CliContext {
        state_dir: state_dir.clone(),
        host: None,
    };
    let record =
        load_session_record(&context, "resolver-runtime-replaced").expect("initial session record");
    let discovery = Arc::new(ProviderPromptDiscoveryRegistry::default());
    let task = tokio::spawn(resolve_provider_prompt_tail(
        context.clone(),
        discovery,
        record,
        Instant::now() + Duration::from_secs(60),
    ));
    let mut replacement = load_session_record(&context, "resolver-runtime-replaced")
        .expect("replacement session record");
    let runtime = replacement.runtime.as_mut().expect("runtime");
    runtime.generation = 2;
    runtime.launch_id = "replacement-launch".to_string();
    runtime.tmux_session = "hs-resolver-runtime-replacement".to_string();
    replacement.provider_resume = Some(crate::ProviderResume {
        provider: "codex".to_string(),
        session_id: "replacement-provider-id".to_string(),
        captured_at: "2026-07-11T00:00:00Z".to_string(),
        capture_method: "codex-user-prompt-submit-hook".to_string(),
        resume_args: vec!["resume".to_string(), "replacement-provider-id".to_string()],
        extra: std::collections::BTreeMap::new(),
    });
    crate::write_session_record(&context, &replacement).expect("persist replacement runtime");

    let (resolved_record, tail) = task.await.expect("resolver task");
    assert_eq!(
        resolved_record
            .runtime
            .as_ref()
            .expect("resolved runtime")
            .generation,
        2
    );
    assert!(
        tail.is_none(),
        "runtime replacement must not open either runtime transcript"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pending_provider_prompt_discovery_coalesces_concurrent_scans() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let claude_config = tmp.path().join("claude-config");
    std::fs::create_dir_all(claude_config.join("projects/-pending")).expect("claude projects root");
    let _claude_config = EnvGuard::set(&lock, "CLAUDE_CONFIG_DIR", claude_config.to_str().unwrap());
    let mut record = test_record("pending-shared-scan", "hs-pending-shared-scan");
    record.agent = "claude".to_string();
    record.provider_resume = Some(crate::ProviderResume {
        provider: "claude".to_string(),
        session_id: "pending-shared-provider-id".to_string(),
        captured_at: "2026-07-11T00:00:00Z".to_string(),
        capture_method: "claude-explicit-session-id".to_string(),
        resume_args: vec![
            "--resume".to_string(),
            "pending-shared-provider-id".to_string(),
        ],
        extra: std::collections::BTreeMap::new(),
    });
    record.runtime = Some(crate::RuntimeInfo {
        kind: "tmux".to_string(),
        tmux_session: "hs-pending-shared-scan".to_string(),
        generation: 1,
        started_at: "2026-07-11T00:00:00Z".to_string(),
        launch_id: "launch-shared-scan".to_string(),
        extra: std::collections::BTreeMap::new(),
    });
    let registry = Arc::new(ProviderPromptDiscoveryRegistry::default());
    let mut tasks = tokio::task::JoinSet::new();
    for _ in 0..8 {
        let registry = registry.clone();
        let record = record.clone();
        tasks.spawn(async move { registry.resolve_source(&record).await });
    }
    while tasks.join_next().await.is_some() {}

    assert_eq!(
        registry.scan_attempts(&record).await,
        1,
        "concurrent clients must share one provider-history scan"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pending_provider_prompt_scan_survives_lead_waiter_cancellation() {
    let started = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let active = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let max_active = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let release = Arc::new(AtomicBool::new(false));
    let registry = Arc::new(ProviderPromptDiscoveryRegistry::with_resolver({
        let started = started.clone();
        let active = active.clone();
        let max_active = max_active.clone();
        let release = release.clone();
        move |_| {
            started.fetch_add(1, Ordering::SeqCst);
            let current = active.fetch_add(1, Ordering::SeqCst) + 1;
            max_active.fetch_max(current, Ordering::SeqCst);
            while !release.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(5));
            }
            active.fetch_sub(1, Ordering::SeqCst);
            None
        }
    }));
    let record = provider_discovery_record(
        "cancelled-lead-scan",
        "hs-cancelled-lead-scan",
        "launch-cancelled-lead-scan",
        1,
    );
    let lead = {
        let registry = registry.clone();
        let record = record.clone();
        tokio::spawn(async move { registry.resolve_source(&record).await })
    };
    while started.load(Ordering::SeqCst) == 0 {
        tokio::task::yield_now().await;
    }
    lead.abort();
    let _ = lead.await;

    let mut replacements = tokio::task::JoinSet::new();
    for _ in 0..4 {
        let registry = registry.clone();
        let record = record.clone();
        replacements.spawn(async move { registry.resolve_source(&record).await });
    }
    registry.wait_for_parked_scan_waiters(&record, 4).await;
    assert_eq!(started.load(Ordering::SeqCst), 1);
    assert_eq!(max_active.load(Ordering::SeqCst), 1);
    release.store(true, Ordering::SeqCst);
    while replacements.join_next().await.is_some() {}
    assert_eq!(registry.scan_attempts(&record).await, 1);
    assert!(registry.resolve_source(&record).await.is_none());
    assert_eq!(registry.scan_attempts(&record).await, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn provider_prompt_recovery_is_single_flight_and_concurrency_bounded() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let transcript = tmp.path().join("prompt.jsonl");
    std::fs::write(&transcript, "").expect("transcript");
    let started = Arc::new(AtomicUsize::new(0));
    let active = Arc::new(AtomicUsize::new(0));
    let max_active = Arc::new(AtomicUsize::new(0));
    let release = Arc::new(AtomicBool::new(false));
    let registry = Arc::new(
        ProviderPromptDiscoveryRegistry::with_resolver_and_tracker_opener(
            {
                let transcript = transcript.clone();
                move |record| {
                    Some(ProviderPromptSource::test_path(
                        ProviderKind::Codex,
                        record.id.clone(),
                        transcript.clone(),
                    ))
                }
            },
            {
                let started = started.clone();
                let active = active.clone();
                let max_active = max_active.clone();
                let release = release.clone();
                move |_| {
                    started.fetch_add(1, Ordering::SeqCst);
                    let current = active.fetch_add(1, Ordering::SeqCst) + 1;
                    max_active.fetch_max(current, Ordering::SeqCst);
                    while !release.load(Ordering::SeqCst) {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    active.fetch_sub(1, Ordering::SeqCst);
                    None
                }
            },
        ),
    );
    let records: Vec<_> = (0..6)
        .map(|index| {
            provider_discovery_record(
                &format!("recovery-{index}"),
                &format!("hs-recovery-{index}"),
                &format!("launch-recovery-{index}"),
                1,
            )
        })
        .collect();

    // Source discovery runs in the background, so recoveries start on the
    // list passes after it resolves.
    tokio::time::timeout(Duration::from_secs(5), async {
        while started.load(Ordering::SeqCst) < PROVIDER_PROMPT_RECOVERY_MAX_CONCURRENT_SCANS {
            for record in &records {
                assert!(registry.last_prompt(record).await.is_none());
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("recoveries start");
    for record in &records {
        assert!(registry.last_prompt(record).await.is_none());
    }
    assert_eq!(
        started.load(Ordering::SeqCst),
        PROVIDER_PROMPT_RECOVERY_MAX_CONCURRENT_SCANS,
        "duplicate list polls must not start duplicate recoveries"
    );
    assert_eq!(
        max_active.load(Ordering::SeqCst),
        PROVIDER_PROMPT_RECOVERY_MAX_CONCURRENT_SCANS
    );

    release.store(true, Ordering::SeqCst);
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if started.load(Ordering::SeqCst) == records.len() && active.load(Ordering::SeqCst) == 0
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("bounded recoveries complete");
    assert!(max_active.load(Ordering::SeqCst) <= PROVIDER_PROMPT_RECOVERY_MAX_CONCURRENT_SCANS);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn provider_prompt_discovery_recovers_and_tracks_long_codex_turns() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let codex_home = tmp.path().join("codex-home");
    let transcript = codex_home.join("sessions/2026/07/session.jsonl");
    std::fs::create_dir_all(transcript.parent().expect("transcript parent"))
        .expect("transcript dir");
    let provider_id = "long-codex-provider";
    let metadata = format!(
        "{}\n",
        json!({
            "type":"session_meta",
            "payload":{
                "id":provider_id,
                "session_id":provider_id,
                "cwd":"/tmp",
                "source":"cli",
                "timestamp":"2099-01-01T00:00:00Z"
            }
        })
    );
    let prompt_line = |text: &str| {
        format!(
            "{}\n",
            json!({
                "type":"event_msg",
                "payload":{"type":"user_message","message":text}
            })
        )
    };
    let long_output = format!("{}\n", " ".repeat(1024 * 1024));
    std::fs::write(
        &transcript,
        format!("{metadata}{}{}", prompt_line("cold prompt"), long_output),
    )
    .expect("transcript");
    let _codex_home = EnvGuard::set(&lock, "CODEX_HOME", codex_home.to_str().unwrap());
    let mut record = provider_discovery_record("long-codex", "hs-long-codex", "launch", 1);
    record.agent = "codex".to_string();
    let resume = record.provider_resume.as_mut().expect("provider resume");
    resume.provider = "codex".to_string();
    resume.session_id = provider_id.to_string();
    let registry = ProviderPromptDiscoveryRegistry::default();

    assert_eq!(
        registry.last_prompt(&record).await,
        None,
        "cold recovery must warm outside the GET /sessions critical path"
    );
    let cold_prompt = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(prompt) = registry.last_prompt(&record).await {
                break prompt;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("cold prompt recovery")
    .text;
    assert_eq!(cold_prompt, "cold prompt");
    OpenOptions::new()
        .append(true)
        .open(&transcript)
        .expect("append transcript")
        .write_all(prompt_line("appended prompt").as_bytes())
        .expect("append prompt");
    let current = registry.last_prompt(&record).await.expect("current prompt");
    assert_eq!(
        current.text, "appended prompt",
        "a bounded warm append must return the newest prompt immediately"
    );
    let appended_prompt = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(prompt) = registry.last_prompt(&record).await {
                break prompt;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("appended prompt refresh")
    .text;
    assert_eq!(appended_prompt, "appended prompt");

    std::fs::write(
        &transcript,
        format!("{metadata}{}", prompt_line("replacement prompt")),
    )
    .expect("replace transcript");
    assert_eq!(registry.last_prompt(&record).await, None);
    let replacement_prompt = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(prompt) = registry.last_prompt(&record).await {
                break prompt;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("replacement prompt recovery")
    .text;
    assert_eq!(replacement_prompt, "replacement prompt");
}

#[tokio::test]
async fn provider_prompt_discovery_prunes_old_runtimes_and_deleted_sessions() {
    let registry = ProviderPromptDiscoveryRegistry::with_resolver(|_| None);
    let first = provider_discovery_record(
        "discovery-lifecycle",
        "hs-discovery-lifecycle",
        "launch-1",
        1,
    );
    assert!(registry.resolve_source(&first).await.is_none());
    assert_eq!(registry.entry_count().await, 1);

    let second = provider_discovery_record(
        "discovery-lifecycle",
        "hs-discovery-lifecycle",
        "launch-2",
        2,
    );
    assert!(registry.resolve_source(&second).await.is_none());
    assert_eq!(registry.entry_count().await, 1);

    registry
        .evict_session_if_matches(&crate::SessionRegistryFence::from_record(&first))
        .await;
    assert_eq!(
        registry.entry_count().await,
        1,
        "late cleanup for the deleted generation must retain its replacement"
    );
    registry
        .evict_session_if_matches(&crate::SessionRegistryFence::from_record(&second))
        .await;
    assert_eq!(registry.entry_count().await, 0);
}

#[tokio::test]
async fn provider_prompt_discovery_bounds_distinct_session_churn() {
    let registry = ProviderPromptDiscoveryRegistry::with_resolver(|_| None);
    let retained: Vec<_> = (0..PROVIDER_PROMPT_DISCOVERY_MAX_ENTRIES)
        .map(|index| {
            let id = format!("discovery-churn-{index}");
            provider_discovery_record(&id, &format!("hs-{id}"), &format!("launch-{index}"), 1)
        })
        .collect();
    for record in &retained {
        assert!(registry.resolve_source(record).await.is_none());
    }
    for index in 0..10 {
        let id = format!("discovery-churn-{index}");
        let record = provider_discovery_record(
            &format!("overflow-{id}"),
            &format!("hs-overflow-{id}"),
            &format!("overflow-launch-{index}"),
            1,
        );
        assert!(registry.resolve_source(&record).await.is_none());
        assert!(
            !registry.contains_record(&record).await,
            "overflow sessions must not churn the stable recovery set"
        );
    }
    assert_eq!(
        registry.entry_count().await,
        PROVIDER_PROMPT_DISCOVERY_MAX_ENTRIES
    );
    for record in &retained {
        assert!(
            registry.contains_record(record).await,
            "stable entries must survive repeated overflow attempts"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn provider_prompt_discovery_does_not_evict_active_slots_and_bounds_scans() {
    let started = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let active = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let max_active = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let release = Arc::new(AtomicBool::new(false));
    let registry = Arc::new(ProviderPromptDiscoveryRegistry::with_resolver({
        let started = started.clone();
        let active = active.clone();
        let max_active = max_active.clone();
        let release = release.clone();
        move |_| {
            started.fetch_add(1, Ordering::SeqCst);
            let current = active.fetch_add(1, Ordering::SeqCst) + 1;
            max_active.fetch_max(current, Ordering::SeqCst);
            while !release.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(5));
            }
            active.fetch_sub(1, Ordering::SeqCst);
            None
        }
    }));
    let records: Vec<_> = (0..PROVIDER_PROMPT_DISCOVERY_MAX_ENTRIES)
        .map(|index| {
            let id = format!("active-discovery-{index}");
            provider_discovery_record(&id, &format!("hs-{id}"), &format!("launch-{index}"), 1)
        })
        .collect();
    let mut tasks = tokio::task::JoinSet::new();
    for record in &records {
        let registry = registry.clone();
        let record = record.clone();
        tasks.spawn(async move { registry.resolve_source(&record).await });
    }
    while registry.in_flight_count().await < PROVIDER_PROMPT_DISCOVERY_MAX_ENTRIES {
        tokio::task::yield_now().await;
    }
    while started.load(Ordering::SeqCst) < PROVIDER_PROMPT_DISCOVERY_MAX_CONCURRENT_SCANS {
        tokio::task::yield_now().await;
    }
    for index in 0..10 {
        let id = format!("overflow-discovery-{index}");
        let record = provider_discovery_record(
            &id,
            &format!("hs-{id}"),
            &format!("overflow-launch-{index}"),
            1,
        );
        assert!(registry.resolve_source(&record).await.is_none());
    }
    let duplicate = {
        let registry = registry.clone();
        let record = records[0].clone();
        tokio::spawn(async move { registry.resolve_source(&record).await })
    };
    tokio::time::sleep(Duration::from_millis(25)).await;
    assert_eq!(registry.scan_attempts(&records[0]).await, 1);
    assert_eq!(
        max_active.load(Ordering::SeqCst),
        PROVIDER_PROMPT_DISCOVERY_MAX_CONCURRENT_SCANS
    );

    release.store(true, Ordering::SeqCst);
    while tasks.join_next().await.is_some() {}
    duplicate.await.expect("duplicate waiter");
    assert_eq!(
        registry.entry_count().await,
        PROVIDER_PROMPT_DISCOVERY_MAX_ENTRIES
    );
    assert!(max_active.load(Ordering::SeqCst) <= PROVIDER_PROMPT_DISCOVERY_MAX_CONCURRENT_SCANS);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn provider_prompt_resolver_evicts_stale_cache_and_rediscovers_exact_source() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let claude_config = tmp.path().join("claude-config");
    let _claude_config = EnvGuard::set(&lock, "CLAUDE_CONFIG_DIR", claude_config.to_str().unwrap());
    let record =
        provider_discovery_record("stale-cache", "hs-stale-cache", "launch-stale-cache", 2);
    let provider_id = record
        .provider_resume
        .as_ref()
        .expect("provider resume")
        .session_id
        .clone();
    let first = claude_config
        .join("projects/-first")
        .join(format!("{provider_id}.jsonl"));
    std::fs::create_dir_all(first.parent().expect("first parent")).expect("first dir");
    std::fs::write(
        &first,
        format!(
            "{}\n",
            json!({
                "type":"user",
                "sessionId":provider_id,
                "cwd":record.cwd.clone(),
                "message":{"role":"user","content":"first"}
            })
        ),
    )
    .expect("first transcript");
    let registry = Arc::new(ProviderPromptDiscoveryRegistry::default());
    assert!(registry.resolve_source(&record).await.is_some());

    // The cached source disappears; a still-present mismatched exact file is
    // covered separately, because it must fail closed.
    std::fs::remove_file(&first).expect("remove cached transcript");
    let second = claude_config
        .join("projects/-second")
        .join(format!("{provider_id}.jsonl"));
    std::fs::create_dir_all(second.parent().expect("second parent")).expect("second dir");
    std::fs::write(
        &second,
        format!(
            "{}\n",
            json!({
                "type":"user",
                "sessionId":provider_id,
                "cwd":record.cwd.clone(),
                "message":{"role":"user","content":"replacement"}
            })
        ),
    )
    .expect("replacement transcript");

    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let (_, tail) = resolve_provider_prompt_tail(
        context,
        registry.clone(),
        record.clone(),
        Instant::now() + Duration::from_secs(1),
    )
    .await;
    assert!(tail.is_some(), "stale cache must be rediscovered exactly");
    assert_eq!(registry.scan_attempts(&record).await, 2);
}

#[tokio::test]
async fn outbound_writer_prioritizes_terminal_bytes_over_control_events() {
    let (terminal_tx, mut terminal_rx) = mpsc::channel(2);
    let (control_tx, mut control_rx) = mpsc::channel(2);
    control_tx
        .send(Message::Text("control".into()))
        .await
        .expect("queue control");
    terminal_tx
        .send(Message::Binary(vec![1, 2, 3].into()))
        .await
        .expect("queue terminal");

    let mut terminal_burst = 0;
    let first = next_outbound_message(&mut terminal_rx, &mut control_rx, &mut terminal_burst)
        .await
        .expect("first frame");
    let second = next_outbound_message(&mut terminal_rx, &mut control_rx, &mut terminal_burst)
        .await
        .expect("second frame");
    assert!(matches!(first, Message::Binary(_)));
    assert!(matches!(second, Message::Text(_)));
}

#[tokio::test]
async fn outbound_writer_delivers_control_within_a_bounded_terminal_burst() {
    let (terminal_tx, mut terminal_rx) = mpsc::channel(ATTACH_TERMINAL_QUEUE_CAPACITY);
    let (control_tx, mut control_rx) = mpsc::channel(2);
    for byte in 0..ATTACH_TERMINAL_QUEUE_CAPACITY {
        terminal_tx
            .send(Message::Binary(vec![byte as u8].into()))
            .await
            .expect("queue terminal");
    }
    control_tx
        .send(Message::Text("capability".into()))
        .await
        .expect("queue control");

    let mut selected_control = false;
    let mut terminal_burst = 0;
    for _ in 0..=32 {
        let message = next_outbound_message(&mut terminal_rx, &mut control_rx, &mut terminal_burst)
            .await
            .expect("frame");
        if matches!(message, Message::Text(_)) {
            selected_control = true;
            break;
        }
    }
    assert!(
        selected_control,
        "control frame exceeded the terminal burst bound"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn websocket_subscription_emits_capability_and_new_provider_prompt_only() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let state_dir = tmp.path().join("state");
    let codex_home = tmp.path().join("codex-home");
    let transcript = codex_home.join("sessions/2026/07/session.jsonl");
    let cwd = tmp.path().join("repo");
    std::fs::create_dir_all(transcript.parent().expect("transcript parent"))
        .expect("transcript dir");
    std::fs::create_dir_all(&cwd).expect("cwd");
    std::fs::write(
        &transcript,
        format!(
            "{}\n{}\n",
            json!({
                "timestamp":"2099-01-01T00:00:00Z",
                "type":"session_meta",
                "payload":{
                    "id":"resume-session-id",
                    "session_id":"resume-session-id",
                    "cwd":cwd.to_string_lossy(),
                    "source":"cli",
                    "timestamp":"2099-01-01T00:00:00Z"
                }
            }),
            json!({
                "type":"event_msg",
                "payload":{"type":"user_message","message":"pre-subscription prompt"}
            })
        ),
    )
    .expect("transcript");
    let _codex_home = EnvGuard::set(&lock, "CODEX_HOME", codex_home.to_str().unwrap());
    seed_resumable_session(
        &state_dir,
        "ws-prompt",
        "codex",
        "hs-codex-ws-prompt",
        &cwd,
        &["resume", "resume-session-id"],
    );
    let tmux = minimal_tmux(tmp.path());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener");
    let addr = listener.local_addr().expect("address");
    let server_state_dir = state_dir.clone();
    let server_state = state(&server_state_dir, Some(TOKEN), tmux);
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, router(server_state)).await;
    });

    let mut request = format!("ws://{addr}/sessions/ws-prompt/attach")
        .into_client_request()
        .expect("request");
    request.headers_mut().insert(
        AUTHORIZATION,
        format!("Bearer {TOKEN}").parse().expect("authorization"),
    );
    let (mut socket, _) = connect_async(request).await.expect("connect");
    let snapshot = tokio::time::timeout(Duration::from_secs(2), socket.next())
        .await
        .expect("snapshot timeout")
        .expect("snapshot frame")
        .expect("snapshot message");
    assert!(matches!(snapshot, ClientMessage::Binary(_)));

    socket
        .send(ClientMessage::Text(
            json!({"subscribe":[PROVIDER_PROMPT_CAPABILITY]})
                .to_string()
                .into(),
        ))
        .await
        .expect("subscribe");
    let capability = tokio::time::timeout(Duration::from_secs(2), socket.next())
        .await
        .expect("capability timeout")
        .expect("capability frame")
        .expect("capability message");
    let ClientMessage::Text(capability) = capability else {
        panic!("capability must be a text frame");
    };
    let capability: Value = serde_json::from_str(capability.as_str()).expect("capability json");
    assert_eq!(capability["type"], "capability");
    assert_eq!(capability["supported"], true);

    OpenOptions::new()
        .append(true)
        .open(&transcript)
        .expect("append transcript")
        .write_all(
            format!(
                "{}\n",
                json!({
                    "type":"event_msg",
                    "payload":{"type":"user_message","message":"websocket prompt"}
                })
            )
            .as_bytes(),
        )
        .expect("write prompt");
    let event = tokio::time::timeout(Duration::from_secs(2), socket.next())
        .await
        .expect("event timeout")
        .expect("event frame")
        .expect("event message");
    let ClientMessage::Text(event) = event else {
        panic!("prompt event must be a text frame");
    };
    let event: Value = serde_json::from_str(event.as_str()).expect("event json");
    assert_eq!(event["type"], "prompt_submitted");
    assert_eq!(event["text"], "websocket prompt");
    assert!(event["event_id"].is_string());
    assert!(event["submitted_at"].is_string());
    assert_eq!(event["provider"], "codex");
    assert!(
        !event
            .to_string()
            .contains(transcript.to_string_lossy().as_ref())
    );

    let rotated = transcript.with_extension("jsonl.old");
    std::fs::rename(&transcript, &rotated).expect("rotate transcript");
    std::fs::write(
        &transcript,
        format!(
            "{}\n",
            json!({
                "timestamp":"2099-01-01T00:00:00Z",
                "type":"session_meta",
                "payload":{
                    "id":"different-session-id",
                    "cwd":cwd.to_string_lossy(),
                    "source":"cli",
                    "timestamp":"2099-01-01T00:00:00Z"
                }
            })
        ),
    )
    .expect("mismatched replacement");
    socket
        .send(ClientMessage::Text(
            json!({"subscribe":[PROVIDER_PROMPT_CAPABILITY]})
                .to_string()
                .into(),
        ))
        .await
        .expect("resubscribe");
    let capability = tokio::time::timeout(Duration::from_secs(2), socket.next())
        .await
        .expect("resubscribe timeout")
        .expect("resubscribe frame")
        .expect("resubscribe message");
    let ClientMessage::Text(capability) = capability else {
        panic!("capability must be a text frame");
    };
    let capability: Value = serde_json::from_str(capability.as_str()).expect("capability json");
    assert_eq!(capability["supported"], false);

    let _ = socket.close(None).await;
    let session_dir = state_dir.join("sessions/ws-prompt");
    let mut cleaned = false;
    for _ in 0..20 {
        cleaned = std::fs::read_dir(&session_dir)
            .expect("session dir")
            .flatten()
            .all(|entry| !entry.file_name().to_string_lossy().starts_with("attach-"));
        if cleaned {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(cleaned, "disconnect must remove the private attach pipe");
    server.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn websocket_subscription_recovers_fresh_codex_identity_and_first_prompt() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let state_dir = tmp.path().join("state");
    let codex_home = tmp.path().join("codex-home");
    let cwd = tmp.path().join("repo");
    std::fs::create_dir_all(&cwd).expect("cwd");
    let created_at = seed_fresh_provider_session(
        &state_dir,
        "ws-codex-pending",
        "codex",
        "hs-codex-pending",
        &cwd,
        None,
    );
    let _codex_home = EnvGuard::set(&lock, "CODEX_HOME", codex_home.to_str().unwrap());
    let tmux = minimal_tmux(tmp.path());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener");
    let addr = listener.local_addr().expect("address");
    let server_state_dir = state_dir.clone();
    let server_state = state(&server_state_dir, Some(TOKEN), tmux);
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, router(server_state)).await;
    });

    let mut request = format!("ws://{addr}/sessions/ws-codex-pending/attach")
        .into_client_request()
        .expect("request");
    request.headers_mut().insert(
        AUTHORIZATION,
        format!("Bearer {TOKEN}").parse().expect("authorization"),
    );
    let (mut socket, _) = connect_async(request).await.expect("connect");
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(2), socket.next())
            .await
            .expect("snapshot timeout")
            .expect("snapshot frame")
            .expect("snapshot message"),
        ClientMessage::Binary(_)
    ));
    socket
        .send(ClientMessage::Text(
            json!({"subscribe":[PROVIDER_PROMPT_CAPABILITY]})
                .to_string()
                .into(),
        ))
        .await
        .expect("subscribe before identity");

    let transcript = codex_home.join("sessions/2026/07/pending.jsonl");
    std::fs::create_dir_all(transcript.parent().expect("transcript parent"))
        .expect("transcript dir");
    std::fs::write(
        &transcript,
        format!(
            "{}\n{}\n",
            json!({
                "timestamp":created_at,
                "type":"session_meta",
                "payload":{
                    "id":"fresh-codex-id",
                    "session_id":"fresh-codex-id",
                    "cwd":cwd.to_string_lossy(),
                    "source":"cli",
                    "timestamp":created_at
                }
            }),
            json!({
                "timestamp":created_at,
                "type":"event_msg",
                "payload":{"type":"user_message","message":"fresh codex first prompt"}
            })
        ),
    )
    .expect("fresh transcript");
    persist_provider_resume_identity(
        &state_dir,
        "ws-codex-pending",
        "codex",
        "fresh-codex-id",
        "codex-user-prompt-submit-hook",
    );

    assert_supported_provider_prompt(&mut socket, "codex").await;
    assert_provider_prompt(&mut socket, "codex", "fresh codex first prompt").await;
    assert!(
        tokio::time::timeout(Duration::from_millis(250), socket.next())
            .await
            .is_err(),
        "fresh first prompt must be emitted exactly once"
    );
    let _ = socket.close(None).await;

    let mut request = format!("ws://{addr}/sessions/ws-codex-pending/attach")
        .into_client_request()
        .expect("reconnect request");
    request.headers_mut().insert(
        AUTHORIZATION,
        format!("Bearer {TOKEN}").parse().expect("authorization"),
    );
    let (mut reconnect, _) = connect_async(request).await.expect("reconnect");
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(2), reconnect.next())
            .await
            .expect("reconnect snapshot timeout")
            .expect("reconnect snapshot frame")
            .expect("reconnect snapshot message"),
        ClientMessage::Binary(_)
    ));
    reconnect
        .send(ClientMessage::Text(
            json!({"subscribe":[PROVIDER_PROMPT_CAPABILITY]})
                .to_string()
                .into(),
        ))
        .await
        .expect("reconnect subscribe");
    assert_supported_provider_prompt(&mut reconnect, "codex").await;
    assert!(
        tokio::time::timeout(Duration::from_millis(250), reconnect.next())
            .await
            .is_err(),
        "reconnect must baseline at EOF instead of replaying the recovered prompt"
    );
    let _ = reconnect.close(None).await;
    server.abort();
    let _ = server.await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn websocket_pending_codex_rejects_unrelated_same_cwd_transcript() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let state_dir = tmp.path().join("state");
    let codex_home = tmp.path().join("codex-home");
    let cwd = tmp.path().join("repo");
    std::fs::create_dir_all(&cwd).expect("cwd");
    let created_at = seed_fresh_provider_session(
        &state_dir,
        "ws-codex-unrelated",
        "codex",
        "hs-codex-unrelated",
        &cwd,
        None,
    );
    let _codex_home = EnvGuard::set(&lock, "CODEX_HOME", codex_home.to_str().unwrap());
    let tmux = minimal_tmux(tmp.path());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener");
    let addr = listener.local_addr().expect("address");
    let server_state_dir = state_dir.clone();
    let server_state = state(&server_state_dir, Some(TOKEN), tmux);
    let list_state = server_state.clone();
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, router(server_state)).await;
    });

    let mut request = format!("ws://{addr}/sessions/ws-codex-unrelated/attach")
        .into_client_request()
        .expect("request");
    request.headers_mut().insert(
        AUTHORIZATION,
        format!("Bearer {TOKEN}").parse().expect("authorization"),
    );
    let (mut socket, _) = connect_async(request).await.expect("connect");
    let _snapshot = tokio::time::timeout(Duration::from_secs(2), socket.next())
        .await
        .expect("snapshot timeout")
        .expect("snapshot frame")
        .expect("snapshot message");
    socket
        .send(ClientMessage::Text(
            json!({"subscribe":[PROVIDER_PROMPT_CAPABILITY]})
                .to_string()
                .into(),
        ))
        .await
        .expect("subscribe");

    let transcript = codex_home.join("sessions/2026/07/unrelated.jsonl");
    std::fs::create_dir_all(transcript.parent().expect("transcript parent"))
        .expect("transcript dir");
    std::fs::write(
        &transcript,
        format!(
            "{}\n{}\n",
            json!({
                "timestamp":created_at,
                "type":"session_meta",
                "payload":{
                    "id":"unrelated-codex-id",
                    "session_id":"unrelated-codex-id",
                    "cwd":cwd.to_string_lossy(),
                    "source":"cli",
                    "timestamp":created_at
                }
            }),
            json!({
                "timestamp":created_at,
                "type":"event_msg",
                "payload":{"type":"user_message","message":"unrelated private prompt"}
            })
        ),
    )
    .expect("unrelated transcript");

    let (status, _) = call(router(list_state), get("/sessions")).await;
    assert_eq!(status, StatusCode::OK, "list must remain a read-only probe");

    if let Ok(Some(Ok(ClientMessage::Text(frame)))) =
        tokio::time::timeout(Duration::from_millis(500), socket.next()).await
    {
        let frame: Value = serde_json::from_str(frame.as_str()).expect("control json");
        assert_eq!(
            frame["supported"], false,
            "unrelated transcript must not resolve capability"
        );
    }
    let record: Value = serde_json::from_slice(
        &std::fs::read(state_dir.join("sessions/ws-codex-unrelated/session.json"))
            .expect("session record"),
    )
    .expect("session json");
    assert!(
        record.get("provider_resume").is_none(),
        "pending provider discovery must not persist same-cwd history"
    );
    let _ = socket.close(None).await;

    server.abort();
    let _ = server.await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn websocket_subscription_recovers_fresh_claude_transcript_and_first_prompt() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let state_dir = tmp.path().join("state");
    let claude_config = tmp.path().join("claude-config");
    let cwd = tmp.path().join("repo");
    let provider_session_id = "fresh-claude-id";
    std::fs::create_dir_all(&cwd).expect("cwd");
    let created_at = seed_fresh_provider_session(
        &state_dir,
        "ws-claude-pending",
        "claude",
        "hs-claude-pending",
        &cwd,
        Some((provider_session_id, "claude-explicit-session-id")),
    );
    let _claude_config = EnvGuard::set(&lock, "CLAUDE_CONFIG_DIR", claude_config.to_str().unwrap());
    let tmux = minimal_tmux(tmp.path());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener");
    let addr = listener.local_addr().expect("address");
    let server_state_dir = state_dir.clone();
    let server_state = state(&server_state_dir, Some(TOKEN), tmux);
    let pending_state = server_state.clone();
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, router(server_state)).await;
    });

    let mut request = format!("ws://{addr}/sessions/ws-claude-pending/attach")
        .into_client_request()
        .expect("request");
    request.headers_mut().insert(
        AUTHORIZATION,
        format!("Bearer {TOKEN}").parse().expect("authorization"),
    );
    let (mut socket, _) = connect_async(request).await.expect("connect");
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(2), socket.next())
            .await
            .expect("snapshot timeout")
            .expect("snapshot frame")
            .expect("snapshot message"),
        ClientMessage::Binary(_)
    ));
    socket
        .send(ClientMessage::Text(
            json!({"subscribe":[PROVIDER_PROMPT_CAPABILITY]})
                .to_string()
                .into(),
        ))
        .await
        .expect("subscribe before transcript");

    sync_waits::wait_for_pending_scan(&pending_state, "ws-claude-pending").await;
    let transcript = claude_config
        .join("projects/-pending")
        .join(format!("{provider_session_id}.jsonl"));
    std::fs::create_dir_all(transcript.parent().expect("transcript parent"))
        .expect("transcript dir");
    std::fs::write(
        &transcript,
        format!(
            "{}\n{}\n",
            json!({
                "type":"user",
                "uuid":"claude-turn-1",
                "sessionId":provider_session_id,
                "cwd":cwd.to_string_lossy(),
                "timestamp":created_at,
                "message":{"role":"user","content":"fresh claude first prompt"}
            }),
            json!({
                "type":"last-prompt",
                "sessionId":provider_session_id,
                "leafUuid":"claude-turn-1",
                "timestamp":created_at,
                "lastPrompt":"fresh claude first prompt"
            })
        ),
    )
    .expect("fresh transcript");

    assert_supported_provider_prompt(&mut socket, "claude").await;
    assert_provider_prompt(&mut socket, "claude", "fresh claude first prompt").await;
    assert!(
        tokio::time::timeout(Duration::from_millis(250), socket.next())
            .await
            .is_err(),
        "fresh first prompt must be emitted exactly once"
    );
    let _ = socket.close(None).await;

    let mut request = format!("ws://{addr}/sessions/ws-claude-pending/attach")
        .into_client_request()
        .expect("reconnect request");
    request.headers_mut().insert(
        AUTHORIZATION,
        format!("Bearer {TOKEN}").parse().expect("authorization"),
    );
    let (mut reconnect, _) = connect_async(request).await.expect("reconnect");
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(2), reconnect.next())
            .await
            .expect("reconnect snapshot timeout")
            .expect("reconnect snapshot frame")
            .expect("reconnect snapshot message"),
        ClientMessage::Binary(_)
    ));
    reconnect
        .send(ClientMessage::Text(
            json!({"subscribe":[PROVIDER_PROMPT_CAPABILITY]})
                .to_string()
                .into(),
        ))
        .await
        .expect("reconnect subscribe");
    assert_supported_provider_prompt(&mut reconnect, "claude").await;
    assert!(
        tokio::time::timeout(Duration::from_millis(250), reconnect.next())
            .await
            .is_err(),
        "Claude reconnect must baseline the cached source at EOF"
    );
    let _ = reconnect.close(None).await;
    server.abort();
    let _ = server.await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn websocket_unsubscribed_client_gets_binary_only_and_unsupported_ack() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let state_dir = tmp.path().join("state");
    seed_session(&state_dir, "ws-unsupported", "dsh", "hs-dsh-ws");
    let tmux = minimal_tmux(tmp.path());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener");
    let addr = listener.local_addr().expect("address");
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, router(state(&state_dir, Some(TOKEN), tmux))).await;
    });

    let mut request = format!("ws://{addr}/sessions/ws-unsupported/attach")
        .into_client_request()
        .expect("request");
    request.headers_mut().insert(
        AUTHORIZATION,
        format!("Bearer {TOKEN}").parse().expect("authorization"),
    );
    let (mut socket, _) = connect_async(request).await.expect("connect");
    let snapshot = tokio::time::timeout(Duration::from_secs(2), socket.next())
        .await
        .expect("snapshot timeout")
        .expect("snapshot frame")
        .expect("snapshot message");
    assert!(matches!(snapshot, ClientMessage::Binary(_)));
    assert!(
        tokio::time::timeout(Duration::from_millis(200), socket.next())
            .await
            .is_err(),
        "unsubscribed clients must not receive text events"
    );

    socket
        .send(ClientMessage::Text(
            json!({"subscribe":[PROVIDER_PROMPT_CAPABILITY]})
                .to_string()
                .into(),
        ))
        .await
        .expect("subscribe");
    let capability = tokio::time::timeout(Duration::from_secs(2), socket.next())
        .await
        .expect("capability timeout")
        .expect("capability frame")
        .expect("capability message");
    let ClientMessage::Text(capability) = capability else {
        panic!("capability must be a text frame");
    };
    let capability: Value = serde_json::from_str(capability.as_str()).expect("capability json");
    assert_eq!(capability["supported"], false);
    let _ = socket.close(None).await;
    server.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn websocket_subscription_rejects_heuristic_same_cwd_codex_backfill() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let state_dir = tmp.path().join("state");
    let codex_home = tmp.path().join("codex-home");
    let transcript = codex_home.join("sessions/2000/01/session.jsonl");
    std::fs::create_dir_all(transcript.parent().expect("parent")).expect("dirs");
    std::fs::write(
        &transcript,
        format!(
            "{}\n",
            json!({
                "timestamp":"2000-01-01T00:00:00Z",
                "type":"session_meta",
                "payload":{
                    "id":"backfill-resume-id",
                    "cwd":"/tmp",
                    "source":"cli",
                    "timestamp":"2000-01-01T00:00:00Z"
                }
            })
        ),
    )
    .expect("transcript");
    let _codex_home = EnvGuard::set(&lock, "CODEX_HOME", codex_home.to_str().unwrap());
    seed_session(&state_dir, "ws-backfill", "codex", "hs-codex-backfill");
    let record_path = state_dir.join("sessions/ws-backfill/session.json");
    let tmux = minimal_tmux(tmp.path());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener");
    let addr = listener.local_addr().expect("address");
    let server_state_dir = state_dir.clone();
    let server = tokio::spawn(async move {
        let _ = axum::serve(
            listener,
            router(state(&server_state_dir, Some(TOKEN), tmux)),
        )
        .await;
    });

    let mut request = format!("ws://{addr}/sessions/ws-backfill/attach")
        .into_client_request()
        .expect("request");
    request.headers_mut().insert(
        AUTHORIZATION,
        format!("Bearer {TOKEN}").parse().expect("authorization"),
    );
    let (mut socket, _) = connect_async(request).await.expect("connect");
    let _snapshot = tokio::time::timeout(Duration::from_secs(2), socket.next())
        .await
        .expect("snapshot timeout")
        .expect("snapshot frame")
        .expect("snapshot message");
    let before: Value =
        serde_json::from_slice(&std::fs::read(&record_path).expect("record before subscription"))
            .expect("record json");
    assert!(before.get("provider_resume").is_none());

    socket
        .send(ClientMessage::Text(
            json!({"subscribe":[PROVIDER_PROMPT_CAPABILITY]})
                .to_string()
                .into(),
        ))
        .await
        .expect("subscribe");
    let capability = tokio::time::timeout(Duration::from_secs(2), socket.next())
        .await
        .expect("capability timeout")
        .expect("capability frame")
        .expect("capability message");
    let ClientMessage::Text(capability) = capability else {
        panic!("capability must be a text frame");
    };
    let capability: Value = serde_json::from_str(capability.as_str()).expect("capability json");
    assert_eq!(capability["supported"], false);
    let after: Value =
        serde_json::from_slice(&std::fs::read(&record_path).expect("record after subscription"))
            .expect("record json");
    assert!(after.get("provider_resume").is_none());

    OpenOptions::new()
        .append(true)
        .open(&transcript)
        .expect("open unrelated transcript")
        .write_all(
            format!(
                "{}\n",
                json!({
                    "timestamp":"2099-01-01T00:00:00Z",
                    "type":"event_msg",
                    "payload":{
                        "type":"user_message",
                        "message":"unrelated same-cwd prompt"
                    }
                })
            )
            .as_bytes(),
        )
        .expect("append unrelated prompt");
    assert!(
        tokio::time::timeout(Duration::from_millis(250), socket.next())
            .await
            .is_err(),
        "an unrelated same-cwd transcript must not emit prompt events"
    );
    let _ = socket.close(None).await;
    server.abort();
}

/// #2027: a reminder typed into a busy Claude turn is logged as a
/// `queue-operation` enqueue. That is the submission; reading it as unknown
/// requeues the reminder and types it again every few seconds.
#[test]
fn terminal_notification_observes_a_claude_queue_enqueue() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let transcript = tmp.path().join("claude.jsonl");
    std::fs::write(&transcript, "").expect("baseline");
    let tail = ProviderPromptTail::open_path(
        ProviderKind::Claude,
        "provider-beta",
        transcript.clone(),
        Duration::ZERO,
    )
    .expect("tail");
    let prompt = "Coordination mailbox has unread messages; run it.";
    std::fs::write(
        &transcript,
        format!(
            "{}\n",
            json!({
                "type": "queue-operation",
                "operation": "enqueue",
                "timestamp": "2099-01-01T00:00:00Z",
                "sessionId": "provider-beta",
                "content": prompt
            })
        ),
    )
    .expect("append enqueue");
    assert!(observe_terminal_coordination_prompt(tail, prompt));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn provider_prompt_loop_drops_events_when_control_queue_is_saturated() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let transcript = tmp.path().join("codex.jsonl");
    std::fs::write(&transcript, "").expect("baseline");
    let tail = ProviderPromptTail::open_path(
        ProviderKind::Codex,
        "codex-id",
        transcript.clone(),
        Duration::ZERO,
    )
    .expect("tail");
    let (control_tx, mut control_rx) = mpsc::channel(1);
    control_tx
        .send(Message::Text("capability".into()))
        .await
        .expect("fill queue");
    let task = tokio::spawn(provider_prompt_loop(tail, control_tx));
    OpenOptions::new()
        .append(true)
        .open(&transcript)
        .expect("open transcript")
        .write_all(
            format!(
                "{}\n",
                json!({
                    "timestamp":"2099-01-01T00:00:00Z",
                    "type":"event_msg",
                    "payload":{"type":"user_message","message":"dropped prompt"}
                })
            )
            .as_bytes(),
        )
        .expect("append prompt");
    tokio::time::sleep(Duration::from_millis(250)).await;
    assert_eq!(
        control_rx.recv().await,
        Some(Message::Text("capability".into()))
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(150), control_rx.recv())
            .await
            .is_err(),
        "advisory event should be dropped while the queue is full"
    );
    task.abort();
}

#[tokio::test]
async fn provider_prompt_loop_stops_when_control_receiver_closes() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let transcript = tmp.path().join("codex.jsonl");
    std::fs::write(&transcript, "").expect("baseline");
    let tail =
        ProviderPromptTail::open_path(ProviderKind::Codex, "codex-id", transcript, Duration::ZERO)
            .expect("tail");
    let (control_tx, control_rx) = mpsc::channel(1);
    let task = tokio::spawn(provider_prompt_loop(tail, control_tx));
    drop(control_rx);

    tokio::time::timeout(Duration::from_millis(250), task)
        .await
        .expect("provider watcher must stop when its writer queue closes")
        .expect("provider watcher task");
}

fn write_automatic_activity(
    state_dir: &Path,
    id: &str,
    revision: u64,
    current: Option<&str>,
    last: Option<&str>,
) {
    let runtime_id = format!("launch-{id}");
    let activity_path = state_dir.join(format!("sessions/{id}/activity.json"));
    std::fs::write(
        activity_path,
        serde_json::to_vec_pretty(&json!({
            "schema_version":"agent-session.activity.v1",
            "runtime_id":runtime_id,
            "runtime_generation":1,
            "state":{
                "schema_version":"agent-session.turn-state.v1",
                "phase":if current.is_some() { "working" } else { "waiting" },
                "phase_changed_at":"2026-09-07T00:00:00Z",
                "revision":revision,
                "source":{"kind":"provider_hook","provider":"codex","confidence":"authoritative"},
                "current_turn":current.map(|turn_id| json!({
                    "provider_turn_id":turn_id,
                    "started_at":"2026-09-07T00:00:00Z"
                })),
                "last_turn":last.map(|turn_id| json!({
                    "provider_turn_id":turn_id,
                    "completed_at":"2026-09-07T00:00:01Z",
                    "outcome":"completed"
                }))
            }
        }))
        .unwrap(),
    )
    .unwrap();
}

async fn wait_for_test_path(path: &Path) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while !path.exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("test provider did not reach its barrier");
}

async fn wait_for_retitle_v3_operation(app: &Router, id: &str, operation_hash: &str) -> Value {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let (status, body) = call(
                app.clone(),
                get_auth(
                    &format!("/sessions/{id}/retitle-v3/operations/{operation_hash}"),
                    Some(TOKEN),
                ),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "body={body}");
            if body["data"]["retitle"]["status"] == "terminal" {
                return body;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("retitle v3 operation did not reach terminal state")
}

pub(super) fn state(state_dir: &Path, token: Option<&str>, tmux_bin: PathBuf) -> Arc<ServeState> {
    state_with_activity_broker(
        state_dir,
        token,
        tmux_bin,
        ActivityBroker::for_test(MACHINE),
    )
}

fn state_with_activity_broker(
    state_dir: &Path,
    token: Option<&str>,
    tmux_bin: PathBuf,
    activity_broker: Arc<ActivityBroker>,
) -> Arc<ServeState> {
    state_with_dependencies(
        state_dir,
        token,
        tmux_bin,
        activity_broker,
        default_session_collector(),
    )
}

fn state_with_session_collector(
    state_dir: &Path,
    token: Option<&str>,
    tmux_bin: PathBuf,
    session_collector: SessionCollector,
) -> Arc<ServeState> {
    state_with_dependencies(
        state_dir,
        token,
        tmux_bin,
        ActivityBroker::for_test(MACHINE),
        session_collector,
    )
}

fn state_with_dependencies(
    state_dir: &Path,
    token: Option<&str>,
    tmux_bin: PathBuf,
    activity_broker: Arc<ActivityBroker>,
    session_collector: SessionCollector,
) -> Arc<ServeState> {
    let launch_profiles = AgentLaunchProfiles::default();
    let history_catalog = Arc::new(HistoryCatalog::new(
        launch_profiles.history_sources(),
        provider_history::archive_root(state_dir),
        provider_history::star_root(state_dir),
    ));
    Arc::new(ServeState {
        context: CliContext {
            state_dir: state_dir.to_path_buf(),
            host: None,
        },
        machine: MACHINE.to_string(),
        token: token.map(str::to_string),
        board: false,
        federation: None,
        mailbox_services_file: None,
        mailbox_service_workers: Arc::new(tokio::sync::Semaphore::new(16)),
        max_attachment_bytes: DEFAULT_MAX_ATTACHMENT_BYTES,
        tmux_bin,
        attach_brokers: AttachBrokerRegistry::default(),
        provider_prompt_discovery: Arc::new(ProviderPromptDiscoveryRegistry::default()),
        activity_broker,
        codex_controls: Arc::new(StdMutex::new(HashMap::new())),
        account_switches: AccountSwitchRegistry::default(),
        session_collector,
        launch_profiles,
        history_catalog,
        retitle: Arc::new(crate::retitle::RetitleService::from_environment()),
        retitle_v3_manual_waiters: Arc::new(StdMutex::new(HashMap::new())),
        retitle_v3_continuations: Arc::new(StdMutex::new(HashSet::new())),
        managed_history_titles: Arc::new(StdMutex::new(ManagedHistoryTitleCache::default())),
        coordination_wait_workers: Arc::new(tokio::sync::Semaphore::new(
            COORDINATION_WAIT_WORKER_LIMIT,
        )),
        coordination_notification_wake: Arc::new(tokio::sync::Notify::new()),
        remote_outbox_wake: Arc::new(tokio::sync::Notify::new()),
        provider_usage: Arc::new(crate::usage::UsageService::from_environment()),
        board_programs: Arc::new(crate::board::ProgramCache::from_environment()),
    })
}

#[tokio::test]
async fn clipboard_route_authenticates_and_rejects_invalid_input_before_provider_access() {
    let lock = nils_test_support::GlobalStateLock::new();
    let _config = nils_test_support::EnvGuard::set(&lock, "AGENT_SESSION_RETITLE_CONFIG", "");
    let tmp = tempfile::TempDir::new().unwrap();
    let state = state(tmp.path(), Some(TOKEN), PathBuf::from("tmux"));
    let uri = "/clipboard/unwrap/v1";
    let (status, _) = call(
        router(state.clone()),
        post_json(uri, None, json!({"selection":"echo hi"})),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    for selection in ["   ", "a\0b", "x\u{7f}y"] {
        let (status, body) = call(
            router(state.clone()),
            post_json(uri, Some(TOKEN), json!({"selection": selection})),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"]["code"], "invalid-clipboard-selection");
    }
    let (status, body) = call(
        router(state.clone()),
        post_json(uri, Some(TOKEN), json!({"selection": "x".repeat(4097)})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "invalid-clipboard-selection");
    let (status, body) = call(
        router(state),
        post_json(
            uri,
            Some(TOKEN),
            json!({"selection": "https://example.test/"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["error"]["code"], "retitle-provider-not-configured");
}

#[tokio::test]
async fn coordination_json_routes_share_the_versioned_error_envelope() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let state = state(tmp.path(), Some(TOKEN), minimal_tmux(tmp.path()));
    for path in [
        "/sessions/example/work-context/check/v1",
        "/coordination/work-context/check/v1",
        "/sessions/example/work-context/claim/v1",
        "/sessions/example/work-context/renew/v1",
        "/sessions/example/work-context/release/v1",
        "/sessions/example/work-context/admit/v1",
        "/sessions/example/work-context/complete/v1",
        "/sessions/example/work-context/reconcile/v1",
        "/sessions/example/broker/adopt/v1",
        "/sessions/example/broker/adopt/v2",
        "/sessions/example/broker/reconcile/v1",
        "/sessions/example/broker/reconcile/v2",
        "/sessions/example/activity/provider-turn/operator-reconcile/v1",
        "/sessions/example/messages/v1",
        "/sessions/example/messages/message/ack/v1",
        "/sessions/example/messages/message/reply/v1",
    ] {
        let request = Request::builder()
            .method("POST")
            .uri(path)
            .header(AUTHORIZATION, format!("Bearer {TOKEN}"))
            .header("content-type", "application/json")
            .body(Body::from("{"))
            .expect("request");
        let (status, body) = call(router(state.clone()), request).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "path={path} body={body}");
        assert_eq!(body["ok"], false, "path={path} body={body}");
        assert_eq!(
            body["error"]["code"], "invalid-json-body",
            "path={path} body={body}"
        );
    }
}

#[tokio::test]
async fn provider_turn_reconcile_body_is_structurally_strict_and_semantically_versioned() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let state = state(tmp.path(), Some(TOKEN), minimal_tmux(tmp.path()));
    let base = json!({
        "schema_version": "agent-session.operator-provider-turn-reconcile-request.v1",
        "expected_session_incarnation": "runtime",
        "expected_runtime_launch_id": "runtime",
        "expected_runtime_generation": 1,
        "if_activity_revision": 2,
        "expected_provider_turn_id": "provider-turn",
        "reason": "authoritative-completion-signal-missing",
        "attest_inactive": true,
        "confirmed": true,
        "idempotency_key": "operator-provider-turn-strict-body"
    });
    let mut unknown = base.clone();
    unknown["unknown_field"] = json!(true);
    let mut missing = base.clone();
    missing
        .as_object_mut()
        .expect("body object")
        .remove("expected_provider_turn_id");
    for body in [unknown, missing] {
        let request = Request::builder()
            .method("POST")
            .uri("/sessions/example/activity/provider-turn/operator-reconcile/v1")
            .header(AUTHORIZATION, format!("Bearer {TOKEN}"))
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).expect("body")))
            .expect("request");
        let (status, rejected) = call(router(state.clone()), request).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{rejected}");
        assert_eq!(rejected["error"]["code"], "invalid-json-body");
    }

    let mut unsupported_schema = base.clone();
    unsupported_schema["schema_version"] =
        json!("agent-session.operator-provider-turn-reconcile-request.v2");
    let mut unsupported_reason = base;
    unsupported_reason["reason"] = json!("operator-guessed-completion");
    for body in [unsupported_schema, unsupported_reason] {
        let request = Request::builder()
            .method("POST")
            .uri("/sessions/example/activity/provider-turn/operator-reconcile/v1")
            .header(AUTHORIZATION, format!("Bearer {TOKEN}"))
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).expect("body")))
            .expect("request");
        let (status, rejected) = call(router(state.clone()), request).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{rejected}");
        assert_eq!(
            rejected["error"]["code"],
            "invalid-operator-provider-turn-reconcile-request"
        );
    }
    assert!(!tmp.path().join("coordination/registry.json").exists());
    assert!(!tmp.path().join("sessions/example").exists());
}

#[tokio::test]
async fn coordination_query_routes_share_the_versioned_error_envelope() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let state = state(tmp.path(), Some(TOKEN), minimal_tmux(tmp.path()));
    for path in [
        "/sessions/example/messages/v1?limit=not-a-number",
        "/sessions/example/messages/message/wait/v1?if_revision=not-a-number&timeout=1s",
    ] {
        let (status, body) = call(
            router(state.clone()),
            get_coordination(path, "private-capability"),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "path={path} body={body}");
        assert_eq!(body["ok"], false, "path={path} body={body}");
        assert_eq!(
            body["error"]["code"], "invalid-query",
            "path={path} body={body}"
        );
    }
}

fn live_pane_stub_prelude(dir: &Path) -> String {
    let start_time_capture = if cfg!(target_os = "linux") {
        r#"stat = open("/proc/self/stat", encoding="utf-8").read()
start_time = stat[stat.rfind(") ") + 2:].split()[19]"#
    } else {
        r#"start_time = "0""#
    };
    format!(
        r#"NILS_TEST_PANE_PARENT={}
start_live_pane() {{
  if ! command -v python3 >/dev/null 2>&1; then
    printf '%s\n' 'agent-session live-pane test fixture requires python3' >&2
    return 127
  fi
  ready="$NILS_TEST_PANE_PARENT/.pane-ready-$$"
  rm -f "$ready"
  NILS_TEST_PANE_PARENT="$NILS_TEST_PANE_PARENT" NILS_TEST_PANE_READY="$ready" NILS_TEST_PANE_LIFETIME_MS="${{NILS_TEST_PANE_LIFETIME_MS:-30000}}" python3 -c '
import os
import time
os.setsid()
ready = os.environ["NILS_TEST_PANE_READY"]
temporary = ready + ".tmp." + str(os.getpid())
{start_time_capture}
with open(temporary, "x", encoding="utf-8") as handle:
    handle.write(str(os.getpid()) + " " + start_time + "\n")
os.replace(temporary, ready)
deadline = time.monotonic() + int(os.environ["NILS_TEST_PANE_LIFETIME_MS"]) / 1000
parent = os.environ["NILS_TEST_PANE_PARENT"]
while os.path.isdir(parent) and time.monotonic() < deadline:
    time.sleep(0.02)
' >/dev/null 2>&1 &
  launcher=$!
  attempts=0
  while [ ! -s "$ready" ]; do
    attempts=$((attempts + 1))
    if [ "$attempts" -ge 200 ] || ! kill -0 "$launcher" 2>/dev/null; then
      kill "$launcher" 2>/dev/null || true
      wait "$launcher" 2>/dev/null || true
      rm -f "$ready"
      return 1
    fi
    sleep 0.01
  done
  set -- $(cat "$ready")
  NILS_TEST_PANE_PID=$1
  NILS_TEST_PANE_START_TIME=$2
  rm -f "$ready"
  printf '%s %s\n' "$NILS_TEST_PANE_PID" "$NILS_TEST_PANE_START_TIME" > "$NILS_TEST_PANE_PARENT/.pane-identity"
  [ -n "$NILS_TEST_PANE_PID" ] && [ -n "$NILS_TEST_PANE_START_TIME" ] &&
    kill -0 "$NILS_TEST_PANE_PID" 2>/dev/null
}}
"#,
        shell_words::quote(&dir.to_string_lossy()),
        start_time_capture = start_time_capture,
    )
}

fn minimal_tmux(dir: &Path) -> PathBuf {
    let bin = dir.join("tmux");
    let pane_stub = live_pane_stub_prelude(dir);
    std::fs::write(
            &bin,
            format!("#!/usr/bin/env sh\n{pane_stub}\ncase \"$1\" in\n  has-session) exit 0 ;;\n  new-session) heartbeat=''; slot=0; for arg in \"$@\"; do if [ \"$slot\" = 2 ]; then incarnation=\"$arg\"; break; elif [ \"$slot\" = 1 ]; then slot=2; else case \"$arg\" in */coordination/heartbeat) heartbeat=\"$arg\"; slot=1 ;; esac; fi; done; if [ -n \"$heartbeat\" ] && [ -n \"$incarnation\" ]; then mkdir -p \"$(dirname \"$heartbeat\")\"; printf '%s:%s\\n' \"$incarnation\" \"$(date +%s)\" > \"$heartbeat\"; chmod 600 \"$heartbeat\"; fi; start_live_pane || exit 1; printf '%s\\t%s\\t%s\\n' '$77' '%77' \"$NILS_TEST_PANE_PID\"; exit 0 ;;\n  display-message) set -- $(cat \"$NILS_TEST_PANE_PARENT/.pane-identity\"); printf '%s\\t%s\\t%s\\n' '$77' '%77' \"$1\"; exit 0 ;;\n  capture-pane) printf 'pane\\n'; exit 0 ;;\n  show-buffer) printf 'buffered selection\\n'; exit 0 ;;\n  *) exit 0 ;;\nesac\n"),
        )
        .unwrap();
    let mut perms = std::fs::metadata(&bin).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&bin, perms).unwrap();
    bin
}

#[cfg(target_os = "linux")]
#[test]
fn live_pane_stub_publishes_private_session_identity_and_exits_with_fixture() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let script = tmp.path().join("live-pane-stub");
    let prelude = live_pane_stub_prelude(tmp.path());
    std::fs::write(
            &script,
            format!(
                "#!/usr/bin/env sh\n{prelude}\nstart_live_pane || exit 1\nprintf '%s %s\\n' \"$NILS_TEST_PANE_PID\" \"$NILS_TEST_PANE_START_TIME\"\n"
            ),
        )
        .expect("stub script");
    let mut permissions = std::fs::metadata(&script).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&script, permissions).unwrap();
    let output = ProcessCommand::new(&script).output().expect("run stub");
    assert!(
        output.status.success(),
        "stub failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let fields = String::from_utf8(output.stdout)
        .expect("utf8 output")
        .split_whitespace()
        .map(str::to_string)
        .collect::<Vec<_>>();
    assert_eq!(fields.len(), 2);
    let pid = fields[0].parse::<libc::pid_t>().expect("pane pid");
    let expected_start_time = fields[1].parse::<u64>().expect("pane start time");
    assert_eq!(unsafe { libc::getpgid(pid) }, pid);
    assert_eq!(unsafe { libc::getsid(pid) }, pid);
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).expect("pane stat");
    let observed_start_time = stat
        .rsplit_once(") ")
        .expect("stat command boundary")
        .1
        .split_whitespace()
        .nth(19)
        .expect("stat start-time field")
        .parse::<u64>()
        .expect("stat start time");
    assert_eq!(observed_start_time, expected_start_time);

    drop(tmp);
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while Path::new(&format!("/proc/{pid}")).exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        !Path::new(&format!("/proc/{pid}")).exists(),
        "the live pane stub must exit when its TempDir-owned sentinel disappears"
    );
}

fn failing_delete_tmux(
    dir: &Path,
    state_dir: &Path,
    id: &str,
    pane_pid: u32,
    calls: &Path,
) -> PathBuf {
    let bin = dir.join("failing-delete-tmux");
    std::fs::write(
        &bin,
        format!(
            r#"#!/usr/bin/env sh
printf '%s\n' "$*" >> {calls}
case "$1" in
  display-message) printf '$88\t%%88\t{pane_pid}\n'; exit 0 ;;
  show-environment)
    case "$4" in
      AGENT_SESSION_ID) printf 'AGENT_SESSION_ID=%s\n' {id} ;;
      AGENT_SESSION_STATE_DIR) printf 'AGENT_SESSION_STATE_DIR=%s\n' {state_dir} ;;
      AGENT_SESSION_RUNTIME_ID) printf 'AGENT_SESSION_RUNTIME_ID=%s\n' {runtime_id} ;;
    esac
    exit 0 ;;
  kill-session) exit 42 ;;
  has-session) exit 0 ;;
  *) exit 42 ;;
esac
"#,
            id = shell_words::quote(id),
            state_dir = shell_words::quote(&state_dir.to_string_lossy()),
            runtime_id = shell_words::quote(&format!("launch-{id}")),
            calls = shell_words::quote(&calls.to_string_lossy()),
        ),
    )
    .unwrap();
    let mut perms = std::fs::metadata(&bin).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&bin, perms).unwrap();
    bin
}

fn seed_session(state_dir: &Path, id: &str, agent: &str, tmux_session: &str) {
    let dir = state_dir.join("sessions").join(id);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
            dir.join("session.json"),
            format!(
                r#"{{"schema_version":"agent-session.session.v1","id":"{id}","agent":"{agent}","mode":"interactive","title":null,"cwd":"/tmp","tmux_session":"{tmux_session}","prompt_file":null,"log_file":null,"created_at":"2000-01-01T00:00:00Z","updated_at":"2000-01-01T00:00:00Z"}}"#
            ),
        )
        .unwrap();
}

fn seed_session_with_runtime(state_dir: &Path, id: &str, agent: &str, tmux_session: &str) {
    seed_session(state_dir, id, agent, tmux_session);
    let record_path = state_dir.join("sessions").join(id).join("session.json");
    let mut record: Value =
        serde_json::from_str(&std::fs::read_to_string(&record_path).unwrap()).unwrap();
    record["runtime"] = json!({
        "kind": "tmux",
        "tmux_session": tmux_session,
        "generation": 1,
        "started_at": "2000-01-01T00:00:00Z",
        "launch_id": format!("launch-{id}")
    });
    std::fs::write(record_path, serde_json::to_vec_pretty(&record).unwrap()).unwrap();
}

fn seed_codex_app_server_session(state_dir: &Path, id: &str) -> String {
    seed_session_with_runtime(state_dir, id, "codex", &format!("hs-codex-{id}"));
    let record_path = state_dir.join("sessions").join(id).join("session.json");
    let mut record: Value =
        serde_json::from_str(&std::fs::read_to_string(&record_path).unwrap()).unwrap();
    let launch_id = format!("launch-{id}");
    record["runtime"]["kind"] = json!(codex_app_server::RUNTIME_KIND);
    record["runtime"][codex_app_server::PROTOCOL_KEY] = json!(codex_app_server::PROTOCOL_VERSION);
    for (key, suffix) in [
        (codex_app_server::SOCKET_KEY, "sock"),
        (codex_app_server::PROXY_KEY, "proxy"),
        (codex_app_server::THREAD_HANDOFF_KEY, "thread"),
        (codex_app_server::THREAD_ATTACHED_KEY, "attached"),
    ] {
        record["runtime"][key] = json!(state_dir.join(format!("{id}.{suffix}")));
    }
    std::fs::write(record_path, serde_json::to_vec_pretty(&record).unwrap()).unwrap();
    launch_id
}

#[test]
fn reconnect_fence_accepts_historical_binding_without_tmux_server() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let state_dir = tmp.path().join("state");
    seed_codex_app_server_session(&state_dir, "historical-binding");
    let record_path = state_dir.join("sessions/historical-binding/session.json");
    let mut record: Value = serde_json::from_slice(&std::fs::read(&record_path).unwrap()).unwrap();
    record["codex_account_binding"] = json!({"retained": true});
    std::fs::write(&record_path, serde_json::to_vec_pretty(&record).unwrap()).unwrap();
    let tmux = executable(
        &tmp.path().join("tmux-missing"),
        "#!/bin/sh\nprintf '%s\n' 'error connecting to /tmp/tmux-1000/default (No such file or directory)' >&2\nexit 1\n",
    );
    let context = CliContext {
        state_dir,
        host: None,
    };

    fence_codex_controls_before_listen(&context, &tmux)
        .expect("there are no live controls to fence");
}

#[test]
fn reconnect_fence_keeps_unknown_tmux_failure_unavailable() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let state_dir = tmp.path().join("state");
    seed_codex_app_server_session(&state_dir, "historical-binding");
    let record_path = state_dir.join("sessions/historical-binding/session.json");
    let mut record: Value = serde_json::from_slice(&std::fs::read(&record_path).unwrap()).unwrap();
    record["codex_account_binding"] = json!({"retained": true});
    std::fs::write(&record_path, serde_json::to_vec_pretty(&record).unwrap()).unwrap();
    let tmux = executable(
        &tmp.path().join("tmux-failed"),
        "#!/bin/sh\nprintf '%s\n' 'permission denied' >&2\nexit 1\n",
    );
    let context = CliContext {
        state_dir,
        host: None,
    };

    let error = fence_codex_controls_before_listen(&context, &tmux)
        .expect_err("unknown tmux failure must stay fail-closed");

    assert_eq!(error.code(), "codex-account-reconnect-fence-unavailable");
}

fn seed_resumable_session(
    state_dir: &Path,
    id: &str,
    agent: &str,
    tmux_session: &str,
    cwd: &Path,
    resume_args: &[&str],
) {
    let dir = state_dir.join("sessions").join(id);
    std::fs::create_dir_all(&dir).unwrap();
    let resume_args = serde_json::to_string(resume_args).unwrap();
    std::fs::write(
        dir.join("session.json"),
        format!(
            r#"{{
  "schema_version": "agent-session.session.v1",
  "id": "{id}",
  "agent": "{agent}",
  "mode": "interactive",
  "title": null,
  "cwd": "{}",
  "tmux_session": "{tmux_session}",
  "prompt_file": null,
  "log_file": null,
  "created_at": "2000-01-01T00:00:00Z",
  "updated_at": "2000-01-01T00:00:00Z",
  "provider_resume": {{
    "provider": "{agent}",
    "session_id": "resume-session-id",
    "captured_at": "2000-01-01T00:00:00Z",
    "capture_method": "fixture",
    "resume_args": {resume_args}
  }},
  "runtime": {{
    "kind": "tmux",
    "tmux_session": "{tmux_session}",
    "generation": 1,
    "started_at": "2000-01-01T00:00:00Z",
    "launch_id": "never-launched-fixture"
  }},
  "tmux_runtime_never_launched": "never-launched-fixture",
  "startup": {{
    "schema_version": "agent-session.startup.v1",
    "state": "failed",
    "stage": "tmux",
    "started_at": "2000-01-01T00:00:00Z",
    "failure_code": "terminal-runtime-create-failed",
    "message": "The terminal runtime could not be created.",
    "occurred_at": "2000-01-01T00:00:01Z",
    "retry_safe": true
  }},
  "agent_args": []
}}"#,
            cwd.to_string_lossy()
        ),
    )
    .unwrap();
}

fn seed_fresh_provider_session(
    state_dir: &Path,
    id: &str,
    agent: &str,
    tmux_session: &str,
    cwd: &Path,
    provider_resume: Option<(&str, &str)>,
) -> String {
    let created_at = jiff::Timestamp::now().to_string();
    let dir = state_dir.join("sessions").join(id);
    std::fs::create_dir_all(&dir).expect("session dir");
    let mut record = json!({
        "schema_version":"agent-session.session.v1",
        "id":id,
        "agent":agent,
        "mode":"interactive",
        "title":null,
        "cwd":cwd.to_string_lossy(),
        "tmux_session":tmux_session,
        "prompt_file":null,
        "log_file":null,
        "created_at":created_at,
        "updated_at":created_at,
        "runtime":{
            "kind":"tmux",
            "tmux_session":tmux_session,
            "generation":1,
            "started_at":created_at,
            "launch_id":format!("launch-{id}")
        },
        "agent_args":[]
    });
    if let Some((session_id, capture_method)) = provider_resume {
        record["provider_resume"] = json!({
            "provider":agent,
            "session_id":session_id,
            "captured_at":created_at,
            "capture_method":capture_method,
            "resume_args":[]
        });
    }
    std::fs::write(
        dir.join("session.json"),
        serde_json::to_vec_pretty(&record).expect("session json"),
    )
    .expect("session record");
    created_at
}

fn persist_provider_resume_identity(
    state_dir: &Path,
    id: &str,
    provider: &str,
    session_id: &str,
    capture_method: &str,
) {
    let path = state_dir.join("sessions").join(id).join("session.json");
    let mut record: Value = serde_json::from_slice(&std::fs::read(&path).expect("session record"))
        .expect("session json");
    record["provider_resume"] = json!({
        "provider":provider,
        "session_id":session_id,
        "captured_at":jiff::Timestamp::now().to_string(),
        "capture_method":capture_method,
        "resume_args":[]
    });
    std::fs::write(
        path,
        serde_json::to_vec_pretty(&record).expect("session json"),
    )
    .expect("persist provider identity");
}

async fn assert_supported_provider_prompt(
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    provider: &str,
) {
    let frame = tokio::time::timeout(Duration::from_secs(3), socket.next())
        .await
        .expect("capability timeout")
        .expect("capability frame")
        .expect("capability message");
    let ClientMessage::Text(frame) = frame else {
        panic!("capability must be a text frame");
    };
    let frame: Value = serde_json::from_str(frame.as_str()).expect("capability json");
    assert_eq!(frame["type"], "capability");
    assert_eq!(frame["capability"], PROVIDER_PROMPT_CAPABILITY);
    assert_eq!(frame["supported"], true);
    assert_eq!(frame["provider"], provider);
}

async fn assert_provider_prompt(
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    provider: &str,
    prompt: &str,
) {
    let frame = tokio::time::timeout(Duration::from_secs(3), socket.next())
        .await
        .expect("prompt timeout")
        .expect("prompt frame")
        .expect("prompt message");
    let ClientMessage::Text(frame) = frame else {
        panic!("prompt must be a text frame");
    };
    let frame: Value = serde_json::from_str(frame.as_str()).expect("prompt json");
    assert_eq!(frame["type"], "prompt_submitted");
    assert_eq!(frame["provider"], provider);
    assert_eq!(frame["text"], prompt);
    assert!(frame["event_id"].is_string());
}

fn add_provider_resume_extra(state_dir: &Path, id: &str) {
    let path = state_dir.join("sessions").join(id).join("session.json");
    let mut record: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    record["provider_resume"]["storage_only"] = json!({ "keep": true });
    std::fs::write(&path, serde_json::to_string(&record).unwrap()).unwrap();
}

fn executable(path: &Path, body: &str) -> PathBuf {
    std::fs::write(path, body).unwrap();
    let mut perms = std::fs::metadata(path).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(path, perms).unwrap();
    path.to_path_buf()
}

/// A Codex v1 broker that lists `accounts` and refuses every other verb.
fn codex_listing_broker(lock: &GlobalStateLock, dir: &Path, accounts: &[&str]) -> EnvGuard {
    let listed = accounts
        .iter()
        .map(|account| json!({ "account": account }))
        .collect::<Vec<_>>();
    let response = json!({
        "schema_version": crate::codex_account::BROKER_SCHEMA_VERSION,
        "accounts": listed,
        "selection_strategies": ["current_default", "default_with_capacity"],
    });
    let broker = executable(
        &dir.join("codex-listing-broker"),
        &format!(
            "#!/usr/bin/env sh\n[ \"$1\" = list ] || exit 1\nprintf '%s\\n' {}\n",
            shell_words::quote(&response.to_string())
        ),
    );
    let argv = serde_json::to_string(&vec![broker.to_string_lossy().into_owned()]).unwrap();
    EnvGuard::set(lock, "AGENT_SESSION_CODEX_ACCOUNT_BROKER", &argv)
}

mod startup_fixtures;
use startup_fixtures::resume_tmux;

fn fake_agent(dir: &Path, name: &str) -> PathBuf {
    executable(
        &dir.join(name),
        "#!/usr/bin/env sh\nprintf 'fake agent started\\n'\n",
    )
}

fn init_git_remote(repo: &Path, remote: &str) {
    std::fs::create_dir_all(repo).unwrap();
    let init = ProcessCommand::new("git")
        .arg("-C")
        .arg(repo)
        .arg("init")
        .output()
        .unwrap();
    assert!(
        init.status.success(),
        "git init failed: {}",
        String::from_utf8_lossy(&init.stderr)
    );
    let add_remote = ProcessCommand::new("git")
        .arg("-C")
        .arg(repo)
        .args(["remote", "add", "origin", remote])
        .output()
        .unwrap();
    assert!(
        add_remote.status.success(),
        "git remote add failed: {}",
        String::from_utf8_lossy(&add_remote.stderr)
    );
}

#[test]
fn normalize_claude_usage_preserves_reset_aliases() {
    let provider = normalize_claude_usage(UsageHelperOutput {
            status_code: Some(0),
            stdout: br#"{
  "schema_version": "claude-cli.usage.v1",
  "command": "usage",
  "ok": true,
  "result": {
    "source": "cli",
    "stale": false,
    "updated_at": 1783666800,
    "windows": [
      { "label": "5h", "used_percent": 3, "remaining_percent": 97, "resets_at": "2030-01-01T00:00:00Z" },
      { "label": "Weekly", "used_percent": 0, "remaining_percent": 100, "resetsAtEpoch": 1805000000 },
      { "label": "Month", "used_percent": 9, "remaining_percent": 91, "resetsAt": "2030-01-01T00:00:00.950339+00:00" },
      { "label": "Year", "used_percent": 1, "remaining_percent": 99, "resets_at_epoch": "1893456000000" },
      { "label": "Human", "used_percent": 4, "remaining_percent": 96, "resets_at": "Jul 12, 9pm (Asia/Taipei)" },
      { "label": "Invalid", "used_percent": 2, "remaining_percent": 98, "resets_at": "/Users/terry/.claude/token" }
    ]
  }
}"#
            .to_vec(),
            stderr: Vec::new(),
            timed_out: false,
        });

    assert!(provider.fresh_authoritative);
    let value = serde_json::to_value(provider).unwrap();
    assert_eq!(value["ok"], true);
    assert_eq!(value["windows"][0]["reset_at"], "2030-01-01T00:00:00Z");
    assert_eq!(value["windows"][0]["reset_at_epoch"], 1_893_456_000);
    assert_eq!(value["windows"][1]["reset_at_epoch"], 1_805_000_000);
    assert!(value["windows"][1].get("reset_at").is_none());
    assert_eq!(
        value["windows"][2]["reset_at"],
        "2030-01-01T00:00:00.950339+00:00"
    );
    assert_eq!(value["windows"][2]["reset_at_epoch"], 1_893_456_000);
    assert_eq!(value["windows"][3]["reset_at_epoch"], 1_893_456_000);
    assert_eq!(value["windows"][4]["reset_at"], "Jul 12, 9pm (Asia/Taipei)");
    assert_eq!(value["windows"][4]["reset_at_epoch"], 1_783_861_200);
    assert!(value["windows"][5].get("reset_at").is_none());
    assert!(value["windows"][5].get("reset_at_epoch").is_none());
}

#[test]
fn normalize_claude_usage_preserves_provider_reason_code() {
    let provider = normalize_claude_usage(UsageHelperOutput {
        status_code: Some(0),
        stdout: br#"{
  "schema_version": "claude-cli.usage.v1",
  "command": "usage",
  "ok": true,
  "result": {
    "source": "none",
    "stale": true,
    "windows": [],
    "reason_code": "organization_disabled"
  }
}"#
        .to_vec(),
        stderr: Vec::new(),
        timed_out: false,
    });

    let value = serde_json::to_value(provider).unwrap();
    assert_eq!(value["ok"], false);
    assert_eq!(value["reason_code"], "organization_disabled");
}

#[test]
fn normalize_claude_usage_preserves_reason_with_cached_windows() {
    let provider = normalize_claude_usage(UsageHelperOutput {
        status_code: Some(0),
        stdout: br#"{
  "schema_version": "claude-cli.usage.v1",
  "command": "usage",
  "ok": true,
  "result": {
    "source": "cache",
    "stale": true,
    "reason_code": "organization_disabled",
    "windows": [{"label": "5h", "used_percent": 20, "remaining_percent": 80}]
  }
}"#
        .to_vec(),
        stderr: Vec::new(),
        timed_out: false,
    });

    let value = serde_json::to_value(provider).unwrap();
    assert_eq!(value["ok"], true);
    assert_eq!(value["windows"][0]["used_percent"], 20);
    assert_eq!(value["reason_code"], "organization_disabled");
}

#[test]
fn normalize_claude_usage_never_authorizes_stale_cached_windows() {
    let provider = normalize_claude_usage(UsageHelperOutput {
        status_code: Some(0),
        stdout: br#"{
  "schema_version": "claude-cli.usage.v1",
  "command": "usage",
  "ok": false,
  "result": {
    "source": "cache",
    "stale": true,
    "windows": [{"label": "5h", "used_percent": 20, "remaining_percent": 80}]
  }
}"#
        .to_vec(),
        stderr: Vec::new(),
        timed_out: false,
    });

    assert!(!provider.fresh_authoritative);
    assert_eq!(provider.windows.len(), 1);
}

#[test]
fn normalize_codex_usage_preserves_failure_reason_with_healthy_windows() {
    let provider = normalize_codex_usage(UsageHelperOutput {
        status_code: Some(1),
        stdout: br#"{
  "schema_version": "codex-cli.diag.rate-limits.v1",
  "command": "diag rate-limits",
  "ok": false,
  "results": [
    {
      "provider": "codex",
      "name": "healthy",
      "status": "ok",
      "ok": true,
      "summary": {"non_weekly_remaining": 75}
    },
    {
      "provider": "codex",
      "name": "billing",
      "status": "error",
      "ok": false,
      "reason_code": "billing_past_due"
    }
  ]
}"#
        .to_vec(),
        stderr: Vec::new(),
        timed_out: false,
    });

    let value = serde_json::to_value(provider).unwrap();
    assert_eq!(value["ok"], true);
    assert_eq!(value["windows"][0]["remaining_percent"], 75);
    assert_eq!(value["reason_code"], "billing_past_due");
}

#[test]
fn normalize_codex_usage_preserves_provider_reason_code() {
    let provider = normalize_codex_usage(UsageHelperOutput {
        status_code: Some(1),
        stdout: br#"{
  "schema_version": "codex-cli.diag.rate-limits.v1",
  "command": "diag rate-limits",
  "ok": false,
  "results": [{
    "provider": "codex",
    "name": "alpha",
    "status": "error",
    "ok": false,
    "reason_code": "auth_expired",
    "error": {"code": "request-failed", "message": "request failed"}
  }]
}"#
        .to_vec(),
        stderr: Vec::new(),
        timed_out: false,
    });

    let value = serde_json::to_value(provider).unwrap();
    assert_eq!(value["ok"], false);
    assert_eq!(value["reason_code"], "auth_expired");
}

#[test]
fn normalize_codex_usage_prefers_actionable_reason_after_unknown_result() {
    let provider = normalize_codex_usage(UsageHelperOutput {
        status_code: Some(1),
        stdout: br#"{
  "schema_version": "codex-cli.diag.rate-limits.v1",
  "command": "diag rate-limits",
  "ok": false,
  "results": [
    {
      "provider": "codex",
      "name": "alpha",
      "status": "error",
      "ok": false,
      "reason_code": "unknown"
    },
    {
      "provider": "codex",
      "name": "beta",
      "status": "error",
      "ok": false,
      "reason_code": "billing_past_due"
    }
  ]
}"#
        .to_vec(),
        stderr: Vec::new(),
        timed_out: false,
    });

    let value = serde_json::to_value(provider).unwrap();
    assert_eq!(value["reason_code"], "billing_past_due");
}

#[test]
fn default_usage_timeout_covers_slow_claude_usage_helpers() {
    assert!(
        Duration::from_millis(DEFAULT_USAGE_TIMEOUT_MS) >= Duration::from_secs(30),
        "m4 claude-cli usage can take more than 20s before returning reset-bearing windows"
    );
}

#[test]
fn claude_inner_timeout_leaves_cleanup_slack_and_clamps_overrides() {
    let outer = Duration::from_secs(45);
    assert_eq!(claude_inner_timeout_seconds(outer, None), 40);
    assert_eq!(
        claude_inner_timeout_seconds(Duration::from_millis(45_001), None),
        40
    );
    assert_eq!(
        claude_inner_timeout_seconds(Duration::from_millis(1_001), None),
        1
    );
    assert_eq!(claude_inner_timeout_seconds(outer, Some("9")), 9);
    assert_eq!(claude_inner_timeout_seconds(outer, Some("90")), 40);
    assert_eq!(claude_inner_timeout_seconds(outer, Some("invalid")), 40);
}

#[test]
fn usage_helper_timeout_kills_descendant_process_group() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let helper = tmp.path().join("hanging-helper");
    let pid_file = tmp.path().join("descendant.pid");
    fs::write(
        &helper,
        "#!/usr/bin/env sh\nsleep 60 &\nprintf '%s' \"$!\" > \"$PID_FILE\"\nwait\n",
    )
    .expect("write helper");
    fs::set_permissions(&helper, fs::Permissions::from_mode(0o755)).expect("chmod helper");

    let started_at = Instant::now();
    let output = run_usage_helper(
        helper.to_str().expect("helper path"),
        &[],
        &[("PID_FILE", pid_file.to_str().expect("pid path"))],
        Duration::from_secs(2),
    )
    .expect("run helper");
    assert!(output.timed_out);
    // A surviving descendant holds the output pipes open for its 60 s sleep.
    assert!(started_at.elapsed() < Duration::from_secs(30));

    let pid = fs::read_to_string(&pid_file)
        .expect("descendant pid")
        .parse::<libc::pid_t>()
        .expect("numeric pid");
    let deadline = Instant::now() + Duration::from_secs(10);
    while unsafe { libc::kill(pid, 0) } == 0 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(25));
    }
    assert_eq!(
        unsafe { libc::kill(pid, 0) },
        -1,
        "timed-out helper descendant must not survive"
    );
    assert_eq!(io::Error::last_os_error().raw_os_error(), Some(libc::ESRCH));
}

fn write_codex_session_meta(codex_home: &Path, session_id: &str, cwd: &Path) {
    write_codex_session_meta_file(codex_home, "session.jsonl", session_id, cwd);
}

fn write_codex_session_meta_file(codex_home: &Path, filename: &str, session_id: &str, cwd: &Path) {
    let path = codex_home.join("sessions/2026/07/05").join(filename);
    std::fs::create_dir_all(path.parent().expect("parent")).unwrap();
    std::fs::write(
            path,
            format!(
                r#"{{"timestamp":"2099-01-01T00:00:00Z","type":"session_meta","payload":{{"id":"{session_id}","session_id":"{session_id}","cwd":"{}","source":"cli","timestamp":"2099-01-01T00:00:00Z"}}}}
"#,
                cwd.to_string_lossy()
            ),
        )
        .unwrap();
}

fn write_claude_session_transcript(
    claude_config: &Path,
    project_slug: &str,
    session_id: &str,
    cwd: &Path,
) {
    let path = claude_config
        .join("projects")
        .join(project_slug)
        .join(format!("{session_id}.jsonl"));
    std::fs::create_dir_all(path.parent().expect("parent")).unwrap();
    std::fs::write(
        path,
        format!(
            r#"{{"type":"mode","mode":"normal","sessionId":"{session_id}"}}
{{"type":"user","isSidechain":false,"cwd":"{}","sessionId":"{session_id}"}}
"#,
            cwd.to_string_lossy()
        ),
    )
    .unwrap();
}

fn assert_serve_lifecycle(dir: &Path, id: &str, expected: &[(&str, usize)]) {
    let context = CliContext {
        state_dir: dir.to_path_buf(),
        host: None,
    };
    let journal = crate::lifecycle::read(&context, id, 100).unwrap();
    let rows = journal["records"].as_array().unwrap();
    let operations = [
        "start",
        "import",
        "create",
        "resume",
        "stop",
        "delete",
        "archive",
        "account-switch",
        "history-resume",
    ];
    let rows = rows
        .iter()
        .filter(|r| operations.contains(&r["operation"].as_str().unwrap()))
        .collect::<Vec<_>>();
    assert_eq!(
        rows.len(),
        expected.iter().map(|(_, n)| n).sum::<usize>(),
        "{journal}"
    );
    for (operation, count) in expected {
        let matching = rows
            .iter()
            .filter(|r| r["operation"] == *operation)
            .collect::<Vec<_>>();
        assert_eq!(matching.len(), *count, "{journal}");
        assert!(
            matching.iter().all(|r| r["caller"]["kind"] == "serve"),
            "{journal}"
        );
    }
}

async fn call(app: Router, req: Request<Body>) -> (StatusCode, Value) {
    let resp = app.oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap()
    };
    (status, value)
}

fn get(uri: &str) -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri(uri)
        .body(Body::empty())
        .unwrap()
}

fn get_auth(uri: &str, token: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder().method("GET").uri(uri);
    if let Some(token) = token {
        builder = builder.header("authorization", format!("Bearer {token}"));
    }
    builder.body(Body::empty()).unwrap()
}

fn get_activity_events(last_event_id: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder()
        .method("GET")
        .uri("/activity/events")
        .header("authorization", format!("Bearer {TOKEN}"));
    if let Some(last_event_id) = last_event_id {
        builder = builder.header("last-event-id", last_event_id);
    }
    builder.body(Body::empty()).unwrap()
}

async fn first_activity_sse_frame(state: Arc<ServeState>, last_event_id: Option<&str>) -> String {
    let response = router(state)
        .oneshot(get_activity_events(last_event_id))
        .await
        .expect("activity stream response");
    assert_eq!(response.status(), StatusCode::OK);
    let mut body = response.into_body().into_data_stream();
    let mut bytes = Vec::new();
    for _ in 0..8 {
        let chunk = tokio::time::timeout(Duration::from_millis(250), body.next())
            .await
            .expect("bounded SSE response frame")
            .expect("SSE response data")
            .expect("SSE response chunk");
        bytes.extend_from_slice(&chunk);
        assert!(bytes.len() <= 64 * 1024, "SSE frame exceeded test bound");
        if bytes.windows(2).any(|window| window == b"\n\n") {
            break;
        }
    }
    let frame = String::from_utf8(bytes).expect("UTF-8 SSE frame");
    assert!(frame.ends_with("\n\n"), "incomplete SSE frame: {frame:?}");
    frame
}

fn parse_activity_sse_frame(frame: &str) -> (String, String, Value) {
    let id = frame
        .lines()
        .find_map(|line| line.strip_prefix("id: "))
        .expect("SSE id")
        .to_string();
    let event = frame
        .lines()
        .find_map(|line| line.strip_prefix("event: "))
        .expect("SSE event")
        .to_string();
    let data = frame
        .lines()
        .find_map(|line| line.strip_prefix("data: "))
        .expect("SSE data");
    (
        id,
        event,
        serde_json::from_str(data).expect("SSE JSON data"),
    )
}

fn assert_degraded_reset_survives_consumer_dedupe(
    previous: &ActivityStreamEvent,
    reset: &ActivityStreamEvent,
) {
    let mut consumer_seen = std::collections::HashSet::new();
    assert!(consumer_seen.insert((
        previous.machine.clone(),
        previous.stream_id.clone(),
        previous.sequence,
    )));
    assert!(
        consumer_seen.insert((
            reset.machine.clone(),
            reset.stream_id.clone(),
            reset.sequence,
        )),
        "consumer dedupe discarded the degraded reset"
    );
    assert_eq!(reset.kind, "reset");
    assert_eq!(reset.stream_id, previous.stream_id);
    assert!(reset.sequence > previous.sequence);
}

fn test_stream_turn_state(
    revision: u64,
    phase_changed_at: impl Into<String>,
) -> crate::activity::StreamTurnState {
    let state: crate::activity::TurnState = serde_json::from_value(json!({
        "schema_version":"agent-session.turn-state.v1",
        "phase":"working",
        "phase_changed_at":phase_changed_at.into(),
        "revision":revision,
        "source":{
            "kind":"provider_hook",
            "provider":"codex",
            "confidence":"authoritative"
        }
    }))
    .expect("test turn state");
    crate::activity::stream_projection(&state)
}

fn auth_headers(token: Option<&str>) -> HeaderMap {
    let mut headers = HeaderMap::new();
    if let Some(token) = token {
        headers.insert(AUTHORIZATION, format!("Bearer {token}").parse().unwrap());
    }
    headers
}

fn post_json(uri: &str, token: Option<&str>, body: Value) -> Request<Body> {
    let mut builder = Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json");
    if let Some(token) = token {
        builder = builder.header("authorization", format!("Bearer {token}"));
    }
    builder
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap()
}

fn get_coordination(uri: &str, capability: &str) -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri(uri)
        .header("authorization", format!("Bearer {TOKEN}"))
        .header("x-agent-session-capability", capability)
        .body(Body::empty())
        .unwrap()
}

fn post_coordination(uri: &str, capability: &str, body: Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("authorization", format!("Bearer {TOKEN}"))
        .header("x-agent-session-capability", capability)
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap()
}

fn post_operator_provider_turn(body: &Value) -> Request<Body> {
    post_json(
        "/sessions/alpha/activity/provider-turn/operator-reconcile/v1",
        Some(TOKEN),
        body.clone(),
    )
}

fn patch_json(uri: &str, token: Option<&str>, body: Value) -> Request<Body> {
    let mut builder = Request::builder()
        .method("PATCH")
        .uri(uri)
        .header("content-type", "application/json");
    if let Some(token) = token {
        builder = builder.header("authorization", format!("Bearer {token}"));
    }
    builder
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap()
}

fn put_json(uri: &str, token: Option<&str>, body: Value) -> Request<Body> {
    let mut builder = Request::builder()
        .method("PUT")
        .uri(uri)
        .header("content-type", "application/json");
    if let Some(token) = token {
        builder = builder.header("authorization", format!("Bearer {token}"));
    }
    builder
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap()
}

fn post_bytes(uri: &str, token: Option<&str>, body: &[u8]) -> Request<Body> {
    let mut builder = Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/octet-stream");
    if let Some(token) = token {
        builder = builder.header("authorization", format!("Bearer {token}"));
    }
    builder.body(Body::from(body.to_vec())).unwrap()
}

#[tokio::test]
async fn healthz_reports_machine() {
    let tmp = tempfile::TempDir::new().unwrap();
    let st = state(tmp.path(), Some(TOKEN), PathBuf::from("tmux"));
    let (status, body) = call(router(st.clone()), get("/healthz")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["ok"], true);
    assert_eq!(body["data"]["machine"], MACHINE);
}

/// `machine offline`, `runtime unavailable`, and `session protected` are
/// three distinct operator situations. Collapsing them into one generic
/// offline state is what left Agent Console unable to say anything useful
/// during the `sympoies/nils-cli#1409` incident.
#[tokio::test]
async fn healthz_separates_runtime_and_session_protection_from_machine_reach() {
    let tmp = tempfile::TempDir::new().unwrap();
    let st = state(tmp.path(), Some(TOKEN), PathBuf::from("tmux"));
    let (status, body) = call(router(st.clone()), get("/healthz")).await;
    assert_eq!(status, StatusCode::OK);
    let data = &body["data"];

    // Reaching this handler already answers machine reachability, so
    // `status` keeps its historical meaning for existing consumers.
    assert_eq!(data["status"], "ok");
    assert_eq!(data["health"], "healthy");
    assert_eq!(data["runtime"]["state"], "available");
    assert_eq!(data["runtime"]["coordination"], "absent");
    assert_eq!(data["runtime"]["reasons"], serde_json::json!([]));
    assert_eq!(data["sessions"]["protected"], 0);
    assert!(
        data["runtime"]["binary_version"]
            .as_str()
            .is_some_and(|value| !value.is_empty()),
        "version skew diagnosis needs the serving release: {data}"
    );
    assert!(
        matches!(
            data["runtime"]["executable_state"].as_str(),
            Some("live") | Some("unknown")
        ),
        "a running daemon is not replaced: {data}"
    );
}

/// An unreadable coordination store degrades the runtime without claiming
/// the machine is offline.
#[tokio::test]
async fn healthz_degrades_on_an_unreadable_coordination_store() {
    let tmp = tempfile::TempDir::new().unwrap();
    let coordination = tmp.path().join("coordination");
    std::fs::create_dir_all(&coordination).unwrap();
    let registry = coordination.join("registry.json");
    std::fs::write(&registry, b"{").unwrap();
    std::fs::set_permissions(&registry, std::fs::Permissions::from_mode(0o600)).unwrap();

    let st = state(tmp.path(), Some(TOKEN), PathBuf::from("tmux"));
    let (status, body) = call(router(st.clone()), get("/healthz")).await;
    assert_eq!(status, StatusCode::OK, "the machine is still reachable");
    let data = &body["data"];
    assert_eq!(data["status"], "ok");
    assert_eq!(data["health"], "degraded");
    assert_eq!(data["runtime"]["state"], "degraded");
    assert_eq!(data["runtime"]["coordination"], "coordination-invalid");
    assert_eq!(
        data["runtime"]["reasons"],
        serde_json::json!(["coordination-invalid"])
    );
}

/// A registry from another release generation is recoverable drift, and it
/// must be named as such rather than as corruption.
#[tokio::test]
async fn healthz_names_release_drift_distinctly_from_corruption() {
    let tmp = tempfile::TempDir::new().unwrap();
    let coordination = tmp.path().join("coordination");
    std::fs::create_dir_all(&coordination).unwrap();
    let registry = coordination.join("registry.json");
    std::fs::write(
        &registry,
        serde_json::to_vec(&serde_json::json!({
            "schema_version": "agent-session.coordination-registry.v9",
            "fingerprint_epoch": 1,
            "fingerprint_key": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            "brokers": {},
            "claims": []
        }))
        .unwrap(),
    )
    .unwrap();
    std::fs::set_permissions(&registry, std::fs::Permissions::from_mode(0o600)).unwrap();

    let st = state(tmp.path(), Some(TOKEN), PathBuf::from("tmux"));
    let (_status, body) = call(router(st.clone()), get("/healthz")).await;
    assert_eq!(
        body["data"]["runtime"]["coordination"],
        "runtime-version-skew"
    );
    assert_eq!(body["data"]["health"], "degraded");
}

/// The registry epoch is a candidate screen, not freshness authority. A
/// broker whose registry epoch lags its sidecar write must still be
/// authenticated, or the cheap pre-filter would silently under-report a
/// protected runtime and invite a restart that eats an active claim.
#[tokio::test]
async fn healthz_still_authenticates_a_broker_whose_registry_epoch_lags() {
    let tmp = tempfile::TempDir::new().unwrap();
    let now = jiff::Timestamp::now().as_second();
    let coordination = tmp.path().join("coordination");
    std::fs::create_dir_all(&coordination).unwrap();
    let registry = coordination.join("registry.json");
    std::fs::write(
        &registry,
        serde_json::to_vec(&serde_json::json!({
            "schema_version": "agent-session.coordination-registry.v1",
            "fingerprint_epoch": 1,
            "fingerprint_key": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            "brokers": {
                "lagging": {
                    "session_id": "lagging",
                    "incarnation": "inc-lagging",
                    "state": "ready",
                    // Well behind the sidecar below, but inside the screen.
                    "heartbeat_epoch": now - 120
                }
            },
            "claims": []
        }))
        .unwrap(),
    )
    .unwrap();
    std::fs::set_permissions(&registry, std::fs::Permissions::from_mode(0o600)).unwrap();
    let heartbeat = tmp.path().join("sessions/lagging/coordination");
    std::fs::create_dir_all(&heartbeat).unwrap();
    let heartbeat = heartbeat.join("heartbeat");
    std::fs::write(&heartbeat, format!("inc-lagging:{now}\n")).unwrap();
    std::fs::set_permissions(&heartbeat, std::fs::Permissions::from_mode(0o600)).unwrap();

    let st = state(tmp.path(), Some(TOKEN), PathBuf::from("tmux"));
    let (_status, body) = call(router(st.clone()), get("/healthz")).await;
    assert_eq!(
        body["data"]["sessions"]["protected"], 1,
        "the sidecar remains freshness authority: {}",
        body["data"]
    );
}

/// A live session holding a fresh ready broker is protected: its runtime
/// must not be restarted out from under an active claim.
#[tokio::test]
async fn healthz_counts_protected_sessions() {
    let tmp = tempfile::TempDir::new().unwrap();
    let now = jiff::Timestamp::now().as_second();
    let coordination = tmp.path().join("coordination");
    std::fs::create_dir_all(&coordination).unwrap();
    let registry = coordination.join("registry.json");
    std::fs::write(
        &registry,
        serde_json::to_vec(&serde_json::json!({
            "schema_version": "agent-session.coordination-registry.v1",
            "fingerprint_epoch": 1,
            "fingerprint_key": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            "brokers": {
                "live": {
                    "session_id": "live",
                    "incarnation": "inc-live",
                    "state": "ready",
                    "heartbeat_epoch": now
                },
                "stale": {
                    "session_id": "stale",
                    "incarnation": "inc-stale",
                    "state": "ready",
                    "heartbeat_epoch": 0
                }
            },
            "claims": []
        }))
        .unwrap(),
    )
    .unwrap();
    std::fs::set_permissions(&registry, std::fs::Permissions::from_mode(0o600)).unwrap();
    let heartbeat = tmp.path().join("sessions/live/coordination");
    std::fs::create_dir_all(&heartbeat).unwrap();
    let heartbeat = heartbeat.join("heartbeat");
    std::fs::write(&heartbeat, format!("inc-live:{now}\n")).unwrap();
    std::fs::set_permissions(&heartbeat, std::fs::Permissions::from_mode(0o600)).unwrap();

    let st = state(tmp.path(), Some(TOKEN), PathBuf::from("tmux"));
    let (_status, body) = call(router(st.clone()), get("/healthz")).await;
    assert_eq!(body["data"]["runtime"]["coordination"], "available");
    assert_eq!(
        body["data"]["sessions"]["protected"], 1,
        "only a fresh heartbeat proves a protected runtime: {}",
        body["data"]
    );
}

#[tokio::test]
async fn list_empty_reports_machine_and_open_on_reads() {
    let tmp = tempfile::TempDir::new().unwrap();
    let st = state(tmp.path(), Some(TOKEN), PathBuf::from("tmux"));
    // No Authorization header: reads are open on loopback.
    let (status, body) = call(router(st.clone()), get("/sessions")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["schema_version"], "cli.agent-session.serve.v1");
    assert_eq!(body["data"]["machine"], MACHINE);
    let observed_at = body["data"]["observed_at"]
        .as_str()
        .expect("daemon observation anchor");
    observed_at
        .parse::<jiff::Timestamp>()
        .expect("RFC3339 daemon observation anchor");
    assert_eq!(body["data"]["sessions"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn list_projects_managed_account_handoff_capability_per_session() {
    let tmp = tempfile::TempDir::new().unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let mut managed = test_record("managed", "hs-managed");
    managed.runtime = Some(crate::RuntimeInfo {
        kind: crate::codex_app_server::RUNTIME_KIND.to_string(),
        tmux_session: managed.tmux_session.clone(),
        generation: 1,
        started_at: "2030-01-01T00:00:00Z".to_string(),
        launch_id: "launch-managed".to_string(),
        extra: std::collections::BTreeMap::from([(
            "managed_account_handoff_capability".to_string(),
            json!(crate::codex_app_server::MANAGED_ACCOUNT_HANDOFF_CAPABILITY),
        )]),
    });
    let mut raw = test_record("raw", "hs-raw");
    raw.runtime = Some(crate::RuntimeInfo {
        kind: "tmux".to_string(),
        tmux_session: raw.tmux_session.clone(),
        generation: 1,
        started_at: "2030-01-01T00:00:00Z".to_string(),
        launch_id: "launch-raw".to_string(),
        extra: std::collections::BTreeMap::new(),
    });
    let mut claude = test_record("claude", "hs-claude");
    claude.agent = "claude".to_string();
    claude.runtime = Some(crate::RuntimeInfo {
        kind: "tmux".to_string(),
        tmux_session: claude.tmux_session.clone(),
        generation: 1,
        started_at: "2030-01-01T00:00:00Z".to_string(),
        launch_id: "launch-claude".to_string(),
        extra: std::collections::BTreeMap::new(),
    });
    for record in [&managed, &raw, &claude] {
        fs::create_dir_all(session_dir(&context, &record.id)).unwrap();
        crate::write_session_record(&context, record).unwrap();
    }
    let tmux = tmp.path().join("running-capability-tmux");
    fs::write(
            &tmux,
            "#!/usr/bin/env sh\ncase \"$1\" in\n  list-windows) printf 'hs-managed\\t1\\nhs-raw\\t1\\nhs-claude\\t1\\n'; exit 0 ;;\n  has-session) exit 0 ;;\n  *) exit 0 ;;\nesac\n",
        )
        .unwrap();
    fs::set_permissions(&tmux, fs::Permissions::from_mode(0o755)).unwrap();
    let st = state(&context.state_dir, Some(TOKEN), tmux);

    let (status, body) = call(router(st), get("/sessions")).await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let sessions = body["data"]["sessions"].as_array().unwrap();
    let managed_view = sessions
        .iter()
        .find(|session| session["id"] == "managed")
        .unwrap();
    let raw_view = sessions
        .iter()
        .find(|session| session["id"] == "raw")
        .unwrap();
    let claude_view = sessions
        .iter()
        .find(|session| session["id"] == "claude")
        .unwrap();
    assert_eq!(
        body["data"]["capabilities"]["managed_account_handoff"],
        crate::codex_app_server::MANAGED_ACCOUNT_HANDOFF_CAPABILITY,
        "the global capability reports daemon protocol support, not per-session eligibility"
    );
    assert_eq!(
        managed_view["capabilities"],
        json!([crate::codex_app_server::MANAGED_ACCOUNT_HANDOFF_CAPABILITY]),
        "managed session projection: {managed_view}"
    );
    assert!(
        raw_view
            .as_object()
            .is_some_and(|view| !view.contains_key("capabilities")),
        "raw session falsely advertised managed handoff: {raw_view}"
    );
    assert!(
        claude_view
            .as_object()
            .is_some_and(|view| !view.contains_key("capabilities")),
        "Claude sessions must not advertise the Codex managed handoff capability: {claude_view}"
    );
}

#[tokio::test]
async fn list_omits_managed_account_handoff_capability_for_stopped_runtime() {
    let tmp = tempfile::TempDir::new().unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let mut managed = test_record("stopped-managed", "hs-stopped-managed");
    managed.runtime = Some(crate::RuntimeInfo {
        kind: crate::codex_app_server::RUNTIME_KIND.to_string(),
        tmux_session: managed.tmux_session.clone(),
        generation: 1,
        started_at: "2030-01-01T00:00:00Z".to_string(),
        launch_id: "launch-stopped-managed".to_string(),
        extra: std::collections::BTreeMap::from([(
            "managed_account_handoff_capability".to_string(),
            json!(crate::codex_app_server::MANAGED_ACCOUNT_HANDOFF_CAPABILITY),
        )]),
    });
    fs::create_dir_all(session_dir(&context, &managed.id)).unwrap();
    crate::write_session_record(&context, &managed).unwrap();
    let tmux = tmp.path().join("stopped-tmux");
    fs::write(
        &tmux,
        "#!/usr/bin/env sh\nprintf '%s\\n' \"no server running on fixture\" >&2\nexit 1\n",
    )
    .unwrap();
    fs::set_permissions(&tmux, fs::Permissions::from_mode(0o755)).unwrap();

    let (status, body) = call(
        router(state(&context.state_dir, Some(TOKEN), tmux)),
        get("/sessions"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let managed_view = body["data"]["sessions"]
        .as_array()
        .expect("sessions")
        .iter()
        .find(|session| session["id"] == "stopped-managed")
        .expect("stopped managed session");
    assert_eq!(managed_view["status"], "stopped");
    assert!(
        managed_view
            .as_object()
            .is_some_and(|view| !view.contains_key("capabilities")),
        "stopped runtime must not advertise a live managed-handoff control: {managed_view}"
    );
}

#[tokio::test]
async fn list_observed_at_is_sampled_after_delayed_assembly_and_before_receipt() {
    let tmp = tempfile::TempDir::new().unwrap();
    let collection_finished = Arc::new(StdMutex::new(None));
    let finished = collection_finished.clone();
    let collector: SessionCollector = Arc::new(move |_, _| {
        std::thread::sleep(Duration::from_millis(50));
        *finished.lock().expect("collection timestamp lock") = Some(jiff::Timestamp::now());
        Ok(Vec::new())
    });
    let st =
        state_with_session_collector(tmp.path(), Some(TOKEN), PathBuf::from("tmux"), collector);

    let (status, body) = call(router(st), get("/sessions")).await;
    let received_at = jiff::Timestamp::now();
    assert_eq!(status, StatusCode::OK);
    let observed_at = body["data"]["observed_at"]
        .as_str()
        .unwrap()
        .parse::<jiff::Timestamp>()
        .unwrap();
    let collection_finished = collection_finished
        .lock()
        .expect("collection timestamp lock")
        .expect("collection completed");
    assert!(
        observed_at >= collection_finished,
        "observation preceded collection: observed={observed_at}, finished={collection_finished}"
    );
    assert!(
        observed_at <= received_at,
        "observation followed receipt: observed={observed_at}, receipt={received_at}"
    );
}

#[tokio::test]
async fn activity_stream_requires_auth_and_sets_sse_headers() {
    let tmp = tempfile::TempDir::new().unwrap();
    let st = state(tmp.path(), Some(TOKEN), PathBuf::from("tmux"));

    let (status, body) = call(router(st.clone()), get_auth("/activity/events", None)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"]["code"], "unauthorized");

    let response = router(st)
        .oneshot(get_auth("/activity/events", Some(TOKEN)))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("text/event-stream")
    );
    assert_eq!(
        response.headers()["cache-control"],
        "no-cache, no-transform"
    );
    assert_eq!(response.headers()["x-accel-buffering"], "no");
}

#[tokio::test]
async fn activity_stream_route_frames_cover_cursor_and_reset_contract() {
    let tmp = tempfile::TempDir::new().unwrap();
    let st = state(tmp.path(), Some(TOKEN), PathBuf::from("tmux"));
    let stream_id = st.activity_broker.log.stream_id.clone();

    let (id, event, data) =
        parse_activity_sse_frame(&first_activity_sse_frame(st.clone(), None).await);
    assert_eq!(id, format!("{stream_id}:1"));
    assert_eq!(event, "snapshot");
    assert_eq!(data["schema_version"], ACTIVITY_STREAM_EVENT_SCHEMA_VERSION);
    assert_eq!(data["type"], "snapshot");
    assert_eq!(data["stream_id"], stream_id);
    assert_eq!(data["sequence"], 1);
    assert_eq!(data["machine"], MACHINE);
    assert_eq!(data["sessions"], json!([]));

    st.activity_broker.log.publish_heartbeat();
    st.activity_broker
        .log
        .publish_snapshot(vec![ActivityStreamSession {
            id: "cursor-session".to_string(),
            turn_state: Some(test_stream_turn_state(2, "2026-07-11T00:00:02Z")),
        }]);
    let retained_cursor = format!("{stream_id}:1");
    let (id, event, data) = parse_activity_sse_frame(
        &first_activity_sse_frame(st.clone(), Some(&retained_cursor)).await,
    );
    assert_eq!(id, format!("{stream_id}:2"));
    assert_eq!(event, "heartbeat");
    assert_eq!(data["type"], "heartbeat");
    assert!(data.get("sessions").is_none());

    let retained_after_heartbeat = format!("{stream_id}:2");
    let (id, event, data) = parse_activity_sse_frame(
        &first_activity_sse_frame(st.clone(), Some(&retained_after_heartbeat)).await,
    );
    assert_eq!(id, format!("{stream_id}:3"));
    assert_eq!(event, "snapshot");
    assert_eq!(data["type"], "snapshot");
    assert_eq!(data["sessions"][0]["id"], "cursor-session");

    for (offset, cursor) in [
        "malformed".to_string(),
        format!("{stream_id}:999"),
        "foreign-stream:1".to_string(),
    ]
    .into_iter()
    .enumerate()
    {
        let (id, event, data) =
            parse_activity_sse_frame(&first_activity_sse_frame(st.clone(), Some(&cursor)).await);
        let expected_sequence = 4 + offset as u64;
        assert_eq!(id, format!("{stream_id}:{expected_sequence}"));
        assert_eq!(event, "reset");
        assert_eq!(data["type"], "reset");
        assert_eq!(data["sequence"], expected_sequence);
        assert_eq!(data["sessions"][0]["id"], "cursor-session");
    }

    for _ in 0..ACTIVITY_STREAM_REPLAY_CAPACITY {
        st.activity_broker.log.publish_heartbeat();
    }
    let (id, event, data) = parse_activity_sse_frame(
        &first_activity_sse_frame(st.clone(), Some(&retained_cursor)).await,
    );
    assert_eq!(id, format!("{stream_id}:{}", data["sequence"]));
    assert_eq!(event, "reset");
    assert_eq!(data["type"], "reset");
    assert_eq!(data["sessions"][0]["id"], "cursor-session");
}

#[test]
fn activity_stream_projection_is_metadata_only_and_drops_additive_content() {
    let state: crate::activity::TurnState = serde_json::from_value(json!({
        "schema_version":"agent-session.turn-state.v1",
        "phase":"needs_input",
        "phase_changed_at":"2026-07-11T00:00:00Z",
        "revision":7,
        "source":{
            "kind":"provider_hook",
            "provider":"codex",
            "confidence":"authoritative",
            "config":"must-not-stream"
        },
        "current_turn":{
            "provider_turn_id":"opaque-turn",
            "started_at":"2026-07-11T00:00:00Z",
            "last_progress_at":"2026-07-11T00:00:01Z",
            "attention":{
                "kind":"permission",
                "requested_at":"2026-07-11T00:00:02Z",
                "pending_count":1,
                "tool":"must-not-stream"
            },
            "prompt":"must-not-stream"
        },
        "last_turn":{
            "provider_turn_id":"opaque-prior-turn",
            "started_at":"2026-07-10T23:59:00Z",
            "completed_at":"2026-07-10T23:59:30Z",
            "outcome":"completed",
            "response":"must-not-stream"
        },
        "terminal":"must-not-stream",
        "credential":"must-not-stream"
    }))
    .expect("forward-compatible state");

    let projection = serde_json::to_value(crate::activity::stream_projection(&state)).unwrap();
    assert_eq!(projection["revision"], 7);
    assert_eq!(
        projection["current_turn"]["attention"]["kind"],
        "permission"
    );
    let encoded = projection.to_string();
    for forbidden in [
        "prompt",
        "response",
        "command",
        "tool",
        "terminal",
        "transcript",
        "config",
        "credential",
        "must-not-stream",
    ] {
        assert!(
            !encoded.contains(forbidden),
            "leaked forbidden field: {forbidden}"
        );
    }
}

#[test]
fn activity_stream_multi_session_producer_matches_exact_downstream_fixture() {
    let working: crate::activity::TurnState = serde_json::from_value(json!({
        "schema_version":"agent-session.turn-state.v1",
        "phase":"working",
        "phase_changed_at":"2026-07-11T17:00:00Z",
        "revision":11,
        "source":{
            "kind":"provider_hook",
            "confidence":"authoritative"
        },
        "current_turn":{
            "started_at":"2026-07-11T16:59:00Z"
        }
    }))
    .expect("working fixture state");
    let completed: crate::activity::TurnState = serde_json::from_value(json!({
        "schema_version":"agent-session.turn-state.v1",
        "phase":"waiting",
        "phase_changed_at":"2026-07-11T17:01:00Z",
        "revision":12,
        "source":{
            "kind":"provider_hook",
            "provider":"codex",
            "confidence":"authoritative"
        },
        "last_turn":{
            "completed_at":"2026-07-11T17:01:00Z",
            "outcome":"completed"
        }
    }))
    .expect("completed fixture state");
    let event = ActivityStreamEvent {
        schema_version: ACTIVITY_STREAM_EVENT_SCHEMA_VERSION,
        kind: "snapshot",
        stream_id: "fixture-stream".to_string(),
        sequence: 42,
        machine: "fixture-machine".to_string(),
        observed_at: "2026-07-11T17:01:01Z".to_string(),
        reason: None,
        sessions: Some(vec![
            ActivityStreamSession {
                id: "working-minimal".to_string(),
                turn_state: Some(crate::activity::stream_projection(&working)),
            },
            ActivityStreamSession {
                id: "completed-minimal".to_string(),
                turn_state: Some(crate::activity::stream_projection(&completed)),
            },
            ActivityStreamSession {
                id: "no-activity".to_string(),
                turn_state: None,
            },
        ]),
    };
    let expected: Value = serde_json::from_str(include_str!(
        "../../tests/fixtures/activity/activity-stream-v1-multi-session.json"
    ))
    .expect("canonical Agent Console producer fixture");

    assert_eq!(serde_json::to_value(event).unwrap(), expected);
}

#[tokio::test]
async fn activity_stream_replays_in_order_and_resets_invalid_or_evicted_cursors() {
    let initial = vec![ActivityStreamSession {
        id: "session-1".to_string(),
        turn_state: Some(test_stream_turn_state(1, "2026-07-11T00:00:01Z")),
    }];
    let log = ActivityEventLog::new(MACHINE.to_string(), initial.clone());
    let mut first = log.subscribe(None);
    let snapshot = first.next_event().await.unwrap();
    assert_eq!(snapshot.kind, "snapshot");
    assert_eq!(snapshot.sequence, 1);
    assert_eq!(snapshot.sessions.as_ref(), Some(&initial));

    log.publish_heartbeat();
    log.publish_snapshot(vec![ActivityStreamSession {
        id: "session-1".to_string(),
        turn_state: Some(test_stream_turn_state(2, "2026-07-11T00:00:02Z")),
    }]);
    let mut replay = log.subscribe(Some(&format!("{}:1", log.stream_id)));
    assert_eq!(replay.next_event().await.unwrap().sequence, 2);
    let changed = replay.next_event().await.unwrap();
    assert_eq!(changed.sequence, 3);
    assert_eq!(changed.kind, "snapshot");

    let mut live = log.subscribe(Some(&format!("{}:3", log.stream_id)));
    let mut foreign = log.subscribe(Some("other-stream:3"));
    let reset = foreign.next_event().await.unwrap();
    assert_eq!(reset.kind, "reset");
    assert_eq!(reset.sequence, 4);
    assert_eq!(
        reset.sessions.as_ref().unwrap()[0]
            .turn_state
            .as_ref()
            .unwrap()
            .revision,
        2
    );
    let broadcast_reset = tokio::time::timeout(Duration::from_millis(100), live.next_event())
        .await
        .expect("reconciliation reset must reach existing subscribers")
        .expect("activity stream remains open");
    assert_eq!(broadcast_reset.kind, "reset");
    assert_eq!(broadcast_reset.sequence, reset.sequence);

    for _ in 0..ACTIVITY_STREAM_REPLAY_CAPACITY {
        log.publish_heartbeat();
    }
    let mut fresh = log.subscribe(None);
    let current = fresh.next_event().await.unwrap();
    assert_eq!(current.kind, "snapshot");
    assert_eq!(current.sequence, log.sequence.load(Ordering::SeqCst));
    let mut evicted = log.subscribe(Some(&format!("{}:1", log.stream_id)));
    assert_eq!(evicted.next_event().await.unwrap().kind, "reset");
}

#[tokio::test]
async fn activity_stream_replay_bytes_are_bounded_and_high_cardinality_eviction_resets() {
    let log = ActivityEventLog::new(MACHINE.to_string(), Vec::new());
    let previous_sequence = log.sequence.load(Ordering::SeqCst);
    let sessions = (0..128)
        .map(|index| ActivityStreamSession {
            id: format!("session-{index:03}-{}", "x".repeat(128)),
            turn_state: Some(test_stream_turn_state(index, "x".repeat(8192))),
        })
        .collect();
    log.publish_snapshot(sessions);

    {
        let state = log.state.lock().expect("activity event state lock");
        assert!(
            state.history_bytes <= ACTIVITY_STREAM_REPLAY_BYTE_CAPACITY,
            "replay bytes exceeded cap: {} > {}",
            state.history_bytes,
            ACTIVITY_STREAM_REPLAY_BYTE_CAPACITY
        );
    }

    let mut replay = log.subscribe(Some(&format!("{}:{previous_sequence}", log.stream_id)));
    let reset = replay.next_event().await.unwrap();
    assert_eq!(reset.kind, "reset");
    assert_eq!(reset.sequence, log.sequence.load(Ordering::SeqCst));
    assert_eq!(reset.reason, Some(ACTIVITY_STREAM_OVERSIZED_REASON));
    assert!(reset.sessions.is_none());
}

#[tokio::test]
async fn activity_stream_backpressure_never_blocks_producers_and_lag_resets() {
    let log = ActivityEventLog::new(MACHINE.to_string(), Vec::new());
    let cursor = format!("{}:1", log.stream_id);
    let mut slow = log.subscribe(Some(&cursor));
    for _ in 0..(ACTIVITY_STREAM_BROADCAST_CAPACITY + 4) {
        log.publish_heartbeat();
    }
    let reset = tokio::time::timeout(Duration::from_millis(100), slow.next_event())
        .await
        .expect("lagged subscriber reset")
        .expect("lagged subscriber event");
    assert_eq!(reset.kind, "reset");
    assert_eq!(reset.sequence, log.sequence.load(Ordering::SeqCst));
    assert!(reset.sessions.is_some());
}

#[tokio::test]
async fn activity_stream_oversized_max_fanout_and_lag_reconcile_with_bounded_frames() {
    let broker = ActivityBroker::for_test(MACHINE);
    let cursor = format!("{}:1", broker.log.stream_id);
    let mut subscribers = (0..ACTIVITY_STREAM_SUBSCRIBER_CAPACITY)
        .map(|_| {
            broker
                .subscribe(Some(&cursor))
                .expect("admitted subscriber")
        })
        .collect::<Vec<_>>();
    assert!(matches!(
        broker.subscribe(Some(&cursor)),
        Err(ActivitySubscribeError::Capacity)
    ));
    let sessions = (0..128)
        .map(|index| ActivityStreamSession {
            id: format!("oversized-{index:03}-{}", "x".repeat(128)),
            turn_state: Some(test_stream_turn_state(index, "x".repeat(8192))),
        })
        .collect();

    let encode_count = broker.log.encoder.count();
    broker.log.publish_snapshot(sessions);
    let mut slow = subscribers.pop().expect("lagging subscriber");
    let first = subscribers[0]
        .next_event()
        .await
        .expect("oversized reconciliation frame");
    assert_eq!(first.kind, "reset");
    assert_eq!(first.reason, Some(ACTIVITY_STREAM_OVERSIZED_REASON));
    assert!(first.sessions.is_none());
    assert!(first.replay_bytes() <= ACTIVITY_STREAM_REPLAY_BYTE_CAPACITY);
    let (_, _, wire) =
        parse_activity_sse_frame(std::str::from_utf8(first.sse.as_ref()).expect("UTF-8 SSE frame"));
    assert_eq!(wire["type"], "reset");
    assert_eq!(wire["reason"], ACTIVITY_STREAM_OVERSIZED_REASON);
    assert!(wire.get("sessions").is_none());
    assert_eq!(
        broker.log.encoder.count(),
        encode_count + 2,
        "oversized payload and its bounded reset each serialize once"
    );
    for subscriber in &mut subscribers[1..] {
        let observed = subscriber
            .next_event()
            .await
            .expect("shared oversized reconciliation frame");
        assert!(Arc::ptr_eq(&first, &observed));
    }
    assert_eq!(
        broker.log.encoder.count(),
        encode_count + 2,
        "maximum subscriber fan-out must share the pre-serialized frame"
    );

    for _ in 0..(ACTIVITY_STREAM_BROADCAST_CAPACITY + 4) {
        broker.log.publish_heartbeat();
    }
    let before_lag_reset = broker.log.encoder.count();
    let lagged = tokio::time::timeout(Duration::from_millis(100), slow.next_event())
        .await
        .expect("lagged subscriber reset")
        .expect("lagged subscriber event");
    assert_eq!(lagged.kind, "reset");
    assert_eq!(lagged.reason, Some(ACTIVITY_STREAM_OVERSIZED_REASON));
    assert!(lagged.sessions.is_none());
    assert!(lagged.replay_bytes() <= ACTIVITY_STREAM_REPLAY_BYTE_CAPACITY);
    assert_eq!(
        broker.log.encoder.count(),
        before_lag_reset + 1,
        "the first lagging subscriber materializes one cached reset frame"
    );
    let second_lagged = subscribers[0]
        .next_event()
        .await
        .expect("second lagging subscriber reset");
    assert!(Arc::ptr_eq(&lagged, &second_lagged));
    assert_eq!(
        broker.log.encoder.count(),
        before_lag_reset + 1,
        "lagging subscribers must share the cached reconciliation frame"
    );
    let state = broker.log.state.lock().expect("activity event state lock");
    assert!(state.history_bytes <= ACTIVITY_STREAM_REPLAY_BYTE_CAPACITY);
    assert_eq!(ACTIVITY_STREAM_BROADCAST_CAPACITY, 1);
}

#[tokio::test]
async fn activity_stream_persistent_oversized_state_emits_one_transition_reset() {
    let log = ActivityEventLog::new(MACHINE.to_string(), Vec::new());
    let cursor = format!("{}:1", log.stream_id);
    let mut subscriber = log.subscribe(Some(&cursor));
    let oversized_sessions = |prefix: &str| {
        (0..128)
            .map(|index| ActivityStreamSession {
                id: format!("{prefix}-{index:03}-{}", "x".repeat(128)),
                turn_state: Some(test_stream_turn_state(index, "x".repeat(8192))),
            })
            .collect()
    };

    log.publish_snapshot(oversized_sessions("oversized-first"));
    let transition = subscriber.next_event().await.expect("oversized reset");
    assert_eq!(transition.kind, "reset");
    assert_eq!(transition.reason, Some(ACTIVITY_STREAM_OVERSIZED_REASON));
    let transition_sequence = transition.sequence;

    log.publish_snapshot(oversized_sessions("oversized-still"));
    assert_eq!(
        log.sequence.load(Ordering::SeqCst),
        transition_sequence,
        "persistent oversized snapshots must not create hidden gaps or repeated resets"
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(25), subscriber.next_event())
            .await
            .is_err(),
        "persistent oversized state amplified one transition into another poll reset"
    );

    log.publish_snapshot(vec![ActivityStreamSession {
        id: "bounded-recovery".to_string(),
        turn_state: None,
    }]);
    let recovery = subscriber.next_event().await.expect("bounded recovery");
    assert_eq!(recovery.kind, "snapshot");
    assert_eq!(recovery.sequence, transition_sequence + 1);
    assert_eq!(
        recovery.sessions.as_ref().unwrap()[0].id,
        "bounded-recovery"
    );
}

#[tokio::test]
async fn activity_stream_rejects_subscriber_saturation_and_reclaims_permits() {
    let tmp = tempfile::TempDir::new().unwrap();
    let broker = ActivityBroker::for_test_with_subscriber_limit(MACHINE, 1);
    let st = state_with_activity_broker(tmp.path(), Some(TOKEN), PathBuf::from("tmux"), broker);

    let first = router(st.clone())
        .oneshot(get_auth("/activity/events", Some(TOKEN)))
        .await
        .unwrap();
    assert_eq!(first.status(), StatusCode::OK);

    let (status, body) = call(
        router(st.clone()),
        get_auth("/activity/events", Some(TOKEN)),
    )
    .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(body["error"]["code"], "activity-stream-capacity");

    drop(first);
    let reclaimed = router(st)
        .oneshot(get_auth("/activity/events", Some(TOKEN)))
        .await
        .unwrap();
    assert_eq!(reclaimed.status(), StatusCode::OK);
}

#[tokio::test]
async fn activity_broker_watcher_error_resets_existing_stream_and_disables_heartbeats() {
    let tmp = tempfile::TempDir::new().unwrap();
    let broker = ActivityBroker::for_test(MACHINE);
    let mut subscription = broker.subscribe(None).expect("ready subscription");
    let initial = subscription.next_event().await.expect("initial snapshot");
    assert_eq!(initial.kind, "snapshot");
    let sequence_before_failure = broker.log.sequence.load(Ordering::SeqCst);
    let (change_tx, _change_rx) = mpsc::channel(1);

    activity_watch_callback(
        Err(notify::Error::generic("injected watcher failure")),
        &change_tx,
        &broker.lifecycle,
        &tmp.path().join("sessions"),
    );

    let reset = subscription
        .next_event()
        .await
        .expect("degraded stream reconciliation reset");
    assert_degraded_reset_survives_consumer_dedupe(&initial, &reset);
    assert_eq!(reset.observed_at, initial.observed_at);
    assert!(subscription.next_event().await.is_none());
    assert!(matches!(
        broker.subscribe(None),
        Err(ActivitySubscribeError::Unavailable)
    ));
    assert!(!activity_publish_heartbeat_if_ready(
        &broker.log,
        &broker.lifecycle
    ));
    assert_eq!(
        broker.log.sequence.load(Ordering::SeqCst),
        reset.sequence,
        "a degraded broker must not emit healthy heartbeats"
    );
    assert_eq!(reset.sequence, sequence_before_failure + 1);

    tokio::time::sleep(Duration::from_millis(2)).await;
    let st = state_with_activity_broker(tmp.path(), Some(TOKEN), PathBuf::from("tmux"), broker);
    let (poll_status, poll_body) = call(router(st.clone()), get("/sessions")).await;
    assert_eq!(poll_status, StatusCode::OK);
    let poll_observed_at = poll_body["data"]["observed_at"]
        .as_str()
        .unwrap()
        .parse::<jiff::Timestamp>()
        .unwrap();
    let reset_observed_at = reset.observed_at.parse::<jiff::Timestamp>().unwrap();
    assert!(poll_observed_at > reset_observed_at);
    let (status, body) = call(router(st), get_auth("/activity/events", Some(TOKEN))).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["error"]["code"], "activity-stream-unavailable");
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("session polling")
    );
}

#[tokio::test]
async fn activity_broker_rescan_flag_forces_full_snapshot_refresh() {
    let tmp = tempfile::TempDir::new().unwrap();
    let context = CliContext {
        state_dir: tmp.path().to_path_buf(),
        host: None,
    };
    let broker = ActivityBroker::for_test(MACHINE);
    let calls = Arc::new(AtomicUsize::new(0));
    let observed_calls = calls.clone();
    let collector: SessionCollector = Arc::new(move |_, _| {
        observed_calls.fetch_add(1, Ordering::SeqCst);
        Ok(Vec::new())
    });
    let (change_tx, change_rx) = mpsc::channel(1);
    let change_task = tokio::spawn(activity_change_loop(
        broker.log.clone(),
        broker.lifecycle.clone(),
        collector,
        context,
        PathBuf::from("tmux"),
        change_rx,
    ));
    let rescan = NotifyEvent::new(EventKind::Other).set_flag(notify::event::Flag::Rescan);

    activity_watch_callback(
        Ok(rescan),
        &change_tx,
        &broker.lifecycle,
        &tmp.path().join("sessions"),
    );
    tokio::time::timeout(Duration::from_secs(1), async {
        while calls.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("rescan-triggered full refresh");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(*broker.lifecycle.borrow(), ActivityBrokerLifecycle::Ready);

    change_task.abort();
    let _ = change_task.await;
}

#[tokio::test]
async fn activity_broker_root_loss_rearm_failure_degrades_and_stops_stream() {
    let tmp = tempfile::TempDir::new().unwrap();
    let context = CliContext {
        state_dir: tmp.path().to_path_buf(),
        host: None,
    };
    let broker = ActivityBroker::for_test(MACHINE);
    let mut subscription = broker.subscribe(None).expect("ready subscription");
    let initial = subscription.next_event().await.expect("initial snapshot");
    let sessions_root = tmp.path().join("sessions");
    std::fs::create_dir_all(&sessions_root).unwrap();
    let (change_tx, change_rx) = mpsc::channel(1);
    let change_task = tokio::spawn(activity_change_loop_inner(
        broker.log.clone(),
        broker.lifecycle.clone(),
        default_session_collector(),
        context,
        PathBuf::from("tmux"),
        change_rx,
        ActivityChangeLoopControls {
            refresh_started: None,
            watch_rearm: Some(Arc::new(|| false)),
        },
    ));
    let removed = NotifyEvent::new(EventKind::Remove(notify::event::RemoveKind::Folder))
        .add_path(sessions_root.clone());

    activity_watch_callback(Ok(removed), &change_tx, &broker.lifecycle, &sessions_root);
    tokio::time::timeout(Duration::from_secs(1), change_task)
        .await
        .expect("root-loss loop terminates")
        .expect("root-loss task");
    assert_eq!(
        *broker.lifecycle.borrow(),
        ActivityBrokerLifecycle::Degraded
    );
    let reset = subscription.next_event().await.expect("degraded reset");
    assert_degraded_reset_survives_consumer_dedupe(&initial, &reset);
    assert!(subscription.next_event().await.is_none());
    assert!(matches!(
        broker.subscribe(None),
        Err(ActivitySubscribeError::Unavailable)
    ));
    assert!(!activity_publish_heartbeat_if_ready(
        &broker.log,
        &broker.lifecycle
    ));
}

#[tokio::test]
async fn activity_broker_collector_failure_degrades_and_stops_existing_stream() {
    let tmp = tempfile::TempDir::new().unwrap();
    let broker = ActivityBroker::for_test(MACHINE);
    let mut subscription = broker.subscribe(None).expect("ready subscription");
    let initial = subscription.next_event().await.expect("initial snapshot");
    let failing_collector: SessionCollector = Arc::new(|_, _| {
        Err(CliError::runtime(
            "activity-snapshot-failed",
            "injected activity snapshot failure",
            None,
        ))
    });
    let context = CliContext {
        state_dir: tmp.path().to_path_buf(),
        host: None,
    };

    assert!(
        !claude_completion::activity_refresh_snapshot(
            broker.log.clone(),
            broker.lifecycle.clone(),
            failing_collector,
            context,
            PathBuf::from("tmux"),
        )
        .await
    );

    let reset = subscription
        .next_event()
        .await
        .expect("collector failure reset");
    assert_degraded_reset_survives_consumer_dedupe(&initial, &reset);
    assert_eq!(reset.observed_at, initial.observed_at);
    assert!(subscription.next_event().await.is_none());
    assert!(matches!(
        broker.subscribe(None),
        Err(ActivitySubscribeError::Unavailable)
    ));
}

#[tokio::test]
async fn sessions_poll_and_activity_broker_share_one_injected_snapshot_source() {
    let tmp = tempfile::TempDir::new().unwrap();
    let context = CliContext {
        state_dir: tmp.path().to_path_buf(),
        host: None,
    };
    let tmux = minimal_tmux(tmp.path());
    let cwd = tmp.path().join("repo");
    std::fs::create_dir_all(&cwd).unwrap();
    seed_fresh_provider_session(
        tmp.path(),
        "must-be-hidden-by-injected-source",
        "codex",
        "hs-source-parity",
        &cwd,
        None,
    );
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let observed_calls = calls.clone();
    let collector: SessionCollector = Arc::new(move |_, _| {
        observed_calls.fetch_add(1, Ordering::SeqCst);
        Ok(Vec::new())
    });
    let broker = ActivityBroker::for_test_with_session_collector(
        MACHINE,
        &context,
        &tmux,
        collector.clone(),
    );
    let st = state_with_dependencies(tmp.path(), Some(TOKEN), tmux, broker.clone(), collector);

    let (status, body) = call(router(st), get("/sessions")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["data"]["sessions"], json!([]));
    let mut subscription = broker.subscribe(None).expect("ready subscription");
    let snapshot = subscription.next_event().await.expect("stream snapshot");
    assert_eq!(snapshot.sessions, Some(Vec::new()));
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn activity_broker_initial_collector_failure_starts_degraded_and_returns_poll_fallback() {
    let tmp = tempfile::TempDir::new().unwrap();
    let context = CliContext {
        state_dir: tmp.path().to_path_buf(),
        host: None,
    };
    let failing_collector: SessionCollector = Arc::new(|_, _| {
        Err(CliError::runtime(
            "activity-snapshot-failed",
            "injected initial activity snapshot failure",
            None,
        ))
    });
    let broker = ActivityBroker::start(
        context,
        MACHINE.to_string(),
        minimal_tmux(tmp.path()),
        failing_collector.clone(),
    )
    .await;
    assert_eq!(
        *broker.lifecycle.borrow(),
        ActivityBrokerLifecycle::Degraded
    );
    assert!(matches!(
        broker.subscribe(None),
        Err(ActivitySubscribeError::Unavailable)
    ));

    let st = state_with_dependencies(
        tmp.path(),
        Some(TOKEN),
        PathBuf::from("tmux"),
        broker,
        failing_collector,
    );
    let (status, body) = call(router(st), get_auth("/activity/events", Some(TOKEN))).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["error"]["code"], "activity-stream-unavailable");
}

#[tokio::test]
async fn activity_broker_change_channel_close_resets_once_stops_stream_and_heartbeat() {
    let tmp = tempfile::TempDir::new().unwrap();
    let context = CliContext {
        state_dir: tmp.path().to_path_buf(),
        host: None,
    };
    let broker = ActivityBroker::for_test(MACHINE);
    let mut subscription = broker.subscribe(None).expect("ready subscription");
    let initial = subscription.next_event().await.expect("initial snapshot");
    assert_eq!(initial.kind, "snapshot");
    let sequence_before_close = broker.log.sequence.load(Ordering::SeqCst);
    let (change_tx, change_rx) = mpsc::channel(1);
    let change_task = tokio::spawn(activity_change_loop(
        broker.log.clone(),
        broker.lifecycle.clone(),
        default_session_collector(),
        context,
        PathBuf::from("tmux"),
        change_rx,
    ));
    let heartbeat_task = tokio::spawn(activity_heartbeat_loop(
        broker.log.clone(),
        broker.lifecycle.clone(),
    ));

    drop(change_tx);
    tokio::time::timeout(Duration::from_millis(250), change_task)
        .await
        .expect("change loop stops after notification channel close")
        .expect("change loop task");
    assert_eq!(
        *broker.lifecycle.borrow(),
        ActivityBrokerLifecycle::Degraded
    );
    let reset = subscription
        .next_event()
        .await
        .expect("channel-close reconciliation");
    assert_degraded_reset_survives_consumer_dedupe(&initial, &reset);
    assert_eq!(reset.observed_at, initial.observed_at);
    assert!(subscription.next_event().await.is_none());
    tokio::time::timeout(Duration::from_millis(250), heartbeat_task)
        .await
        .expect("heartbeat loop stops after degradation")
        .expect("heartbeat loop task");
    assert_eq!(
        broker.log.sequence.load(Ordering::SeqCst),
        reset.sequence,
        "notification-channel failure must not leave a healthy heartbeat running"
    );
    assert_eq!(reset.sequence, sequence_before_close + 1);
}

#[tokio::test]
async fn activity_broker_notification_refresh_matches_stateful_shared_poll_source() {
    let tmp = tempfile::TempDir::new().unwrap();
    let context = CliContext {
        state_dir: tmp.path().to_path_buf(),
        host: None,
    };
    let tmux = minimal_tmux(tmp.path());
    let cwd = tmp.path().join("repo");
    std::fs::create_dir_all(&cwd).unwrap();
    for id in ["source-before", "source-after"] {
        seed_fresh_provider_session(tmp.path(), id, "codex", &format!("hs-{id}"), &cwd, None);
    }
    let source_revision = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let observed_revision = source_revision.clone();
    let collector: SessionCollector = Arc::new(move |context, tmux_bin| {
        let expected_id = if observed_revision.load(Ordering::SeqCst) == 0 {
            "source-before"
        } else {
            "source-after"
        };
        crate::list_sessions(context, Some(tmux_bin)).map(|sessions| {
            sessions
                .into_iter()
                .filter(|session| session.id == expected_id)
                .collect()
        })
    });
    let broker = ActivityBroker::for_test_with_session_collector(
        MACHINE,
        &context,
        &tmux,
        collector.clone(),
    );
    let mut subscription = broker.subscribe(None).expect("ready subscription");
    let initial = subscription.next_event().await.expect("initial snapshot");
    assert_eq!(initial.sessions.as_ref().unwrap()[0].id, "source-before");
    let st = state_with_dependencies(
        tmp.path(),
        Some(TOKEN),
        tmux.clone(),
        broker.clone(),
        collector.clone(),
    );
    source_revision.store(1, Ordering::SeqCst);

    let (status, body) = call(router(st), get("/sessions")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["data"]["sessions"][0]["id"], "source-after");

    let (change_tx, change_rx) = mpsc::channel(1);
    let change_task = tokio::spawn(activity_change_loop(
        broker.log.clone(),
        broker.lifecycle.clone(),
        collector,
        context,
        tmux,
        change_rx,
    ));
    change_tx
        .send(ActivityChange::Refresh)
        .await
        .expect("activity notification");
    let refreshed = tokio::time::timeout(Duration::from_secs(1), subscription.next_event())
        .await
        .expect("notification refresh")
        .expect("refreshed snapshot");
    assert_eq!(refreshed.kind, "snapshot");
    assert_eq!(refreshed.sessions.as_ref().unwrap()[0].id, "source-after");

    change_task.abort();
    let _ = change_task.await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn activity_broker_notification_storm_with_slow_scans_converges_final_revision() {
    const NOTIFICATIONS: usize = 300;
    let tmp = tempfile::TempDir::new().unwrap();
    let context = CliContext {
        state_dir: tmp.path().to_path_buf(),
        host: None,
    };
    let tmux = minimal_tmux(tmp.path());
    let cwd = tmp.path().join("repo");
    std::fs::create_dir_all(&cwd).unwrap();
    for id in ["storm-before", "storm-final"] {
        seed_fresh_provider_session(tmp.path(), id, "codex", &format!("hs-{id}"), &cwd, None);
    }
    let source_revision = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let observed_revision = source_revision.clone();
    let collector: SessionCollector = Arc::new(move |context, tmux_bin| {
        std::thread::sleep(Duration::from_millis(75));
        let expected_id = if observed_revision.load(Ordering::SeqCst) == NOTIFICATIONS {
            "storm-final"
        } else {
            "storm-before"
        };
        crate::list_sessions(context, Some(tmux_bin)).map(|sessions| {
            sessions
                .into_iter()
                .filter(|session| session.id == expected_id)
                .collect()
        })
    });
    let broker = ActivityBroker::for_test_with_session_collector(
        MACHINE,
        &context,
        &tmux,
        collector.clone(),
    );
    let mut subscription = broker.subscribe(None).expect("ready subscription");
    let initial = subscription.next_event().await.expect("initial snapshot");
    assert_eq!(initial.sessions.as_ref().unwrap()[0].id, "storm-before");
    let (change_tx, change_rx) = mpsc::channel(1);
    let change_task = tokio::spawn(activity_change_loop(
        broker.log.clone(),
        broker.lifecycle.clone(),
        collector,
        context,
        tmux,
        change_rx,
    ));

    for revision in 1..=NOTIFICATIONS {
        source_revision.store(revision, Ordering::SeqCst);
        let _ = change_tx.try_send(ActivityChange::Refresh);
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    // This real-time integration case asserts only liveness and final-state
    // convergence. macOS CI scheduling can legitimately vary the number of
    // scans observed during the wall-clock storm; exact cadence and spacing
    // are covered by the paused-time dense, periodic, and slow-scan tests.
    let final_snapshot =
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let event = subscription.next_event().await.expect("activity event");
                if event.sessions.as_ref().is_some_and(|sessions| {
                    sessions.iter().any(|session| session.id == "storm-final")
                }) {
                    break event;
                }
            }
        })
        .await
        .expect("storm converges to final revision");
    assert_eq!(final_snapshot.kind, "snapshot");

    change_task.abort();
    let _ = change_task.await;
}

#[tokio::test]
async fn activity_broker_refresh_starts_respect_the_minimum_cadence() {
    let tmp = tempfile::TempDir::new().unwrap();
    let context = CliContext {
        state_dir: tmp.path().to_path_buf(),
        host: None,
    };
    let broker = ActivityBroker::for_test(MACHINE);
    let (started_tx, mut started_rx) = mpsc::unbounded_channel();
    let (change_tx, change_rx) = mpsc::channel(1);
    let change_task = tokio::spawn(activity_change_loop_inner(
        broker.log.clone(),
        broker.lifecycle.clone(),
        Arc::new(|_, _| Ok(Vec::new())),
        context,
        PathBuf::from("tmux"),
        change_rx,
        ActivityChangeLoopControls {
            refresh_started: Some(started_tx),
            ..Default::default()
        },
    ));

    change_tx
        .send(ActivityChange::Refresh)
        .await
        .expect("first activity notification");
    let first = tokio::time::timeout(Duration::from_secs(1), started_rx.recv())
        .await
        .expect("first refresh start")
        .expect("first refresh timestamp");
    change_tx
        .send(ActivityChange::Refresh)
        .await
        .expect("second activity notification");
    let second = tokio::time::timeout(Duration::from_secs(1), started_rx.recv())
        .await
        .expect("second refresh start")
        .expect("second refresh timestamp");
    assert!(
        second.duration_since(first) >= ACTIVITY_STREAM_MAX_REFRESH_CADENCE,
        "refreshes started too close together: {:?}",
        second.duration_since(first)
    );

    change_task.abort();
    let _ = change_task.await;
}

async fn drive_paused_activity_refresh_pattern(
    interval: Duration,
    notifications: usize,
) -> (Vec<Duration>, Vec<usize>) {
    let tmp = tempfile::TempDir::new().unwrap();
    let context = CliContext {
        state_dir: tmp.path().to_path_buf(),
        host: None,
    };
    let broker = ActivityBroker::for_test(MACHINE);
    let source_revision = Arc::new(AtomicUsize::new(0));
    let sampled_revisions = Arc::new(StdMutex::new(Vec::new()));
    let (sampled_tx, mut sampled_rx) = watch::channel(0_usize);
    let observed_revision = source_revision.clone();
    let observed_samples = sampled_revisions.clone();
    let collector: SessionCollector = Arc::new(move |_, _| {
        let revision = observed_revision.load(Ordering::SeqCst);
        observed_samples
            .lock()
            .expect("sampled revision lock")
            .push(revision);
        sampled_tx.send_replace(revision);
        Ok(Vec::new())
    });
    let (change_tx, change_rx) = mpsc::channel(1);
    let (started_tx, mut started_rx) = mpsc::unbounded_channel();
    let origin = tokio::time::Instant::now();
    let change_task = tokio::spawn(activity_change_loop_inner(
        broker.log.clone(),
        broker.lifecycle.clone(),
        collector,
        context,
        PathBuf::from("tmux"),
        change_rx,
        ActivityChangeLoopControls {
            refresh_started: Some(started_tx),
            watch_rearm: None,
        },
    ));

    for revision in 1..=notifications {
        source_revision.store(revision, Ordering::SeqCst);
        let _ = change_tx.try_send(ActivityChange::Refresh);
        for _ in 0..if revision == 1 { 4 } else { 1 } {
            tokio::task::yield_now().await;
        }
        tokio::time::advance(interval).await;
    }
    for _ in 0..4 {
        tokio::task::yield_now().await;
    }
    tokio::time::advance(ACTIVITY_STREAM_MAX_REFRESH_CADENCE + ACTIVITY_STREAM_DEBOUNCE).await;
    let (deadline_tx, mut deadline_rx) = mpsc::unbounded_channel();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(5));
        let _ = deadline_tx.send(());
    });
    tokio::select! {
        _ = async {
            loop {
                if *sampled_rx.borrow_and_update() == notifications {
                    break;
                }
                sampled_rx.changed().await.expect("sampled revision channel");
            }
        } => {}
        _ = deadline_rx.recv() => panic!("activity refresh did not sample revision {notifications}"),
    }

    let mut starts = Vec::new();
    while let Ok(started_at) = started_rx.try_recv() {
        starts.push(started_at.duration_since(origin));
    }
    let samples = sampled_revisions
        .lock()
        .expect("sampled revision lock")
        .clone();
    change_task.abort();
    let _ = change_task.await;
    (starts, samples)
}

#[tokio::test(start_paused = true)]
async fn activity_refresh_manual_time_dense_and_26ms_patterns_are_rate_bounded() {
    let (dense_starts, dense_samples) =
        drive_paused_activity_refresh_pattern(Duration::from_millis(2), 300).await;
    assert_eq!(dense_starts.first(), Some(&Duration::from_millis(250)));
    assert!(dense_starts.len() <= 3, "dense refreshes: {dense_starts:?}");
    assert_eq!(dense_samples.last(), Some(&300));
    assert!(
        dense_starts
            .windows(2)
            .all(|starts| { starts[1] - starts[0] >= ACTIVITY_STREAM_MAX_REFRESH_CADENCE })
    );

    let (periodic_starts, periodic_samples) =
        drive_paused_activity_refresh_pattern(Duration::from_millis(26), 30).await;
    assert!(
        periodic_starts.first().is_some_and(
            |start| *start >= Duration::from_millis(25) && *start <= Duration::from_millis(26)
        ),
        "isolated trailing quiet start: {periodic_starts:?}"
    );
    assert!(
        periodic_starts.len() <= 4,
        "26ms refreshes: {periodic_starts:?}"
    );
    assert_eq!(periodic_samples.last(), Some(&30));
    assert!(
        periodic_starts
            .windows(2)
            .all(|starts| { starts[1] - starts[0] >= ACTIVITY_STREAM_MAX_REFRESH_CADENCE })
    );
}

#[tokio::test(start_paused = true)]
async fn activity_refresh_manual_time_slow_scan_retains_dirty_final_revision() {
    const NOTIFICATIONS: usize = 100;
    let tmp = tempfile::TempDir::new().unwrap();
    let context = CliContext {
        state_dir: tmp.path().to_path_buf(),
        host: None,
    };
    let tmux = minimal_tmux(tmp.path());
    let cwd = tmp.path().join("repo");
    std::fs::create_dir_all(&cwd).unwrap();
    for id in ["slow-before", "slow-final"] {
        seed_fresh_provider_session(tmp.path(), id, "codex", &format!("hs-{id}"), &cwd, None);
    }
    let broker = ActivityBroker::for_test(MACHINE);
    let mut subscription = broker.subscribe(None).expect("ready subscription");
    assert_eq!(
        subscription.next_event().await.unwrap().sessions,
        Some(Vec::new())
    );
    let source_revision = Arc::new(AtomicUsize::new(0));
    let calls = Arc::new(AtomicUsize::new(0));
    let observed_revision = source_revision.clone();
    let observed_calls = calls.clone();
    let (collector_started_tx, mut collector_started_rx) = mpsc::unbounded_channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let release_rx = Arc::new(StdMutex::new(release_rx));
    let observed_release = release_rx.clone();
    let collector: SessionCollector = Arc::new(move |context, tmux_bin| {
        let call = observed_calls.fetch_add(1, Ordering::SeqCst);
        let revision = observed_revision.load(Ordering::SeqCst);
        if call == 0 {
            let _ = collector_started_tx.send(());
            observed_release
                .lock()
                .expect("slow collector release lock")
                .recv()
                .expect("slow collector release");
        }
        let expected_id = if revision == 0 {
            "slow-before"
        } else {
            "slow-final"
        };
        crate::list_sessions(context, Some(tmux_bin)).map(|sessions| {
            sessions
                .into_iter()
                .filter(|session| session.id == expected_id)
                .collect()
        })
    });
    let (change_tx, change_rx) = mpsc::channel(1);
    let (started_tx, mut started_rx) = mpsc::unbounded_channel();
    let origin = tokio::time::Instant::now();
    let change_task = tokio::spawn(activity_change_loop_inner(
        broker.log.clone(),
        broker.lifecycle.clone(),
        collector,
        context,
        tmux,
        change_rx,
        ActivityChangeLoopControls {
            refresh_started: Some(started_tx),
            watch_rearm: None,
        },
    ));

    change_tx
        .send(ActivityChange::Refresh)
        .await
        .expect("initial activity notification");
    for _ in 0..4 {
        tokio::task::yield_now().await;
    }
    tokio::time::advance(ACTIVITY_STREAM_DEBOUNCE).await;
    assert_eq!(
        started_rx.recv().await.unwrap().duration_since(origin),
        Duration::from_millis(25)
    );
    collector_started_rx
        .recv()
        .await
        .expect("slow collector started");
    for revision in 1..=NOTIFICATIONS {
        source_revision.store(revision, Ordering::SeqCst);
        let _ = change_tx.try_send(ActivityChange::Refresh);
        tokio::time::advance(Duration::from_millis(2)).await;
    }
    release_tx.send(()).expect("release slow collector");
    let before = subscription.next_event().await.expect("pre-storm snapshot");
    assert_eq!(before.sessions.as_ref().unwrap()[0].id, "slow-before");
    tokio::time::advance(Duration::from_millis(50)).await;
    let second_started = started_rx.recv().await.expect("final refresh start");
    assert_eq!(
        second_started.duration_since(origin),
        Duration::from_millis(275)
    );
    let final_snapshot = subscription.next_event().await.expect("final snapshot");
    assert_eq!(
        final_snapshot.sessions.as_ref().unwrap()[0].id,
        "slow-final"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 2);

    change_task.abort();
    let _ = change_task.await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn activity_sessions_root_delete_recreate_delivers_transition_or_degrades() {
    let tmp = tempfile::TempDir::new().unwrap();
    let tmux = minimal_tmux(tmp.path());
    let context = CliContext {
        state_dir: tmp.path().to_path_buf(),
        host: None,
    };
    let broker = ActivityBroker::start(
        context.clone(),
        MACHINE.to_string(),
        tmux,
        default_session_collector(),
    )
    .await;
    let mut subscription = broker.subscribe(None).expect("ready activity broker");
    let initial = subscription.next_event().await.expect("initial snapshot");
    let sessions_root = tmp.path().join("sessions");
    std::fs::remove_dir_all(&sessions_root).expect("remove watched sessions root");
    let _ = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if *broker.lifecycle.borrow() == ActivityBrokerLifecycle::Degraded
                || sessions_root.is_dir()
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;

    if *broker.lifecycle.borrow() == ActivityBrokerLifecycle::Degraded {
        let reset = subscription.next_event().await.expect("root-loss reset");
        assert_degraded_reset_survives_consumer_dedupe(&initial, &reset);
        assert!(subscription.next_event().await.is_none());
        assert!(matches!(
            broker.subscribe(None),
            Err(ActivitySubscribeError::Unavailable)
        ));
        assert!(!activity_publish_heartbeat_if_ready(
            &broker.log,
            &broker.lifecycle
        ));
        broker.shutdown().await;
        return;
    }

    let cwd = tmp.path().join("repo");
    std::fs::create_dir_all(&cwd).unwrap();
    seed_fresh_provider_session(
        tmp.path(),
        "root-rearmed-session",
        "codex",
        "hs-root-rearmed-session",
        &cwd,
        None,
    );
    let record = load_session_record(&context, "root-rearmed-session").expect("session record");
    crate::activity::activate_runtime(&context, &record).expect("activity snapshot");
    let delivered = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let event = subscription.next_event().await.expect("activity event");
            if event.sessions.as_ref().is_some_and(|sessions| {
                sessions
                    .iter()
                    .any(|session| session.id == "root-rearmed-session")
            }) {
                break event;
            }
        }
    })
    .await
    .expect("re-armed root delivers later transition");
    assert_eq!(delivered.kind, "snapshot");
    assert_eq!(*broker.lifecycle.borrow(), ActivityBrokerLifecycle::Ready);
    broker.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn activity_file_notification_publishes_transition_without_http_polling() {
    let tmp = tempfile::TempDir::new().unwrap();
    let tmux = minimal_tmux(tmp.path());
    let context = CliContext {
        state_dir: tmp.path().to_path_buf(),
        host: None,
    };
    let broker = ActivityBroker::start(
        context.clone(),
        MACHINE.to_string(),
        tmux,
        default_session_collector(),
    )
    .await;
    let mut subscription = broker.subscribe(None).expect("ready activity broker");
    assert_eq!(subscription.next_event().await.unwrap().kind, "snapshot");

    let cwd = tmp.path().join("repo");
    std::fs::create_dir_all(&cwd).unwrap();
    seed_fresh_provider_session(
        tmp.path(),
        "streamed-session",
        "codex",
        "hs-streamed-session",
        &cwd,
        None,
    );
    let record = load_session_record(&context, "streamed-session").expect("session record");
    crate::activity::activate_runtime(&context, &record).expect("activity snapshot");

    let event = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let event = subscription.next_event().await.expect("activity event");
            let has_activity = event.sessions.as_ref().is_some_and(|sessions| {
                sessions
                    .iter()
                    .any(|session| session.id == "streamed-session" && session.turn_state.is_some())
            });
            if has_activity {
                break event;
            }
        }
    })
    .await
    .expect("filesystem-driven transition");
    assert_eq!(event.kind, "snapshot");
    let session = event
        .sessions
        .as_ref()
        .unwrap()
        .iter()
        .find(|session| session.id == "streamed-session")
        .unwrap();
    assert_eq!(session.turn_state.as_ref().unwrap().revision, 1);

    std::fs::remove_file(tmp.path().join("sessions/streamed-session/activity.json"))
        .expect("remove activity snapshot");
    let cleared = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let event = subscription.next_event().await.expect("activity event");
            if event.sessions.as_ref().is_some_and(|sessions| {
                sessions
                    .iter()
                    .any(|session| session.id == "streamed-session" && session.turn_state.is_none())
            }) {
                break event;
            }
        }
    })
    .await
    .expect("activity removal transition");
    assert_eq!(cleared.kind, "snapshot");

    std::fs::remove_dir_all(tmp.path().join("sessions/streamed-session")).expect("remove session");
    let deleted = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let event = subscription.next_event().await.expect("activity event");
            if event.sessions.as_ref().is_some_and(|sessions| {
                sessions
                    .iter()
                    .all(|session| session.id != "streamed-session")
            }) {
                break event;
            }
        }
    })
    .await
    .expect("session removal transition");
    assert_eq!(deleted.kind, "snapshot");
    broker.shutdown().await;
}

#[tokio::test]
async fn list_preserves_provider_resume_metadata() {
    let tmp = tempfile::TempDir::new().unwrap();
    let cwd = tmp.path().join("repo");
    std::fs::create_dir_all(&cwd).unwrap();
    seed_resumable_session(
        tmp.path(),
        "recover",
        "codex",
        "hs-codex-recover",
        &cwd,
        &[
            "resume",
            "resume-session-id",
            "--cd",
            cwd.to_str().unwrap(),
            "--no-alt-screen",
        ],
    );
    add_provider_resume_extra(tmp.path(), "recover");
    let st = state(tmp.path(), Some(TOKEN), minimal_tmux(tmp.path()));

    let (status, body) = call(router(st.clone()), get("/sessions")).await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let session = &body["data"]["sessions"][0];
    assert_eq!(session["provider_resume"]["provider"], "codex");
    assert_eq!(
        session["provider_resume"]["session_id"],
        "resume-session-id"
    );
    assert_eq!(
        session["provider_resume"]["resume_args"],
        json!([
            "resume",
            "resume-session-id",
            "--cd",
            cwd.to_str().unwrap(),
            "--no-alt-screen"
        ])
    );
    assert!(session["provider_resume"].get("storage_only").is_none());
    assert_eq!(body["data"]["capabilities"]["group_archive"], true);
}

#[tokio::test]
async fn history_routes_require_token_and_advertise_metadata_only_search() {
    let tmp = tempfile::TempDir::new().unwrap();
    let st = state(tmp.path(), Some(TOKEN), PathBuf::from("tmux"));

    let (status, body) = call(router(st.clone()), get("/history/sessions")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "body={body}");

    let (status, body) = call(
        router(st),
        get_auth("/history/sessions?limit=25&provider=unknown", Some(TOKEN)),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["data"]["sessions"], json!([]));
    assert_eq!(body["data"]["capabilities"]["metadata_search"], true);
    assert_eq!(body["data"]["capabilities"]["full_text_search"], false);
    assert_eq!(body["data"]["capabilities"]["latest_message_paging"], true);
    assert_eq!(body["data"]["capabilities"]["star"], true);
}

#[tokio::test]
async fn history_list_enables_only_explicit_ready_dsh_exact_resume() {
    let tmp = tempfile::TempDir::new().unwrap();
    let cwd = tmp.path().join("repo");
    let history_root = tmp.path().join("dsh-sessions");
    fs::create_dir(&cwd).unwrap();
    fs::create_dir(&history_root).unwrap();
    let launcher = executable(&tmp.path().join("dsh-launcher"), "#!/bin/sh\nexit 0\n");
    let adapter = executable(
        &tmp.path().join("dsh-history"),
        &format!(
            r##"#!/bin/sh
case "$1" in
  list) printf '%s\n' '{{"schema_version":"dsh-runtime-kit.history.v1","data":[{{"provider_session_id":"dsh-one","cwd":"{}","created_at":"2026-09-12T01:00:00.000Z","updated_at":"2026-09-12T02:00:00.000Z","revision":"rev-1"}}]}}' ;;
  summaries) printf '%s\n' '{{"schema_version":"dsh-runtime-kit.history.v1","data":[]}}' ;;
  *) exit 64 ;;
esac
"##,
            cwd.display()
        ),
    );
    let profiles = AgentLaunchProfiles::from_json(
        &json!([{
            "id":"dsh-workbench",
            "label":"DSH",
            "agent":"dsh",
            "agent_bin":launcher,
            "dsh_history":{
                "command":adapter,
                "root":history_root,
                "resume":"exact-id"
            }
        }])
        .to_string(),
    )
    .unwrap();
    let mut st = state(tmp.path(), Some(TOKEN), PathBuf::from("tmux"));
    Arc::get_mut(&mut st).unwrap().history_catalog = Arc::new(
        HistoryCatalog::new(
            Vec::new(),
            provider_history::archive_root(tmp.path()),
            provider_history::star_root(tmp.path()),
        )
        .with_dsh_sources(profiles.dsh_history_sources()),
    );
    Arc::get_mut(&mut st).unwrap().launch_profiles = profiles;

    let (status, body) = call(
        router(st),
        get_auth("/history/sessions?provider=dsh", Some(TOKEN)),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["data"]["sessions"][0]["provider"], "dsh");
    assert_eq!(body["data"]["sessions"][0]["resumable"], true);
    assert_eq!(body["data"]["capabilities"]["resume"], true);
}

#[test]
fn history_resume_restores_the_archived_session_id_only_while_it_is_free() {
    let tmp = tempfile::TempDir::new().unwrap();
    let transcripts = tmp.path().join("codex-sessions");
    let day = transcripts.join("2026/09/30");
    fs::create_dir_all(&day).unwrap();
    fs::write(
            day.join("rollout.jsonl"),
            "{\"timestamp\":\"2026-09-30T00:00:00Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"resume-session-id\",\"cwd\":\"/work/example\",\"source\":\"cli\",\"timestamp\":\"2026-09-30T00:00:00Z\"}}\n",
        )
        .unwrap();
    let context = CliContext {
        state_dir: tmp.path().to_path_buf(),
        host: None,
    };
    seed_resumable_session(
        tmp.path(),
        "archived-owner",
        "codex",
        "hs-codex-archived-owner",
        tmp.path(),
        &["resume", "resume-session-id"],
    );
    let mut record = load_session_record(&context, "archived-owner").unwrap();
    record
        .extra
        .insert("title_mode".to_string(), json!("pinned"));
    record.title = Some("Saved topic - Saved activity".to_string());
    record.title_state = Some(
        serde_json::from_value(json!({
            "topic": "Saved topic",
            "topic_source": "auto",
            "references": [],
            "activity": "Saved activity"
        }))
        .unwrap(),
    );
    crate::write_session_record(&context, &record).unwrap();
    let tmux = executable(
        &tmp.path().join("archive-tmux"),
        "#!/usr/bin/env sh\ncase \"$1\" in\n  display-message|has-session) printf '%s\\n' \"can't find session: archive-me\" >&2; exit 1 ;;\n  *) exit 42 ;;\nesac\n",
    );
    // The fake tmux is a shell script whose launch cannot guarantee fitting
    // the termination probe's bounded startup budget under load, so pin the
    // probe outcome it models: the session is gone (sympoies/nils-cli#2154).
    let _probe =
        crate::tmux_probe_fixture::install(&tmux, crate::tmux_probe_fixture::Probe::Stopped);
    let (archived, _) = crate::archive_session_with_expected_incarnation(
        &context,
        "archived-owner",
        tmux,
        "never-launched-fixture",
    )
    .unwrap();
    assert_eq!(archived.session_id.as_deref(), Some("archived-owner"));
    assert_eq!(archived.title.as_deref(), record.title.as_deref());
    assert_eq!(archived.title_state, record.title_state);
    assert_eq!(
        serde_json::to_value(&archived).unwrap()["title_mode"],
        "pinned"
    );
    let catalog = HistoryCatalog::new(
        vec![provider_history::HistorySource {
            provider: "codex".to_string(),
            agent_profile: None,
            root: transcripts,
        }],
        provider_history::archive_root(tmp.path()),
        provider_history::star_root(tmp.path()),
    );
    let history = catalog.resolve_fresh(&archived.history_id).unwrap();
    assert_eq!(
        history.title.as_deref(),
        Some("Saved topic - Saved activity")
    );

    let restored = history_resume_identity(&context, &history).unwrap();
    assert_eq!(restored.id.as_deref(), Some("archived-owner"));
    assert_eq!(restored.title_state, record.title_state);
    assert_eq!(
        restored.title_mode,
        crate::display_metadata::TitleMode::Pinned
    );

    // A title that no longer renders from its state keeps the title alone.
    let mut edited = history.clone();
    edited.title = Some("Edited by hand".to_string());
    assert_eq!(
        history_resume_identity(&context, &edited)
            .unwrap()
            .title_state,
        None
    );

    // Another session took the id meanwhile: resume falls back to a new id.
    fs::create_dir_all(tmp.path().join("sessions/archived-owner")).unwrap();
    let restored = history_resume_identity(&context, &history).unwrap();
    assert_eq!(restored.id, None);
}

#[tokio::test]
async fn history_resume_refuses_a_conversation_a_live_session_already_owns() {
    let tmp = tempfile::TempDir::new().unwrap();
    let cwd = tmp.path().join("repo");
    fs::create_dir(&cwd).unwrap();
    let transcripts = tmp.path().join("codex-sessions");
    let day = transcripts.join("2026/09/30");
    fs::create_dir_all(&day).unwrap();
    fs::write(
            day.join("rollout.jsonl"),
            format!(
                "{{\"timestamp\":\"2026-09-30T00:00:00Z\",\"type\":\"session_meta\",\"payload\":{{\"id\":\"resume-session-id\",\"cwd\":\"{}\",\"source\":\"cli\",\"timestamp\":\"2026-09-30T00:00:00Z\"}}}}\n",
                cwd.display()
            ),
        )
        .unwrap();
    seed_resumable_session(
        tmp.path(),
        "live-owner",
        "codex",
        "hs-codex-live-owner",
        &cwd,
        &["resume", "resume-session-id"],
    );
    let log = tmp.path().join("tmux.log");
    let tmux = resume_tmux(tmp.path(), &log);
    let mut st = state(tmp.path(), Some(TOKEN), tmux);
    Arc::get_mut(&mut st).unwrap().history_catalog = Arc::new(HistoryCatalog::new(
        vec![provider_history::HistorySource {
            provider: "codex".to_string(),
            agent_profile: None,
            root: transcripts,
        }],
        provider_history::archive_root(tmp.path()),
        provider_history::star_root(tmp.path()),
    ));
    let id = provider_history::stable_history_id("codex", None, "resume-session-id");

    let (status, body) = call(
        router(st),
        post_json(
            &format!("/history/sessions/{id}/resume"),
            Some(TOKEN),
            json!({}),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::CONFLICT, "body={body}");
    assert_eq!(body["error"]["code"], "history-session-live");
    assert_eq!(body["error"]["details"]["id"], "live-owner");
    assert!(
        !log.exists(),
        "a live conversation must not launch a duplicate"
    );
    let sessions: Vec<_> = fs::read_dir(tmp.path().join("sessions"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(sessions, vec![std::ffi::OsString::from("live-owner")]);
}

#[tokio::test]
async fn history_resume_refuses_retired_dsh_history() {
    let tmp = tempfile::TempDir::new().unwrap();
    let cwd = tmp.path().join("repo");
    let history_root = tmp.path().join("dsh-sessions");
    fs::create_dir(&cwd).unwrap();
    fs::create_dir(&history_root).unwrap();
    let launcher = executable(&tmp.path().join("dsh-launcher"), "#!/bin/sh\nexit 0\n");
    let adapter = executable(
        &tmp.path().join("dsh-history"),
        &format!(
            r##"#!/bin/sh
case "$1" in
  list) printf '%s\n' '{{"schema_version":"dsh-runtime-kit.history.v1","data":[{{"provider_session_id":"dsh-one","cwd":"{}","created_at":"2026-09-12T01:00:00.000Z","updated_at":"2026-09-12T02:00:00.000Z","revision":"rev-1"}}]}}' ;;
  summaries) printf '%s\n' '{{"schema_version":"dsh-runtime-kit.history.v1","data":[]}}' ;;
  *) exit 64 ;;
esac
"##,
            cwd.display()
        ),
    );
    let profiles = AgentLaunchProfiles::from_json(
        &json!([{
            "id":"dsh-workbench",
            "label":"DSH",
            "agent":"dsh",
            "agent_bin":launcher,
            "dsh_history":{
                "command":adapter,
                "root":history_root,
                "resume":"exact-id"
            }
        }])
        .to_string(),
    )
    .unwrap();
    let log = tmp.path().join("tmux.log");
    let tmux = resume_tmux(tmp.path(), &log);
    let mut st = state(tmp.path(), Some(TOKEN), tmux);
    Arc::get_mut(&mut st).unwrap().history_catalog = Arc::new(
        HistoryCatalog::new(
            Vec::new(),
            provider_history::archive_root(tmp.path()),
            provider_history::star_root(tmp.path()),
        )
        .with_dsh_sources(profiles.dsh_history_sources()),
    );
    Arc::get_mut(&mut st).unwrap().launch_profiles = profiles;
    let id = provider_history::stable_history_id("dsh", Some("dsh-workbench"), "dsh-one");

    let (status, body) = call(
        router(st.clone()),
        post_json(&format!("/history/sessions/{id}/resume"), None, json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "body={body}");
    assert!(!log.exists(), "unauthorized history resume must not launch");

    let (status, body) = call(
        router(st),
        post_json(
            &format!("/history/sessions/{id}/resume"),
            Some(TOKEN),
            json!({}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "body={body}");
    assert_eq!(body["error"]["code"], "history-resume-not-supported");
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("no longer supported")
    );
    assert!(!log.exists(), "retired history must not launch");
}

#[tokio::test]
async fn fresh_dsh_create_is_rejected_without_side_effects() {
    let tmp = tempfile::TempDir::new().unwrap();
    let cwd = tmp.path().join("repo");
    fs::create_dir(&cwd).unwrap();
    let log = tmp.path().join("tmux.log");
    let tmux = startup_fixtures::live_resume_tmux(tmp.path(), &log);
    let st = state(tmp.path(), Some(TOKEN), tmux);

    let (status, body) = call(
        router(st),
        post_json(
            "/sessions",
            Some(TOKEN),
            json!({
                "agent":"dsh", "id":"fresh-dsh", "cwd":cwd,
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body={body}");
    assert_eq!(body["error"]["code"], "unsupported-agent");
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("no longer supported")
    );
    assert!(!log.exists(), "a retired agent must not launch tmux");
    assert!(!tmp.path().join("sessions/fresh-dsh/session.json").exists());
}

#[tokio::test]
async fn history_star_toggle_stores_and_clears_one_star() {
    let tmp = tempfile::TempDir::new().unwrap();
    let history_root = tmp.path().join("provider/sessions/2026/09/01");
    fs::create_dir_all(&history_root).unwrap();
    fs::write(
            history_root.join("rollout.jsonl"),
            "{\"timestamp\":\"2026-09-01T00:00:00Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"session-a\",\"cwd\":\"/work/example\",\"source\":\"cli\",\"timestamp\":\"2026-09-01T00:00:00Z\"}}\n",
        )
        .unwrap();
    let mut st = state(tmp.path(), Some(TOKEN), PathBuf::from("tmux"));
    Arc::get_mut(&mut st).unwrap().history_catalog = Arc::new(HistoryCatalog::new(
        vec![HistorySource {
            provider: "codex".to_string(),
            agent_profile: None,
            root: tmp.path().join("provider/sessions"),
        }],
        provider_history::archive_root(tmp.path()),
        provider_history::star_root(tmp.path()),
    ));
    let id = provider_history::stable_history_id("codex", None, "session-a");
    let uri = format!("/history/sessions/{id}/star");
    let record = provider_history::star_root(tmp.path()).join(format!("{id}.json"));

    let (status, body) = call(
        router(st.clone()),
        post_json(&uri, None, json!({ "starred": true })),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "body={body}");

    let (status, body) = call(
        router(st.clone()),
        post_json(&uri, Some(TOKEN), json!({ "starred": true })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert!(body["data"]["starred_at"].is_string(), "body={body}");
    assert!(record.exists(), "starring stores a star record");

    let (status, body) = call(
        router(st.clone()),
        post_json(&uri, Some(TOKEN), json!({ "starred": false })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["data"]["starred_at"], Value::Null);
    assert!(!record.exists(), "unstarring clears the star record");

    // An id that names no history session never reaches the filesystem.
    let (status, body) = call(
        router(st),
        post_json(
            "/history/sessions/..%2F..%2Fescape/star",
            Some(TOKEN),
            json!({ "starred": true }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "body={body}");
    assert!(!tmp.path().join("escape.json").exists());
}

#[test]
fn managed_history_titles_use_the_complete_provider_identity() {
    let tmp = tempfile::TempDir::new().unwrap();
    seed_resumable_session(
        tmp.path(),
        "managed-title",
        "codex",
        "hs-managed-title",
        tmp.path(),
        &["resume", "resume-session-id"],
    );
    let record_path = tmp.path().join("sessions/managed-title/session.json");
    let mut record: Value = serde_json::from_slice(&fs::read(&record_path).unwrap()).unwrap();
    record["title"] = json!("Saved Console title");
    record["runtime"]["agent_profile"] = json!("profile-a");
    fs::write(&record_path, serde_json::to_vec_pretty(&record).unwrap()).unwrap();
    let context = CliContext {
        state_dir: tmp.path().to_path_buf(),
        host: None,
    };

    let cache = StdMutex::new(ManagedHistoryTitleCache::default());
    let titles = managed_history_titles(&context, &cache);

    assert_eq!(
        titles.get(&provider_history::stable_history_id(
            "codex",
            Some("profile-a"),
            "resume-session-id",
        )),
        Some(&"Saved Console title".to_string())
    );
    assert!(!titles.contains_key(&provider_history::stable_history_id(
        "codex",
        None,
        "resume-session-id",
    )));
}

#[test]
fn managed_history_title_scan_is_cached_and_bounded() {
    let tmp = tempfile::TempDir::new().unwrap();
    for index in 0..3 {
        let id = format!("managed-title-{index}");
        let provider_session_id = format!("resume-session-id-{index}");
        seed_resumable_session(
            tmp.path(),
            &id,
            "codex",
            &format!("hs-managed-title-{index}"),
            tmp.path(),
            &["resume", &provider_session_id],
        );
        let record_path = tmp.path().join("sessions").join(&id).join("session.json");
        let mut record: Value = serde_json::from_slice(&fs::read(&record_path).unwrap()).unwrap();
        record["title"] = json!(format!("Saved title {index}"));
        record["provider_resume"]["session_id"] = json!(provider_session_id);
        fs::write(&record_path, serde_json::to_vec_pretty(&record).unwrap()).unwrap();
    }
    let context = CliContext {
        state_dir: tmp.path().to_path_buf(),
        host: None,
    };

    let (bounded, visited) =
        scan_managed_history_titles_with_stats(&context, 2, Duration::from_secs(1));
    assert_eq!(visited, 2, "the entry limit must stop before N+1");
    assert!(bounded.len() <= visited);
    assert!(
        scan_managed_history_titles(&context, usize::MAX, Duration::ZERO).is_empty(),
        "an expired deadline must stop before loading a record"
    );

    let cache = StdMutex::new(ManagedHistoryTitleCache::default());
    let initial = managed_history_titles(&context, &cache);
    let history_id = initial
        .iter()
        .find_map(|(history_id, title)| (title == "Saved title 0").then(|| history_id.clone()))
        .expect("the initial title should be indexed");

    let record_path = tmp.path().join("sessions/managed-title-0/session.json");
    let mut record: Value = serde_json::from_slice(&fs::read(&record_path).unwrap()).unwrap();
    record["title"] = json!("Changed title");
    fs::write(&record_path, serde_json::to_vec_pretty(&record).unwrap()).unwrap();
    assert_eq!(
        managed_history_titles(&context, &cache)
            .get(&history_id)
            .map(String::as_str),
        Some("Saved title 0"),
        "a fresh cache must avoid repeated record I/O"
    );
    invalidate_managed_history_titles(&cache);
    assert_eq!(
        managed_history_titles(&context, &cache)
            .get(&history_id)
            .map(String::as_str),
        Some("Changed title"),
        "invalidation must refresh title mutations"
    );

    let oversized_path = tmp.path().join("sessions/managed-title-1/session.json");
    fs::OpenOptions::new()
        .write(true)
        .open(&oversized_path)
        .unwrap()
        .set_len(MANAGED_HISTORY_TITLE_MAX_RECORD_BYTES + 1)
        .unwrap();
    assert!(!managed_history_record_is_bounded(
        oversized_path.parent().unwrap()
    ));
}

#[tokio::test]
async fn history_routes_enrich_titles_previews_and_latest_older_pages() {
    let tmp = tempfile::TempDir::new().unwrap();
    let history_root = tmp.path().join("provider/sessions/2026/09/01");
    fs::create_dir_all(&history_root).unwrap();
    fs::write(
            history_root.join("rollout.jsonl"),
            concat!(
                "{\"timestamp\":\"2026-09-01T00:00:00Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"resume-session-id\",\"cwd\":\"/work/example\",\"source\":\"cli\",\"timestamp\":\"2026-09-01T00:00:00Z\"}}\n",
                "{\"timestamp\":\"2026-09-01T00:00:01Z\",\"type\":\"event_msg\",\"payload\":{\"type\":\"thread_goal_updated\",\"threadId\":\"resume-session-id\",\"goal\":{\"threadId\":\"resume-session-id\",\"createdAt\":1788220801,\"objective\":\"Repair retitle\",\"status\":\"active\"}}}\n",
                "{\"timestamp\":\"2026-09-01T00:00:01Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"bootstrap wrapper\"}]}}\n",
                "{\"timestamp\":\"2026-09-01T00:00:02Z\",\"type\":\"event_msg\",\"payload\":{\"type\":\"user_message\",\"message\":\"first human prompt\"}}\n",
                "{\"timestamp\":\"2026-09-01T00:00:03Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"message 1\"}]}}\n",
                "{\"timestamp\":\"2026-09-01T00:00:04Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"message 2\"}]}}\n",
                "{\"timestamp\":\"2026-09-01T00:00:05Z\",\"type\":\"event_msg\",\"payload\":{\"type\":\"user_message\",\"message\":\"latest human prompt\"}}\n",
                "{\"timestamp\":\"2026-09-01T00:00:06Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"message 3\"}]}}\n",
                "{\"timestamp\":\"2026-09-01T00:00:07Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"message 4\"}]}}\n"
            ),
        )
        .unwrap();
    seed_resumable_session(
        tmp.path(),
        "managed-title",
        "codex",
        "hs-managed-title",
        tmp.path(),
        &["resume", "resume-session-id"],
    );
    let record_path = tmp.path().join("sessions/managed-title/session.json");
    let mut record: Value = serde_json::from_slice(&fs::read(&record_path).unwrap()).unwrap();
    record["title"] = json!("Saved Console title");
    fs::write(&record_path, serde_json::to_vec_pretty(&record).unwrap()).unwrap();

    let mut st = state(tmp.path(), Some(TOKEN), PathBuf::from("tmux"));
    Arc::get_mut(&mut st).unwrap().history_catalog = Arc::new(HistoryCatalog::new(
        vec![HistorySource {
            provider: "codex".to_string(),
            agent_profile: None,
            root: tmp.path().join("provider/sessions"),
        }],
        provider_history::archive_root(tmp.path()),
        provider_history::star_root(tmp.path()),
    ));

    let (status, body) = call(
        router(st.clone()),
        get_auth("/history/sessions?q=saved%20console", Some(TOKEN)),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let session = &body["data"]["sessions"][0];
    assert_eq!(session["title"], "Saved Console title");
    assert_eq!(session["first_user_prompt_preview"], "first human prompt");
    assert_eq!(session["last_user_prompt_preview"], "latest human prompt");
    let history_id = session["id"].as_str().unwrap();

    let (status, forward) = call(
        router(st.clone()),
        get_auth(
            &format!("/history/sessions/{history_id}/messages?direction=forward&limit=20"),
            Some(TOKEN),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={forward}");
    assert_eq!(forward["data"]["messages"][0]["role"], "goal");
    assert_eq!(forward["data"]["messages"][0]["text"], "Repair retitle");
    assert!(forward["data"]["messages"][0].get("payload").is_none());

    let (status, latest) = call(
        router(st.clone()),
        get_auth(
            &format!("/history/sessions/{history_id}/messages?direction=latest&limit=2"),
            Some(TOKEN),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={latest}");
    assert_eq!(latest["data"]["messages"][0]["text"], "message 3");
    assert_eq!(latest["data"]["messages"][1]["text"], "message 4");
    let older_cursor = latest["data"]["older_cursor"].as_str().unwrap();

    let (status, older) = call(
            router(st),
            get_auth(
                &format!(
                    "/history/sessions/{history_id}/messages?direction=older&limit=2&cursor={older_cursor}"
                ),
                Some(TOKEN),
            ),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "body={older}");
    assert_eq!(older["data"]["messages"][0]["text"], "message 1");
    assert_eq!(older["data"]["messages"][1]["text"], "message 2");
}

#[tokio::test]
async fn archive_is_incarnation_fenced_and_preserves_history_metadata() {
    let tmp = tempfile::TempDir::new().unwrap();
    let tmux = executable(
        &tmp.path().join("archive-tmux"),
        "#!/usr/bin/env sh\ncase \"$1\" in\n  display-message|has-session) printf '%s\\n' \"can't find session: archive-me\" >&2; exit 1 ;;\n  *) exit 42 ;;\nesac\n",
    );
    // The fake tmux is a shell script whose launch cannot guarantee fitting
    // the termination probe's bounded startup budget under load, so pin the
    // probe outcome it models: the session is gone (sympoies/nils-cli#2131).
    let _probe =
        crate::tmux_probe_fixture::install(&tmux, crate::tmux_probe_fixture::Probe::Stopped);
    seed_resumable_session(
        tmp.path(),
        "archive-me",
        "codex",
        "hs-codex-archive-me",
        tmp.path(),
        &["resume", "resume-session-id"],
    );
    let st = state(tmp.path(), Some(TOKEN), tmux);

    let (status, body) = call(
        router(st.clone()),
        post_json(
            "/sessions/archive-me/archive",
            Some(TOKEN),
            json!({ "expected_session_incarnation": "replacement" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "body={body}");
    assert!(
        tmp.path()
            .join("sessions/archive-me/session.json")
            .is_file()
    );

    let (status, body) = call(
        router(st),
        post_json(
            "/sessions/archive-me/archive",
            Some(TOKEN),
            json!({ "expected_session_incarnation": "never-launched-fixture" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(
        body["data"]["archived"]["provider_session_id"],
        "resume-session-id"
    );
    // History resume reuses this id so peers addressing it still reach it.
    assert_eq!(body["data"]["archived"]["session_id"], "archive-me");
    assert!(!tmp.path().join("sessions/archive-me").exists());
    let archive = tmp.path().join("history/archives").join(format!(
        "{}.json",
        body["data"]["archived"]["history_id"].as_str().unwrap()
    ));
    assert!(archive.is_file());
    assert_eq!(
        std::fs::metadata(archive).unwrap().permissions().mode() & 0o777,
        0o600
    );
    // Only the successful archive removed the record: one closed entry.
    let context = CliContext {
        state_dir: tmp.path().to_path_buf(),
        host: None,
    };
    assert_eq!(
        crate::board::closed_reasons_for_test(&context),
        vec![("archive-me".to_string(), "archived".to_string())]
    );
    assert_serve_lifecycle(tmp.path(), "archive-me", &[("archive", 2)]);
}

#[tokio::test]
async fn archive_termination_failure_retains_session_without_history_metadata() {
    let tmp = tempfile::TempDir::new().unwrap();
    let id = "archive-stop-failure";
    let tmux_session = "hs-codex-archive-stop-failure";
    seed_session_with_runtime(tmp.path(), id, "codex", tmux_session);
    let record_path = tmp.path().join("sessions").join(id).join("session.json");
    let mut record: Value =
        serde_json::from_str(&std::fs::read_to_string(&record_path).unwrap()).unwrap();
    record["provider_resume"] = json!({
        "provider": "codex",
        "session_id": "archive-stop-failure-provider",
        "captured_at": "2000-01-01T00:00:00Z",
        "capture_method": "fixture",
        "resume_args": ["resume", "archive-stop-failure-provider"]
    });
    std::fs::write(&record_path, serde_json::to_vec_pretty(&record).unwrap()).unwrap();

    let pane = TestProcessGroup::spawn();
    let calls = tmp.path().join("archive-stop-failure.calls");
    let tmux = failing_delete_tmux(tmp.path(), tmp.path(), id, pane.pid(), &calls);
    let st = state(tmp.path(), Some(TOKEN), tmux);
    let history_id =
        provider_history::stable_history_id("codex", None, "archive-stop-failure-provider");
    let archive_path =
        provider_history::archive_root(&st.context.state_dir).join(format!("{history_id}.json"));

    let (status, body) = call(
        router(st),
        post_json(
            &format!("/sessions/{id}/archive"),
            Some(TOKEN),
            json!({ "expected_session_incarnation": format!("launch-{id}") }),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body={body}");
    assert_eq!(body["error"]["code"], "session-termination-failed");
    assert!(record_path.is_file());
    assert!(
        !archive_path.exists(),
        "a live or unverified runtime must never gain visible archive metadata"
    );
}

#[tokio::test]
async fn writes_require_token() {
    let tmp = tempfile::TempDir::new().unwrap();
    let st = state(tmp.path(), Some(TOKEN), PathBuf::from("tmux"));

    // POST create without a token.
    let (status, body) = call(
        router(st.clone()),
        post_json("/sessions", None, json!({"agent": "codex"})),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"]["code"], "unauthorized");

    // DELETE without a token.
    let del = Request::builder()
        .method("DELETE")
        .uri("/sessions/abc")
        .body(Body::empty())
        .unwrap();
    let (status, _) = call(router(st.clone()), del).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // Wrong token is rejected.
    let (status, _) = call(
        router(st.clone()),
        post_json("/sessions", Some("nope"), json!({"agent": "codex"})),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn http_session_routes_reject_encoded_path_traversal_before_state_access() {
    let tmp = tempfile::TempDir::new().unwrap();
    let outside = tmp.path().join("outside");
    fs::create_dir_all(&outside).unwrap();
    let sentinel = outside.join("sentinel");
    fs::write(&sentinel, "preserve").unwrap();
    let st = state(tmp.path(), Some(TOKEN), minimal_tmux(tmp.path()));
    let encoded_id = "%2E%2E%2Foutside";

    let requests = [
        get_auth(&format!("/sessions/{encoded_id}/buffer"), Some(TOKEN)),
        post_json(
            &format!("/sessions/{encoded_id}/prompt"),
            Some(TOKEN),
            json!({ "text": "ignored" }),
        ),
        post_json(
            &format!("/sessions/{encoded_id}/resume"),
            Some(TOKEN),
            json!({}),
        ),
        patch_json(
            &format!("/sessions/{encoded_id}"),
            Some(TOKEN),
            json!({ "title": "ignored" }),
        ),
        Request::builder()
            .method("DELETE")
            .uri(format!("/sessions/{encoded_id}"))
            .header(AUTHORIZATION, format!("Bearer {TOKEN}"))
            .body(Body::empty())
            .unwrap(),
    ];

    for request in requests {
        let (status, body) = call(router(st.clone()), request).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "body={body}");
        assert_eq!(body["error"]["code"], "invalid-session-id");
    }
    assert_eq!(fs::read_to_string(sentinel).unwrap(), "preserve");
}

#[tokio::test]
async fn http_create_keeps_shell_metacharacters_in_literal_agent_argv() {
    let lock = GlobalStateLock::new();
    let _unpinned = crate::agent_bin::unpinned_provider_env(&lock);
    let tmp = tempfile::TempDir::new().unwrap();
    let cwd = tmp.path().join("repo");
    fs::create_dir_all(&cwd).unwrap();
    let injected = tmp.path().join("must-not-exist");
    let literal_arg = format!("fixture; $(touch {})", injected.display());
    let st = state(tmp.path(), Some(TOKEN), minimal_tmux(tmp.path()));

    let (status, body) = call(
        router(st.clone()),
        post_json(
            "/sessions",
            Some(TOKEN),
            json!({
                "agent": "codex",
                "id": "literal-agent-argv",
                "cwd": cwd,
                "agent_args": ["--model", literal_arg]
            }),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body={body}");
    assert!(!injected.exists(), "agent argv was interpreted by a shell");
    let record = load_session_record(&st.context, "literal-agent-argv").unwrap();
    assert_eq!(record.agent_args, ["--model", literal_arg.as_str()]);
}

#[test]
fn default_codex_launch_resolves_and_records_the_current_account() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().unwrap();
    let broker = executable(
        &tmp.path().join("broker"),
        "#!/usr/bin/env sh\ncase \"$1\" in\n  list) printf '%s\\n' '{\"schema_version\":\"agent-session.codex-auth-broker.v1\",\"accounts\":[{\"account\":\"account-a\"},{\"account\":\"account-b\"}],\"selection_strategies\":[\"current_default\"]}' ;;\n  select) printf '%s\\n' '{\"schema_version\":\"agent-session.codex-auth-broker.v1\",\"account\":\"account-a\"}' ;;\n  *) exit 1 ;;\nesac\n",
    );
    let broker_argv = serde_json::to_string(&vec![broker.to_string_lossy().into_owned()]).unwrap();
    let _broker = EnvGuard::set(&lock, "AGENT_SESSION_CODEX_ACCOUNT_BROKER", &broker_argv);

    let resolved = resolve_initial_codex_account(AgentKind::Codex, None).unwrap();
    assert_eq!(resolved.0.as_deref(), Some("account-a"));
    assert_eq!(resolved.1.as_deref(), Some("default_at_launch"));

    let explicit =
        resolve_initial_codex_account(AgentKind::Codex, Some("account-b".to_string())).unwrap();
    assert_eq!(explicit.0.as_deref(), Some("account-b"));
    assert_eq!(explicit.1.as_deref(), Some("explicit"));
    // An explicit account is validated and must be listed before launch.
    for (account, code) in [
        ("delta", "codex-account-unknown"),
        ("--format", "invalid-codex-account"),
    ] {
        assert_eq!(
            resolve_initial_codex_account(AgentKind::Codex, Some(account.to_string()))
                .unwrap_err()
                .code(),
            code
        );
    }

    let old_broker = executable(
        &tmp.path().join("old-broker"),
        "#!/usr/bin/env sh\nprintf '%s\\n' '{\"schema_version\":\"agent-session.codex-auth-broker.v1\",\"accounts\":[]}'\n",
    );
    let old_argv = serde_json::to_string(&vec![old_broker.to_string_lossy().into_owned()]).unwrap();
    let _old = EnvGuard::set(&lock, "AGENT_SESSION_CODEX_ACCOUNT_BROKER", &old_argv);
    assert_eq!(
        resolve_initial_codex_account(AgentKind::Codex, None).unwrap(),
        (None, None)
    );

    // A broker failure while choosing the automatic default leaves the
    // session unbound; an explicit account still fails closed.
    // A broker that runs and exits non-zero (`/bin/false` is not on macOS).
    let failing_broker = executable(
        &tmp.path().join("failing-broker"),
        "#!/usr/bin/env sh\nexit 1\n",
    );
    let failing_argv =
        serde_json::to_string(&vec![failing_broker.to_string_lossy().into_owned()]).unwrap();
    let _failing = EnvGuard::set(&lock, "AGENT_SESSION_CODEX_ACCOUNT_BROKER", &failing_argv);
    assert_eq!(
        resolve_initial_codex_account(AgentKind::Codex, None).unwrap(),
        (None, None)
    );
    assert_eq!(
        resolve_initial_codex_account(AgentKind::Codex, Some("account-b".to_string()))
            .unwrap_err()
            .code(),
        "codex-account-broker-rejected"
    );
    let _unset = EnvGuard::remove(&lock, "AGENT_SESSION_CODEX_ACCOUNT_BROKER");
    let unsupported =
        resolve_initial_codex_account(AgentKind::Codex, Some("account-b".to_string()))
            .unwrap_err()
            .into_inner();
    assert_eq!(unsupported.code, "codex-account-unsupported");
    // One error kind for the condition, whichever Codex path reports it.
    assert_eq!(unsupported.exit_code, exit::DATA);
    assert_eq!(
        crate::codex_account::list_accounts()
            .unwrap_err()
            .into_inner()
            .exit_code,
        exit::DATA
    );
}

#[tokio::test]
async fn auto_resume_control_is_authenticated_durable_and_projected() {
    let tmp = tempfile::TempDir::new().unwrap();
    seed_session(tmp.path(), "claude-reset", "claude", "hs-claude-reset");
    let st = state(tmp.path(), Some(TOKEN), minimal_tmux(tmp.path()));

    let (status, body) = call(
        router(st.clone()),
        put_json(
            "/sessions/claude-reset/auto-resume",
            None,
            json!({"enabled": true}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "body={body}");

    let (status, body) = call(
        router(st.clone()),
        put_json(
            "/sessions/claude-reset/auto-resume",
            Some(TOKEN),
            json!({"enabled": true}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["data"]["auto_resume"]["enabled"], true);
    assert_eq!(body["data"]["auto_resume"]["state"], "enabled");
    assert_eq!(body["data"]["auto_resume"]["supported"], true);

    let (status, body) = call(router(st.clone()), get("/sessions")).await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let projected = &body["data"]["sessions"][0]["auto_resume"];
    assert_eq!(projected["enabled"], true);
    assert_eq!(projected["state"], "enabled");
    assert!(projected.get("blocked_turn_id").is_none());

    let request = Request::builder()
        .method("DELETE")
        .uri("/sessions/claude-reset/auto-resume")
        .header("authorization", format!("Bearer {TOKEN}"))
        .body(Body::empty())
        .unwrap();
    let (status, body) = call(router(st.clone()), request).await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["data"]["auto_resume"]["enabled"], false);
    assert_eq!(body["data"]["auto_resume"]["state"], "cancelled");

    let persisted = fs::read_to_string(tmp.path().join("sessions/claude-reset/auto-resume.json"))
        .expect("durable auto-resume state");
    assert!(!persisted.contains("prompt"));
    assert!(!persisted.contains("transcript"));
}

#[tokio::test]
async fn codex_auto_resume_projects_the_account_failover_policy() {
    let lock = GlobalStateLock::new();
    let _broker = EnvGuard::set(
        &lock,
        "AGENT_SESSION_CODEX_ACCOUNT_BROKER",
        r#"["/configured/broker"]"#,
    );
    let tmp = tempfile::TempDir::new().unwrap();
    let launch_id = seed_codex_app_server_session(tmp.path(), "codex-failover-policy");
    let st = state(tmp.path(), Some(TOKEN), minimal_tmux(tmp.path()));
    let mut record = load_session_record(&st.context, "codex-failover-policy").unwrap();
    crate::codex_account::set_initial_binding_with_source(
        &mut record,
        Some("account-a"),
        Some("default_at_launch"),
    )
    .unwrap();
    crate::write_session_record(&st.context, &record).unwrap();
    crate::codex_account::finish_binding(
        &st.context,
        &record.id,
        &launch_id,
        "account-a",
        1,
        Ok(()),
    )
    .unwrap();

    let (status, body) = call(
        router(st),
        put_json(
            "/sessions/codex-failover-policy/auto-resume",
            Some(TOKEN),
            json!({
                "enabled": true,
                "recovery_policy": "next_account_then_resume"
            }),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(
        body["data"]["auto_resume"]["recovery_policy"],
        "next_account_then_resume"
    );
}

#[tokio::test]
async fn codex_scheduler_selects_a_healthy_account_after_an_open_window_rejection() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().unwrap();
    let broker = executable(
        &tmp.path().join("broker"),
        "#!/bin/sh\nprintf '%s\\n' '{\"schema_version\":\"agent-session.codex-auth-broker.v1\",\"account\":\"account-b\"}'\n",
    );
    let broker_argv = serde_json::to_string(&vec![broker]).unwrap();
    let _broker = EnvGuard::set(&lock, "AGENT_SESSION_CODEX_ACCOUNT_BROKER", &broker_argv);
    let launch_id = seed_codex_app_server_session(tmp.path(), "codex-open-failover");
    let st = state(tmp.path(), Some(TOKEN), minimal_tmux(tmp.path()));
    let mut record = load_session_record(&st.context, "codex-open-failover").unwrap();
    crate::codex_account::set_initial_binding(&mut record, Some("account-a")).unwrap();
    crate::write_session_record(&st.context, &record).unwrap();
    crate::codex_account::finish_binding(
        &st.context,
        &record.id,
        &launch_id,
        "account-a",
        1,
        Ok(()),
    )
    .unwrap();
    record = load_session_record(&st.context, &record.id).unwrap();
    crate::activity::activate_runtime(&st.context, &record).unwrap();
    crate::codex_account::authorize_input_locked(&st.context, &mut record).unwrap();
    auto_resume::set_enabled_with_policy(
        &st.context,
        &record.id,
        true,
        auto_resume::NEXT_ACCOUNT_THEN_RESUME_POLICY,
        "2030-01-01T00:00:00Z",
    )
    .unwrap();
    crate::activity::ingest_codex_app_server_failure(
        &st.context,
        &record.id,
        &launch_id,
        "thread-open-failover",
        "turn-open-failover",
    )
    .unwrap();
    let (handle, mut commands) = codex_app_server::control_channel();
    st.codex_controls.lock().unwrap().insert(
        record.id.clone(),
        CodexControlEntry {
            launch_id: launch_id.clone(),
            handle,
        },
    );
    let responder = tokio::spawn(async move {
        let Some(codex_app_server::ControlCommand::Usage(response)) = commands.recv().await else {
            panic!("scheduler did not request current Codex usage");
        };
        response
            .send(Ok(UsageSnapshot {
                authoritative: true,
                has_exhausted_windows: false,
                exhausted_reset_epochs: Vec::new(),
                soonest_reset_epoch: None,
            }))
            .unwrap();
    });

    process_codex_auto_resume_id(
        st.clone(),
        CodexAutoResumeTarget {
            id: record.id.clone(),
            launch_id,
            binding: crate::codex_account::binding_snapshot(&record),
        },
    )
    .await;
    responder.await.unwrap();

    assert_eq!(
        auto_resume::view_for_record(&st.context, &record).state,
        "switching_account"
    );
    let account = crate::codex_account::view_for_record(
        &load_session_record(&st.context, &record.id).unwrap(),
    );
    assert_eq!(
        account.next.and_then(|next| next.account).as_deref(),
        Some("account-b")
    );
}

#[tokio::test]
async fn codex_capacity_scheduler_submits_the_fixed_prompt_without_usage_lookup() {
    let tmp = tempfile::TempDir::new().unwrap();
    let launch_id = seed_codex_app_server_session(tmp.path(), "codex-capacity-recovery");
    let st = state(tmp.path(), Some(TOKEN), minimal_tmux(tmp.path()));
    let record = load_session_record(&st.context, "codex-capacity-recovery").unwrap();
    crate::activity::activate_runtime(&st.context, &record).unwrap();
    let failure = crate::activity::ingest_codex_app_server_failure_with_kind(
        &st.context,
        &record.id,
        &launch_id,
        "thread-capacity",
        "turn-capacity",
        codex_app_server::StructuredFailureKind::ProviderCapacity,
    )
    .unwrap();
    let blocked_turn_id = failure
        .turn_state
        .last_turn
        .and_then(|turn| turn.provider_turn_id)
        .unwrap();
    auto_resume::set_enabled(&st.context, &record.id, true, "2020-01-01T00:00:00Z").unwrap();
    let revision = crate::activity::state_for_view(&st.context, &record)
        .unwrap()
        .revision;
    auto_resume::arm_provider_capacity(
        &st.context,
        &record.id,
        &launch_id,
        &crate::codex_account::binding_snapshot(&record),
        blocked_turn_id,
        revision,
        "2020-01-01T00:00:01Z",
    )
    .unwrap();

    let (handle, mut commands) = codex_app_server::control_channel();
    st.codex_controls.lock().unwrap().insert(
        record.id.clone(),
        CodexControlEntry {
            launch_id: launch_id.clone(),
            handle,
        },
    );
    let responder = tokio::spawn(async move {
        let Some(codex_app_server::ControlCommand::Continue { message, response }) =
            commands.recv().await
        else {
            panic!("capacity recovery must submit directly without requesting usage");
        };
        assert_eq!(message, auto_resume::CAPACITY_CONTINUATION_MESSAGE);
        response.send(Ok("continued-turn".to_string())).unwrap();
    });

    process_codex_capacity_auto_resume_id(
        st.clone(),
        CodexAutoResumeTarget {
            id: record.id.clone(),
            launch_id,
            binding: crate::codex_account::binding_snapshot(&record),
        },
    )
    .await;
    responder.await.unwrap();

    assert_eq!(
        auto_resume::view_for_record(&st.context, &record).state,
        "resumed"
    );
}

#[tokio::test]
async fn codex_capacity_scheduler_retries_missing_or_stale_control_without_submitting() {
    let tmp = tempfile::TempDir::new().unwrap();
    let launch_id = seed_codex_app_server_session(tmp.path(), "codex-capacity-no-control");
    let st = state(tmp.path(), Some(TOKEN), minimal_tmux(tmp.path()));
    let record = load_session_record(&st.context, "codex-capacity-no-control").unwrap();
    crate::activity::activate_runtime(&st.context, &record).unwrap();
    let failure = crate::activity::ingest_codex_app_server_failure_with_kind(
        &st.context,
        &record.id,
        &launch_id,
        "thread-capacity",
        "turn-capacity",
        codex_app_server::StructuredFailureKind::ProviderCapacity,
    )
    .unwrap();
    let blocked_turn_id = failure
        .turn_state
        .last_turn
        .and_then(|turn| turn.provider_turn_id)
        .unwrap();
    auto_resume::set_enabled(&st.context, &record.id, true, "2020-01-01T00:00:00Z").unwrap();
    let revision = crate::activity::state_for_view(&st.context, &record)
        .unwrap()
        .revision;
    let binding = crate::codex_account::binding_snapshot(&record);
    auto_resume::arm_provider_capacity(
        &st.context,
        &record.id,
        &launch_id,
        &binding,
        blocked_turn_id.clone(),
        revision,
        "2020-01-01T00:00:01Z",
    )
    .unwrap();

    let (stale_handle, mut stale_commands) = codex_app_server::control_channel();
    st.codex_controls.lock().unwrap().insert(
        record.id.clone(),
        CodexControlEntry {
            launch_id: "stale-launch".to_string(),
            handle: stale_handle,
        },
    );
    process_codex_capacity_auto_resume_id(
        st.clone(),
        CodexAutoResumeTarget {
            id: record.id.clone(),
            launch_id: launch_id.clone(),
            binding: binding.clone(),
        },
    )
    .await;
    assert!(stale_commands.try_recv().is_err());
    let stale_view = auto_resume::view_for_record(&st.context, &record);
    assert_eq!(stale_view.state, "transient_failure");
    assert_eq!(
        stale_view.failure_reason.as_deref(),
        Some("control_unavailable")
    );

    auto_resume::set_enabled(&st.context, &record.id, true, "2020-01-01T00:01:00Z").unwrap();
    auto_resume::arm_provider_capacity(
        &st.context,
        &record.id,
        &launch_id,
        &binding,
        blocked_turn_id,
        revision,
        "2020-01-01T00:01:01Z",
    )
    .unwrap();
    st.codex_controls.lock().unwrap().remove(&record.id);
    process_codex_capacity_auto_resume_id(
        st.clone(),
        CodexAutoResumeTarget {
            id: record.id.clone(),
            launch_id,
            binding,
        },
    )
    .await;
    let missing_view = auto_resume::view_for_record(&st.context, &record);
    assert_eq!(missing_view.state, "transient_failure");
    assert_eq!(
        missing_view.failure_reason.as_deref(),
        Some("control_unavailable")
    );
    assert_eq!(
        auto_resume::pending_sessions(&st.context, i64::MAX)
            .unwrap()
            .capacity_ids,
        vec![record.id]
    );
}

#[tokio::test]
async fn auto_resume_put_authenticates_before_returning_stable_body_errors() {
    let tmp = tempfile::TempDir::new().unwrap();
    seed_session(tmp.path(), "claude-reset", "claude", "hs-claude-reset");
    let st = state(tmp.path(), Some(TOKEN), minimal_tmux(tmp.path()));
    let uri = "/sessions/claude-reset/auto-resume";

    let unauthenticated = Request::builder()
        .method("PUT")
        .uri(uri)
        .header(CONTENT_TYPE, "application/json")
        .body(Body::from("{"))
        .unwrap();
    let (status, body) = call(router(st.clone()), unauthenticated).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "body={body}");
    assert_eq!(body["error"]["code"], "unauthorized");

    for (content_type, body_text) in [
        (Some("application/json"), "{"),
        (None, r#"{"enabled":true}"#),
        (Some("application/json"), "{}"),
        (Some("application/json"), r#"{"enabled":"yes"}"#),
    ] {
        let mut request = Request::builder()
            .method("PUT")
            .uri(uri)
            .header(AUTHORIZATION, format!("Bearer {TOKEN}"));
        if let Some(content_type) = content_type {
            request = request.header(CONTENT_TYPE, content_type);
        }
        let (status, body) = call(
            router(st.clone()),
            request.body(Body::from(body_text)).unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "body={body}");
        assert_eq!(body["schema_version"], "cli.agent-session.serve.v1");
        assert_eq!(body["error"]["code"], "invalid-json-body");
    }

    let unconfigured = state(tmp.path(), None, PathBuf::from("tmux"));
    let request = Request::builder()
        .method("PUT")
        .uri(uri)
        .header(CONTENT_TYPE, "application/json")
        .body(Body::from("{"))
        .unwrap();
    let (status, body) = call(router(unconfigured), request).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "body={body}");
    assert_eq!(body["error"]["code"], "token-not-configured");
}

#[tokio::test]
async fn codex_auto_resume_fails_closed_without_authoritative_failure_signal() {
    let tmp = tempfile::TempDir::new().unwrap();
    seed_session(tmp.path(), "codex-reset", "codex", "hs-codex-reset");
    let st = state(tmp.path(), Some(TOKEN), minimal_tmux(tmp.path()));

    let (status, body) = call(
        router(st.clone()),
        put_json(
            "/sessions/codex-reset/auto-resume",
            Some(TOKEN),
            json!({"enabled": true}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "body={body}");
    assert_eq!(body["error"]["code"], "auto-resume-unsupported");
}

#[tokio::test(start_paused = true)]
async fn codex_scheduler_does_not_serialize_independent_control_timeouts() {
    let tmp = tempfile::TempDir::new().unwrap();
    let st = state(tmp.path(), Some(TOKEN), minimal_tmux(tmp.path()));
    let (slow_handle, _slow_commands) = codex_app_server::control_channel();
    let (fast_handle, mut fast_commands) = codex_app_server::control_channel();
    st.codex_controls.lock().unwrap().extend([
        (
            "slow".to_string(),
            CodexControlEntry {
                launch_id: "slow-launch".to_string(),
                handle: slow_handle,
            },
        ),
        (
            "fast".to_string(),
            CodexControlEntry {
                launch_id: "fast-launch".to_string(),
                handle: fast_handle,
            },
        ),
    ]);
    let (seen, observed) = tokio::sync::oneshot::channel();
    let responder = tokio::spawn(async move {
        let Some(codex_app_server::ControlCommand::Usage(response)) = fast_commands.recv().await
        else {
            panic!("fast control did not receive a usage request");
        };
        let _ = seen.send(());
        let _ = response.send(Ok(UsageSnapshot {
            authoritative: false,
            has_exhausted_windows: false,
            exhausted_reset_epochs: Vec::new(),
            soonest_reset_epoch: None,
        }));
    });
    let scheduler = tokio::spawn(process_codex_auto_resume_ids(
        st,
        vec![
            CodexAutoResumeTarget {
                id: "slow".to_string(),
                launch_id: "slow-launch".to_string(),
                binding: crate::codex_account::BindingSnapshot::Unbound,
            },
            CodexAutoResumeTarget {
                id: "fast".to_string(),
                launch_id: "fast-launch".to_string(),
                binding: crate::codex_account::BindingSnapshot::Unbound,
            },
        ],
    ));

    tokio::time::timeout(Duration::from_millis(1), observed)
        .await
        .expect("fast control waited behind the slow control")
        .unwrap();
    responder.await.unwrap();
    scheduler.abort();
    let _ = scheduler.await;
}

#[tokio::test(start_paused = true)]
async fn codex_account_reconcile_does_not_serialize_independent_control_timeouts() {
    let (slow_handle, _slow_commands) = codex_app_server::control_channel();
    let (fast_handle, mut fast_commands) = codex_app_server::control_channel();
    let (seen, observed) = tokio::sync::oneshot::channel();
    let responder = tokio::spawn(async move {
        let Some(codex_app_server::ControlCommand::ApplyNext { response }) =
            fast_commands.recv().await
        else {
            panic!("fast control did not receive an account reconcile request");
        };
        let _ = seen.send(());
        let _ = response.send(Ok(()));
    });
    let reconcile = tokio::spawn(apply_pending_codex_accounts(vec![slow_handle, fast_handle]));

    tokio::time::timeout(Duration::from_millis(1), observed)
        .await
        .expect("fast account reconcile waited behind the slow control")
        .unwrap();
    responder.await.unwrap();
    reconcile.abort();
    let _ = reconcile.await;
}

#[tokio::test]
async fn missing_codex_control_cannot_checkpoint_a_same_id_replacement() {
    let tmp = tempfile::TempDir::new().unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    seed_session(&context.state_dir, "replacement", "codex", "hs-replacement");
    let mut record = load_session_record(&context, "replacement").unwrap();
    let (_socket_dir, socket) = nils_test_support::short_socket_path("replacement.sock");
    record.runtime = Some(crate::RuntimeInfo {
        kind: codex_app_server::RUNTIME_KIND.to_string(),
        tmux_session: record.tmux_session.clone(),
        generation: 1,
        started_at: "2030-01-01T00:00:00Z".to_string(),
        launch_id: "launch-a".to_string(),
        extra: std::collections::BTreeMap::from([
            (
                codex_app_server::PROTOCOL_KEY.to_string(),
                json!(codex_app_server::PROTOCOL_VERSION),
            ),
            (
                codex_app_server::SOCKET_KEY.to_string(),
                json!(crate::display_path(&socket)),
            ),
            (
                codex_app_server::PROXY_KEY.to_string(),
                json!(crate::display_path(&socket.with_extension("proxy"))),
            ),
            (
                codex_app_server::THREAD_HANDOFF_KEY.to_string(),
                json!(crate::display_path(&socket.with_extension("thread"))),
            ),
            (
                codex_app_server::THREAD_ATTACHED_KEY.to_string(),
                json!(crate::display_path(&socket.with_extension("attached"))),
            ),
        ]),
    });
    crate::write_session_record(&context, &record).unwrap();
    crate::activity::activate_runtime(&context, &record).unwrap();
    auto_resume::set_enabled(&context, &record.id, true, "2030-01-01T00:00:00Z").unwrap();
    crate::activity::ingest_codex_app_server_failure(
        &context, &record.id, "launch-a", "thread-a", "turn-a",
    )
    .unwrap();
    assert_eq!(
        auto_resume::tick_for_runtime(
            &context,
            &record.id,
            "launch-a",
            1_893_456_000,
            &UsageSnapshot {
                authoritative: true,
                has_exhausted_windows: true,
                exhausted_reset_epochs: vec![1_893_456_600],
                soonest_reset_epoch: None,
            },
            |_| panic!("blocked usage must not submit"),
        )
        .unwrap(),
        auto_resume::TickOutcome::Scheduled
    );

    let target = CodexAutoResumeTarget {
        id: record.id.clone(),
        launch_id: "launch-a".to_string(),
        binding: crate::codex_account::binding_snapshot(&record),
    };
    record.runtime.as_mut().unwrap().launch_id = "launch-b".to_string();
    crate::write_session_record(&context, &record).unwrap();
    let st = state(&context.state_dir, Some(TOKEN), minimal_tmux(tmp.path()));
    process_codex_auto_resume_id(st, target).await;

    let view = auto_resume::view_for_record(&context, &record);
    assert_eq!(view.state, "scheduled");
    assert!(view.failure_reason.is_none());
}

#[test]
fn codex_control_discovery_uses_one_bulk_tmux_snapshot() {
    let tmp = tempfile::TempDir::new().unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    for (id, tmux_session) in [("first", "hs-first"), ("second", "hs-second")] {
        seed_session(&context.state_dir, id, "codex", tmux_session);
        let mut record = load_session_record(&context, id).unwrap();
        let (_socket_dir, socket) = nils_test_support::short_socket_path(format!("{id}.sock"));
        record.runtime = Some(crate::RuntimeInfo {
            kind: codex_app_server::RUNTIME_KIND.to_string(),
            tmux_session: tmux_session.to_string(),
            generation: 1,
            started_at: "2030-01-01T00:00:00Z".to_string(),
            launch_id: format!("launch-{id}"),
            extra: std::collections::BTreeMap::from([
                (
                    codex_app_server::PROTOCOL_KEY.to_string(),
                    json!(codex_app_server::PROTOCOL_VERSION),
                ),
                (
                    codex_app_server::SOCKET_KEY.to_string(),
                    json!(crate::display_path(&socket)),
                ),
                (
                    codex_app_server::PROXY_KEY.to_string(),
                    json!(crate::display_path(&socket.with_extension("proxy"))),
                ),
                (
                    codex_app_server::THREAD_HANDOFF_KEY.to_string(),
                    json!(crate::display_path(&socket.with_extension("thread"))),
                ),
                (
                    codex_app_server::THREAD_ATTACHED_KEY.to_string(),
                    json!(crate::display_path(&socket.with_extension("attached"))),
                ),
            ]),
        });
        crate::write_session_record(&context, &record).unwrap();
    }
    let calls = tmp.path().join("tmux.calls");
    let tmux = executable(
        &tmp.path().join("tmux-bulk"),
        &format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> {}\nif [ \"$1\" = list-windows ]; then printf 'hs-first\\t100\\nhs-second\\t100\\n'; exit 0; fi\nexit 1\n",
            shell_words::quote(&calls.to_string_lossy())
        ),
    );

    let records = discover_codex_controls(&context, &tmux);
    assert_eq!(records.len(), 2);
    let calls = fs::read_to_string(calls).unwrap();
    assert_eq!(calls.lines().count(), 1, "calls={calls}");
    assert!(calls.contains("list-windows -a -F"));
    assert!(!calls.contains("has-session"));
}

#[tokio::test]
async fn lifecycle_review_deleted_runtime_helper_returns_reachable_failure() {
    let lock = GlobalStateLock::new();
    let _without_broker = EnvGuard::remove(&lock, "AGENT_SESSION_CODEX_ACCOUNT_BROKER");
    let tmp = tempfile::TempDir::new().unwrap();
    let cwd = tmp.path().join("repo");
    let runtime_dir = tempfile::Builder::new()
        .prefix("cx-")
        .tempdir_in("/tmp")
        .unwrap();
    fs::set_permissions(runtime_dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
    fs::create_dir(&cwd).unwrap();
    let codex = executable(
        &tmp.path().join("codex-app-server"),
        "#!/usr/bin/env sh\nif [ \"$1\" = --version ]; then printf '%s\\n' 'codex-cli 0.145.0'; exit 0; fi\nif [ \"$1\" = app-server ] && [ \"$2\" = --help ]; then printf '%s\\n' '  --listen <URL>  unix://'; exit 0; fi\nexit 1\n",
    );
    let missing_helper = tmp.path().join("token-secret-deleted-agent-session");
    let _codex_bin = EnvGuard::set(&lock, "AGENT_SESSION_CODEX_BIN", codex.to_str().unwrap());
    let _runtime_dir = EnvGuard::set(
        &lock,
        "XDG_RUNTIME_DIR",
        runtime_dir.path().to_str().unwrap(),
    );
    let _runtime_mode = EnvGuard::set(&lock, "AGENT_SESSION_CODEX_RUNTIME", "app-server");
    let _runtime_helper = EnvGuard::set(
        &lock,
        "AGENT_SESSION_TEST_RUNTIME_HELPER",
        missing_helper.to_str().unwrap(),
    );
    let _capture_timeout = EnvGuard::set(&lock, "AGENT_SESSION_CODEX_CAPTURE_TIMEOUT_MS", "10");
    let log = tmp.path().join("tmux.log");
    let st = state(tmp.path(), Some(TOKEN), logging_tmux(tmp.path(), &log));

    let (create_status, create_body) = call(
        router(st.clone()),
        post_json(
            "/sessions",
            Some(TOKEN),
            json!({
                "agent": "codex",
                "id": "deleted-helper",
                "cwd": cwd.to_string_lossy()
            }),
        ),
    )
    .await;

    assert_eq!(
        create_status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "body={create_body}"
    );
    assert_eq!(
        create_body["error"]["code"],
        "codex-app-server-proxy-binary-unavailable"
    );
    assert_eq!(
        create_body["error"]["message"],
        "the agent-session runtime helper is unavailable"
    );
    let record_path = tmp.path().join("sessions/deleted-helper/session.json");
    let record: Value = serde_json::from_slice(&fs::read(&record_path).unwrap()).unwrap();
    let startup = &record["startup"];
    assert_eq!(
        create_body["error"]["details"]["session_id"],
        "deleted-helper"
    );
    assert_eq!(create_body["error"]["details"]["startup"], *startup);
    assert_eq!(startup["state"], "failed");
    assert_eq!(startup["stage"], "proxy");
    assert_eq!(startup["failure_code"], "runtime-helper-unavailable");
    assert_eq!(
        startup["message"],
        "Session runtime helper is unavailable after an upgrade."
    );
    assert!(!create_body.to_string().contains("token-secret"));
    assert!(!log.exists() || fs::read_to_string(&log).unwrap().is_empty());

    let journal = crate::lifecycle::read(&st.context, "deleted-helper", 10).unwrap();
    assert_eq!(
        journal["records"][0]["result"]["code"],
        create_body["error"]["code"]
    );
    assert_eq!(journal["records"][0]["caller"]["kind"], "serve");
    let (list_status, list_body) = call(router(st.clone()), get("/sessions")).await;
    assert_eq!(list_status, StatusCode::OK, "body={list_body}");
    assert_eq!(list_body["data"]["sessions"][0]["startup"], *startup);
    let (glance_status, glance_body) =
        call(router(st), get("/sessions/deleted-helper/glance")).await;
    assert_eq!(glance_status, StatusCode::OK, "body={glance_body}");
    assert_eq!(glance_body["data"]["glance"]["startup"], *startup);
}

#[tokio::test]
async fn lifecycle_review_create_failure_retains_cleanup_projection() {
    let tmp = tempfile::TempDir::new().unwrap();
    let launcher = fake_agent(tmp.path(), "fixture-provider");
    let tmux = executable(
        &tmp.path().join("failed-tmux"),
        "#!/bin/sh\ncase \"$1\" in\n new-session) exit 0 ;;\n display-message) printf '%s\\n' 'malformed identity'; exit 0 ;;\n has-session) exit 0 ;;\n *) exit 0 ;;\nesac\n",
    );
    let profile = AgentLaunchProfiles::from_json(
        &json!([{
            "id":"fixture-profile", "label":"Fixture", "agent":"claude", "agent_bin":launcher
        }])
        .to_string(),
    )
    .unwrap();
    let mut st = state(tmp.path(), Some(TOKEN), tmux);
    Arc::get_mut(&mut st).unwrap().launch_profiles = profile;
    let (status, body) = call(router(st), post_json("/sessions", Some(TOKEN), json!({
            "id":"cleanup-failure", "agent":"claude", "agent_profile":"fixture-profile", "cwd":tmp.path()
        }))).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{body}");
    let retained: Value = serde_json::from_slice(
        &fs::read(tmp.path().join("sessions/cleanup-failure/session.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(body["error"]["details"]["session_id"], "cleanup-failure");
    assert_eq!(body["error"]["details"]["startup"], retained["startup"]);
    assert!(retained["startup"]["cleanup"].is_object(), "{retained}");
    assert_eq!(
        body["error"]["details"]["cleanup"],
        retained["startup"]["cleanup"]
    );
    assert_serve_lifecycle(tmp.path(), "cleanup-failure", &[("create", 1)]);
}

#[tokio::test]
async fn stale_proxy_helper_failure_is_durable_in_list_and_glance() {
    let tmp = tempfile::TempDir::new().unwrap();
    let id = "stale-helper";
    seed_codex_app_server_session(tmp.path(), id);
    let session_dir = tmp.path().join("sessions").join(id);
    let record_path = session_dir.join("session.json");
    let mut record: Value = serde_json::from_slice(&std::fs::read(&record_path).unwrap()).unwrap();
    record["startup"] = json!({
        "schema_version": "agent-session.startup.v1",
        "state": "starting",
        "stage": "proxy",
        "started_at": "2000-01-01T00:00:00Z"
    });
    std::fs::write(&record_path, serde_json::to_vec_pretty(&record).unwrap()).unwrap();
    let failure_path = session_dir.join(".startup-failure");
    std::fs::write(&failure_path, "runtime-helper-unavailable\n").unwrap();
    let failure_epoch: i64 = std::fs::metadata(&failure_path)
        .unwrap()
        .modified()
        .unwrap()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        .try_into()
        .unwrap();
    let expected_occurred_at = jiff::Timestamp::from_second(failure_epoch)
        .unwrap()
        .to_string();
    let stopped_tmux = executable(
        &tmp.path().join("tmux-stopped"),
        "#!/usr/bin/env sh\n[ \"$1\" = has-session ] && { printf '%s\\n' \"can't find session: fixture\" >&2; exit 1; }\nexit 0\n",
    );
    let st = state(tmp.path(), Some(TOKEN), stopped_tmux);

    let (list_status, list_body) = call(router(st.clone()), get("/sessions")).await;
    assert_eq!(list_status, StatusCode::OK, "body={list_body}");
    let startup = &list_body["data"]["sessions"][0]["startup"];
    assert_eq!(startup["schema_version"], "agent-session.startup.v1");
    assert_eq!(startup["state"], "failed");
    assert_eq!(startup["stage"], "proxy");
    assert_eq!(startup["failure_code"], "runtime-helper-unavailable");
    assert_eq!(
        startup["message"],
        "Session runtime helper is unavailable after an upgrade."
    );
    assert_eq!(startup["retry_safe"], true);
    assert_eq!(startup["occurred_at"], expected_occurred_at);

    std::fs::remove_file(session_dir.join(".startup-failure")).unwrap();
    let persisted: Value = serde_json::from_slice(&std::fs::read(&record_path).unwrap()).unwrap();
    assert_eq!(persisted["startup"], *startup);

    let (glance_status, glance_body) = call(router(st), get("/sessions/stale-helper/glance")).await;
    assert_eq!(glance_status, StatusCode::OK, "body={glance_body}");
    assert_eq!(glance_body["data"]["glance"]["startup"], *startup);
}

#[tokio::test]
async fn running_session_startup_becomes_ready_and_stays_ready_after_stop() {
    let tmp = tempfile::TempDir::new().unwrap();
    let id = "startup-ready";
    seed_session_with_runtime(tmp.path(), id, "claude", "hs-claude-startup-ready");
    let record_path = tmp.path().join("sessions/startup-ready/session.json");
    let mut record: Value = serde_json::from_slice(&std::fs::read(&record_path).unwrap()).unwrap();
    record["startup"] = json!({
        "schema_version": "agent-session.startup.v1",
        "state": "starting",
        "stage": "runtime",
        "started_at": "2000-01-01T00:00:00Z"
    });
    std::fs::write(&record_path, serde_json::to_vec_pretty(&record).unwrap()).unwrap();
    let running_tmux = executable(
        &tmp.path().join("tmux-running-startup"),
        "#!/usr/bin/env sh\ncase \"$1\" in\n  list-windows) printf 'hs-claude-startup-ready\\t100\\n'; exit 0 ;;\n  has-session) exit 0 ;;\n  capture-pane) printf 'pane\\n'; exit 0 ;;\n  *) exit 0 ;;\nesac\n",
    );
    let running = state(tmp.path(), Some(TOKEN), running_tmux);

    let (list_status, list_body) = call(router(running), get("/sessions")).await;
    assert_eq!(list_status, StatusCode::OK, "body={list_body}");
    let startup = &list_body["data"]["sessions"][0]["startup"];
    assert_eq!(startup["state"], "ready");
    assert_eq!(startup["stage"], "initial_connection");
    assert!(startup.get("failure_code").is_none());

    let stopped_tmux = executable(
        &tmp.path().join("tmux-stopped-after-ready"),
        "#!/usr/bin/env sh\n[ \"$1\" = has-session ] && { printf '%s\\n' \"can't find session: fixture\" >&2; exit 1; }\nexit 0\n",
    );
    let stopped = state(tmp.path(), Some(TOKEN), stopped_tmux);
    let (glance_status, glance_body) =
        call(router(stopped), get("/sessions/startup-ready/glance")).await;
    assert_eq!(glance_status, StatusCode::OK, "body={glance_body}");
    assert_eq!(glance_body["data"]["glance"]["status"], "stopped");
    assert_eq!(glance_body["data"]["glance"]["startup"], *startup);
}

#[tokio::test]
async fn established_connection_is_ready_without_an_intermediate_observer() {
    let tmp = tempfile::TempDir::new().unwrap();
    let id = "startup-ready-before-poll";
    seed_codex_app_server_session(tmp.path(), id);
    let session_dir = tmp.path().join("sessions").join(id);
    let record_path = session_dir.join("session.json");
    let mut record: Value = serde_json::from_slice(&std::fs::read(&record_path).unwrap()).unwrap();
    record["startup"] = json!({
        "schema_version": "agent-session.startup.v1",
        "state": "starting",
        "stage": "provider_client",
        "started_at": "2000-01-01T00:00:00Z"
    });
    std::fs::write(&record_path, serde_json::to_vec_pretty(&record).unwrap()).unwrap();
    std::fs::write(session_dir.join(".startup-stage"), "initial_connection\n").unwrap();
    let stopped_tmux = executable(
        &tmp.path().join("tmux-stopped-after-connect"),
        "#!/usr/bin/env sh\n[ \"$1\" = has-session ] && { printf '%s\\n' \"can't find session: fixture\" >&2; exit 1; }\nexit 0\n",
    );
    let st = state(tmp.path(), Some(TOKEN), stopped_tmux);

    let (list_status, list_body) = call(router(st), get("/sessions")).await;
    assert_eq!(list_status, StatusCode::OK, "body={list_body}");
    let startup = &list_body["data"]["sessions"][0]["startup"];
    assert_eq!(startup["state"], "ready");
    assert_eq!(startup["stage"], "initial_connection");
    assert!(startup.get("failure_code").is_none());
}

#[tokio::test]
async fn serve_created_capable_codex_session_projects_supported_app_server_runtime() {
    let lock = GlobalStateLock::new();
    let _without_broker = EnvGuard::remove(&lock, "AGENT_SESSION_CODEX_ACCOUNT_BROKER");
    let tmp = tempfile::TempDir::new().unwrap();
    let cwd = tmp.path().join("repo");
    let runtime_dir = tempfile::Builder::new()
        .prefix("cx-")
        .tempdir_in("/tmp")
        .unwrap();
    let home = tmp.path().join("home");
    fs::set_permissions(runtime_dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let log = tmp.path().join("tmux.log");
    fs::create_dir(&cwd).unwrap();
    fs::create_dir_all(home.join(".codex")).unwrap();
    fs::write(
            home.join(".codex/hooks.json"),
            serde_json::to_vec_pretty(&json!({
                "hooks": {
                    "PermissionRequest": [{
                        "hooks": [{
                            "type": "command",
                            "command": "sh -c 'if [ \"${AGENT_SESSION_ATTENTION_AUTHORITY:-hook}\" = protocol ]; then exit 0; fi; exec agent-session activity hook --agent codex'",
                            "timeout": 5
                        }]
                    }]
                }
            }))
            .unwrap(),
        )
        .unwrap();
    let codex = executable(
        &tmp.path().join("codex-app-server"),
        "#!/usr/bin/env sh\nif [ \"$1\" = --version ]; then printf '%s\\n' 'codex-cli 0.145.0'; exit 0; fi\nif [ \"$1\" = app-server ] && [ \"$2\" = --help ]; then printf '%s\\n' '  --listen <URL>  unix://'; exit 0; fi\nexit 1\n",
    );
    let _codex_bin = EnvGuard::set(&lock, "AGENT_SESSION_CODEX_BIN", codex.to_str().unwrap());
    let _home = EnvGuard::set(&lock, "HOME", home.to_str().unwrap());
    let _runtime_dir = EnvGuard::set(
        &lock,
        "XDG_RUNTIME_DIR",
        runtime_dir.path().to_str().unwrap(),
    );
    let _runtime_mode = EnvGuard::set(&lock, "AGENT_SESSION_CODEX_RUNTIME", "app-server");
    let _capture_timeout = EnvGuard::set(&lock, "AGENT_SESSION_CODEX_CAPTURE_TIMEOUT_MS", "10");
    let st = state(tmp.path(), Some(TOKEN), logging_tmux(tmp.path(), &log));

    let (status, body) = call(
        router(st.clone()),
        post_json(
            "/sessions",
            Some(TOKEN),
            json!({
                "agent": "codex",
                "id": "managed-codex",
                "cwd": cwd.to_string_lossy(),
                "agent_args": ["-c", "model_auto_compact_token_limit=40000"]
            }),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body={body}");
    let session = &body["data"]["session"];
    assert_eq!(session["auto_resume"]["supported"], true);
    assert_eq!(
        session["startup"]["schema_version"],
        "agent-session.startup.v1"
    );
    assert_eq!(session["startup"]["state"], "starting");
    assert_eq!(session["startup"]["stage"], "runtime");
    assert!(session["startup"]["started_at"].is_string());
    let record: Value = serde_json::from_slice(
        &fs::read(tmp.path().join("sessions/managed-codex/session.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(record["startup"], session["startup"]);
    assert_eq!(
        record["agent_args"],
        json!(["-c", "model_auto_compact_token_limit=40000"])
    );
    assert_eq!(record["runtime"]["kind"], codex_app_server::RUNTIME_KIND);
    assert_eq!(
        record["runtime"]["codex_app_server_protocol"],
        codex_app_server::PROTOCOL_VERSION
    );
    let handoff = record["runtime"][codex_app_server::THREAD_HANDOFF_KEY]
        .as_str()
        .unwrap();
    assert!(!Path::new(handoff).exists());
    let calls = fs::read_to_string(log).unwrap();
    assert!(calls.contains("agent-session-codex-app-server"));
    assert!(calls.contains("app-server --listen"));
    assert!(calls.contains(&format!(
        "\"$agent\" app-server --listen \"unix://$socket\" -c {}",
        shell_words::quote("model_auto_compact_token_limit=40000")
    )));
    assert!(
        calls.contains("AGENT_SESSION_ATTENTION_AUTHORITY=hook"),
        "Codex 0.145.0 must keep hook attention authority: {calls:?}"
    );
}

// The WebSocket attach handler guards with the same `deny_unauthorized` as
// the write handlers; the `WebSocketUpgrade` extractor cannot be driven
// through `oneshot` (it needs a real upgradable connection), so the guard is
// tested directly here and the streaming path is covered by a manual smoke.
#[test]
fn deny_unauthorized_enforces_bearer_token() {
    let tmp = tempfile::TempDir::new().unwrap();

    // No token configured -> fail closed (503) even with a bearer presented.
    let st = state(tmp.path(), None, PathBuf::from("tmux"));
    assert!(deny_unauthorized(&st, &auth_headers(Some(TOKEN))).is_some());

    // Token configured: missing / wrong -> denied; correct -> allowed.
    let st = state(tmp.path(), Some(TOKEN), PathBuf::from("tmux"));
    assert!(deny_unauthorized(&st, &auth_headers(None)).is_some());
    assert!(deny_unauthorized(&st, &auth_headers(Some("nope"))).is_some());
    // Equal-length but wrong token exercises the constant-time byte loop's
    // deny path (a shorter wrong token short-circuits on the length check).
    let same_len_wrong = "X".repeat(TOKEN.len());
    assert_eq!(same_len_wrong.len(), TOKEN.len());
    assert!(deny_unauthorized(&st, &auth_headers(Some(&same_len_wrong))).is_some());
    assert!(deny_unauthorized(&st, &auth_headers(Some(TOKEN))).is_none());
}

#[test]
fn attach_status_errors_distinguish_unknown_from_non_running() {
    let record = test_record("attach-me", "hs-codex-attach-me");
    let stopped = attach_unavailable_error(&record, "stopped").into_inner();
    assert_eq!(stopped.code, "session-not-running");
    assert_eq!(stopped.exit_code, exit::DATA);
    assert_eq!(
        stopped.details.unwrap()["status"],
        Value::String("stopped".to_string())
    );

    let unknown = attach_unavailable_error(&record, "unknown").into_inner();
    assert_eq!(unknown.code, "session-status-unknown");
    assert_eq!(unknown.exit_code, exit::RUNTIME);
}

#[test]
fn sanitize_token_blanks_fail_closed() {
    // An empty/whitespace --token collapses to None so serve fails closed
    // instead of authorizing an empty bearer.
    assert_eq!(sanitize_token(Some(String::new())), None);
    assert_eq!(sanitize_token(Some("   ".to_string())), None);
    assert_eq!(sanitize_token(None), None);
    assert_eq!(
        sanitize_token(Some("tok".to_string())),
        Some("tok".to_string())
    );
}

#[tokio::test]
async fn missing_token_config_disables_writes_fail_closed() {
    let tmp = tempfile::TempDir::new().unwrap();
    let st = state(tmp.path(), None, PathBuf::from("tmux"));
    let (status, body) = call(
        router(st.clone()),
        post_json("/sessions", Some(TOKEN), json!({"agent": "codex"})),
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["error"]["code"], "token-not-configured");
}

include!("tests/prompt_projection.rs");

#[tokio::test]
async fn create_uses_and_persists_a_ready_server_launch_profile() {
    let tmp = tempfile::TempDir::new().unwrap();
    let cwd = tmp.path().join("repo");
    let config_dir = tmp.path().join(".claude-gpt");
    fs::create_dir_all(&cwd).unwrap();
    fs::create_dir_all(&config_dir).unwrap();
    let launcher = fake_agent(tmp.path(), "claude-gpt");
    let profiles = AgentLaunchProfiles::from_json(
        &json!([{
            "id":"claude-gpt",
            "label":"Claude GPT",
            "agent":"claude",
            "agent_bin":launcher,
            "provider_config_dir":config_dir,
            "readiness_args":["--check"],
            "auto_resume_supported":false,
            "graceful_shutdown":"double-ctrl-c",
        }])
        .to_string(),
    )
    .unwrap();
    let log = tmp.path().join("tmux.log");
    let tmux = logging_tmux(tmp.path(), &log);
    let mut st = state(tmp.path(), Some(TOKEN), tmux);
    Arc::get_mut(&mut st).unwrap().launch_profiles = profiles;

    let (list_status, list_body) = call(router(st.clone()), get("/sessions")).await;
    assert_eq!(list_status, StatusCode::OK, "body={list_body}");
    assert_eq!(
        list_body["data"]["agent_profiles"],
        json!([{"id":"claude-gpt","label":"Claude GPT","agent":"claude","provider_resume_import_supported":true,"history_resume_supported":true}])
    );
    assert_eq!(
        list_body["data"]["capabilities"]["profile_resume_import"],
        true
    );
    assert_eq!(
        list_body["data"]["capabilities"]["managed_resume_command"],
        false
    );
    assert_eq!(list_body["data"]["capabilities"]["last_prompt"], true);
    assert_eq!(list_body["data"]["capabilities"]["last_message"], true);
    assert_eq!(
        list_body["data"]["capabilities"]["managed_account_handoff"],
        crate::codex_app_server::MANAGED_ACCOUNT_HANDOFF_CAPABILITY
    );

    let (status, body) = call(
        router(st.clone()),
        post_json(
            "/sessions",
            Some(TOKEN),
            json!({
                "agent":"claude",
                "agent_profile":"claude-gpt",
                "id":"profiled-claude",
                "cwd":cwd,
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["data"]["session"]["agent"], "claude");
    assert_eq!(body["data"]["session"]["agent_profile"], "claude-gpt");
    assert_eq!(body["data"]["session"]["auto_resume"]["supported"], false);

    let record = load_session_record(
        &CliContext {
            state_dir: tmp.path().to_path_buf(),
            host: None,
        },
        "profiled-claude",
    )
    .unwrap();
    assert_eq!(record.agent_bin.as_deref(), launcher.to_str());
    assert_eq!(crate::session_agent_profile(&record), Some("claude-gpt"));
    assert_eq!(
        crate::session_provider_config_dir(&record),
        Some(config_dir)
    );
    assert!(!crate::session_profile_auto_resume_supported(&record));
    assert_eq!(
        record
            .runtime
            .as_ref()
            .and_then(|runtime| runtime.extra.get("agent_profile_graceful_shutdown"))
            .and_then(Value::as_str),
        Some("double-ctrl-c")
    );
    let id = body["data"]["session"]["id"].as_str().unwrap();
    let journal = crate::lifecycle::read(&st.context, id, 100).unwrap();
    let rows = journal["records"].as_array().unwrap();
    assert_eq!(
        rows.iter().filter(|r| r["operation"] == "create").count(),
        1
    );
    assert_eq!(
        rows.iter().find(|r| r["operation"] == "create").unwrap()["result"]["ok"],
        true
    );
}

#[tokio::test]
async fn create_stores_and_echoes_the_stated_lineage_and_work() {
    let tmp = tempfile::TempDir::new().unwrap();
    let cwd = tmp.path().join("repo");
    let config_dir = tmp.path().join(".claude-lineage");
    fs::create_dir_all(&cwd).unwrap();
    fs::create_dir_all(&config_dir).unwrap();
    let launcher = fake_agent(tmp.path(), "claude-lineage");
    let profiles = AgentLaunchProfiles::from_json(
        &json!([{
            "id":"claude-lineage",
            "label":"Claude",
            "agent":"claude",
            "agent_bin":launcher,
            "provider_config_dir":config_dir,
            "readiness_args":["--check"],
        }])
        .to_string(),
    )
    .unwrap();
    let log = tmp.path().join("tmux.log");
    let mut st = state(tmp.path(), Some(TOKEN), logging_tmux(tmp.path(), &log));
    Arc::get_mut(&mut st).unwrap().launch_profiles = profiles;
    let create = |id: &str, extra: Value| {
        let mut body = json!({
            "agent": "claude",
            "agent_profile": "claude-lineage",
            "id": id,
            "cwd": cwd,
        });
        for (key, value) in extra.as_object().unwrap() {
            body[key] = value.clone();
        }
        post_json("/sessions", Some(TOKEN), body)
    };
    let parent = json!({
        "machine": "sympoies",
        "session_id": "85a7379c-0bfc-4675-bab9-31ed8dcfce92",
        "session_created_at": "2026-10-01T00:00:00Z",
        "session_incarnation": "parent-launch"
    });
    let root = json!({
        "machine": "sympoies",
        "session_id": "root-session",
        "session_created_at": "2026-09-30T00:00:00Z"
    });
    let work = json!({
        "program": {"provider": "github", "repository": "serenvia/laoda", "number": 44},
        "issues": [{"provider": "github", "repository": "Sympoies/nils-cli", "number": 2032}],
        "inherited": true
    });

    let (status, body) = call(
        router(st.clone()),
        create(
            "lineage-child",
            json!({
                "lineage": {
                    "schema_version": "agent-session.session-lineage.v1",
                    "parent": parent,
                    "root": root,
                    "depth": 2,
                    "starter": {"kind": "session", "via": "console"}
                },
                "work": work,
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let expected_lineage = json!({
        "schema_version": "agent-session.session-lineage.v1",
        "machine": MACHINE,
        "parent": parent,
        "root": root,
        "depth": 2,
        "starter": {"kind": "session", "via": "console"},
        "budget": null
    });
    let expected_work = json!({
        "program": {"provider": "github", "repository": "serenvia/laoda", "number": 44},
        "issues": [{"provider": "github", "repository": "sympoies/nils-cli", "number": 2032}],
        "inherited": true,
        "revision": 1
    });
    let session = &body["data"]["session"];
    assert_eq!(session["lineage"], expected_lineage);
    assert_eq!(session["work"], expected_work);
    let record: Value = serde_json::from_slice(
        &fs::read(tmp.path().join("sessions/lineage-child/session.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(record["lineage"], expected_lineage);
    assert_eq!(record["work"], expected_work);

    // Fresh creates persist the explicit pin and the default-auto control.
    for (id, extra, mode) in [
        ("create-mode-auto", json!({}), "auto"),
        (
            "create-mode-pinned",
            json!({"title_mode":"pinned"}),
            "pinned",
        ),
    ] {
        let (status, body) = call(router(st.clone()), create(id, extra)).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["data"]["session"]["title_mode"], mode);
        assert_eq!(body["data"]["session"]["display_revision"], 0);
        let stored = load_session_record(&st.context, id).unwrap();
        assert_eq!(stored.extra["title_mode"], mode);
    }

    let (status, body) = call(
        router(st.clone()),
        create(
            "domain-parented",
            json!({
                "role":"domain-coordinator", "lineage": {
                    "parent":parent, "root":root, "depth":1,
                    "starter":{"kind":"session", "via":"console"}
                }
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["error"]["code"], "role-requires-root");
    assert_eq!(
        body["error"]["message"],
        "role domain-coordinator needs a root start: use --no-parent, or a start with no parent"
    );
    assert!(!tmp.path().join("sessions/domain-parented").exists());
    let (status, body) = call(
        router(st.clone()),
        create("domain-root", json!({"role":"domain-coordinator"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["session"]["role"], "domain-coordinator");
    assert_eq!(body["data"]["session"]["lineage"]["parent"], Value::Null);

    // Without a stated lineage the session is an operator root over HTTP.
    let (status, body) = call(router(st.clone()), create("lineage-root", json!({}))).await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let session = &body["data"]["session"];
    assert_eq!(
        session["lineage"],
        json!({
            "schema_version": "agent-session.session-lineage.v1",
            "machine": MACHINE,
            "parent": null,
            "root": {
                "machine": MACHINE,
                "session_id": "lineage-root",
                "session_created_at": session["created_at"]
            },
            "depth": 0,
            "starter": {"kind": "operator", "via": "http"},
            "budget": null
        })
    );
    assert!(session.get("work").is_none(), "{session}");
    assert!(session.get("role").is_none(), "{session}");

    // An explicit coordinator role is stored verbatim and echoed; any
    // other value is refused before a record exists.
    let (status, body) = call(
        router(st.clone()),
        create("lineage-coordinator", json!({"role": "coordinator"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["data"]["session"]["role"], "coordinator");
    let record: Value = serde_json::from_slice(
        &fs::read(tmp.path().join("sessions/lineage-coordinator/session.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(record["role"], "coordinator");

    // A coordinator is a root: a parented lineage is refused.
    let (status, body) = call(
        router(st.clone()),
        create(
            "role-parented",
            json!({
                "role": "coordinator",
                "lineage": {
                    "parent": parent,
                    "root": root,
                    "depth": 1,
                    "starter": {"kind": "session", "via": "console"}
                }
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body={body}");
    assert_eq!(body["error"]["code"], "role-requires-root");
    assert!(!tmp.path().join("sessions/role-parented").exists());
    // A root lineage with the role is fine.
    let (status, body) = call(
        router(st.clone()),
        create(
            "role-root-lineage",
            json!({
                "role": "coordinator",
                "lineage": {"depth": 0, "starter": {"kind": "operator", "via": "console"}}
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["data"]["session"]["role"], "coordinator");

    for (id, extra, code) in [
        ("bad-role", json!({"role": "invalid role"}), "role-invalid"),
        ("bad-role-type", json!({"role": 7}), "role-invalid"),
        (
            "bad-lineage",
            json!({"lineage": {"parent": parent, "root": null, "depth": 1,
                    "starter": {"kind": "session", "via": "console"}}}),
            "lineage-invalid",
        ),
        (
            "bad-work",
            json!({"work": {"issues": [{"provider": "github", "repository": "a/b", "number": 0}]}}),
            "work-ref-invalid",
        ),
    ] {
        let (status, body) = call(router(st.clone()), create(id, extra)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "body={body}");
        assert_eq!(body["error"]["code"], code);
        assert!(!tmp.path().join("sessions").join(id).exists());
    }
}

#[tokio::test]
async fn create_refuses_untrusted_forge_context_before_session_creation() {
    use pretty_assertions::assert_eq;

    let tmp = tempfile::TempDir::new().unwrap();
    let cwd = tmp.path().join("repo");
    let config = tmp.path().join("provider-config");
    fs::create_dir_all(&cwd).unwrap();
    fs::create_dir_all(&config).unwrap();
    let launcher = fake_agent(tmp.path(), "fixture-agent");
    let profiles = AgentLaunchProfiles::from_json(
        &json!([{
            "id":"fixture-profile","label":"Fixture","agent":"claude",
            "agent_bin":launcher,"provider_config_dir":config,"readiness_args":["--check"]
        }])
        .to_string(),
    )
    .unwrap();
    let mut st = state(
        tmp.path(),
        Some(TOKEN),
        logging_tmux(tmp.path(), &tmp.path().join("tmux.log")),
    );
    Arc::get_mut(&mut st).unwrap().launch_profiles = profiles;
    let reference = json!({"machine":"launch-source","session_id":"parent-session","session_created_at":"2026-10-01T00:00:00Z"});
    for (id, lineage) in [
        (
            "forged-root",
            json!({"depth":0,"starter":{"kind":"operator","via":"http"},
                "forge_context":{"initiator":"operator","role":"reviewer"}}),
        ),
        (
            "forged-child",
            json!({"depth":1,"parent":reference,"root":reference,
                "starter":{"kind":"session","via":"console"},
                "forge_context":{"initiator":"operator","role":"reviewer"}}),
        ),
    ] {
        let (status, body) = call(
            router(st.clone()),
            post_json(
                "/sessions",
                Some(TOKEN),
                json!({
                    "agent":"claude","agent_profile":"fixture-profile","id":id,"cwd":cwd,
                    "role":"reviewer","lineage":lineage
                }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "body={body}");
        assert_eq!(body["error"]["code"], "identity_session_binding_untrusted");
        assert!(!tmp.path().join("sessions").join(id).exists());
    }
}

#[tokio::test]
async fn create_persists_the_profile_codex_usage_account() {
    let tmp = tempfile::TempDir::new().unwrap();
    let cwd = tmp.path().join("repo");
    let config_dir = tmp.path().join(".claude-gpt");
    fs::create_dir_all(&cwd).unwrap();
    fs::create_dir_all(&config_dir).unwrap();
    let launcher = fake_agent(tmp.path(), "claude-gpt");
    let profiles = AgentLaunchProfiles::from_json(
        &json!([{
            "id":"claude-gpt",
            "label":"Claude GPT",
            "agent":"claude",
            "agent_bin":launcher,
            "provider_config_dir":config_dir,
            "readiness_args":["--check"],
            "auto_resume_supported":true,
            "codex_usage_account":"gpt-team",
        }])
        .to_string(),
    )
    .unwrap();
    let log = tmp.path().join("tmux.log");
    let tmux = logging_tmux(tmp.path(), &log);
    let mut st = state(tmp.path(), Some(TOKEN), tmux);
    Arc::get_mut(&mut st).unwrap().launch_profiles = profiles;

    let (status, body) = call(
        router(st),
        post_json(
            "/sessions",
            Some(TOKEN),
            json!({
                "agent":"claude",
                "agent_profile":"claude-gpt",
                "id":"profiled-claude-gpt",
                "cwd":cwd,
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");

    let record = load_session_record(
        &CliContext {
            state_dir: tmp.path().to_path_buf(),
            host: None,
        },
        "profiled-claude-gpt",
    )
    .unwrap();
    assert_eq!(
        crate::session_codex_usage_account(&record),
        Some("gpt-team".to_string())
    );
}

#[tokio::test]
async fn partition_routes_codex_account_backed_claude_to_its_own_usage_source() {
    let tmp = tempfile::TempDir::new().unwrap();
    let state_dir = tmp.path().join("state");
    // A `claude` session whose launch profile declares a Codex usage account.
    seed_session_with_runtime(&state_dir, "gpt-claude", "claude", "hs-gpt-claude");
    let record_path = state_dir.join("sessions/gpt-claude/session.json");
    let mut record: Value =
        serde_json::from_str(&std::fs::read_to_string(&record_path).unwrap()).unwrap();
    // `RuntimeInfo::extra` is `#[serde(flatten)]`, so the profile keys live
    // at the runtime object's top level.
    record["runtime"]["agent_profile"] = json!("claude-gpt");
    record["runtime"]["agent_profile_codex_usage_account"] = json!("gpt-team");
    std::fs::write(&record_path, serde_json::to_vec_pretty(&record).unwrap()).unwrap();
    // A plain native-Claude session with no Codex account.
    seed_session_with_runtime(&state_dir, "native-claude", "claude", "hs-native-claude");

    let st = state(&state_dir, Some(TOKEN), minimal_tmux(tmp.path()));
    let partition = partition_auto_resume_ids(
        &st,
        vec!["gpt-claude".to_string(), "native-claude".to_string()],
    )
    .await;

    assert!(partition.codex.is_empty());
    assert_eq!(partition.claude, vec!["native-claude".to_string()]);
    assert_eq!(partition.codex_account_claude.len(), 1);
    assert_eq!(partition.codex_account_claude[0].id, "gpt-claude");
    assert_eq!(partition.codex_account_claude[0].account, "gpt-team");
}

#[tokio::test]
async fn fresh_profile_session_resume_preserves_context_or_fails_closed_without_proof() {
    fresh_profile_session_resume_fixture(false).await;
}

#[tokio::test]
async fn fresh_profile_session_resume_fails_closed_with_malformed_broker_runtime_identity() {
    fresh_profile_session_resume_fixture(true).await;
}

async fn fresh_profile_session_resume_fixture(malformed_broker_identity: bool) {
    let lock = GlobalStateLock::new();
    let _without_broker = EnvGuard::remove(&lock, "AGENT_SESSION_CODEX_ACCOUNT_BROKER");
    let tmp = tempfile::TempDir::new().unwrap();
    let cwd = tmp.path().join("worktrees/issue-362");
    let config_dir = tmp.path().join("codex-profile");
    let transcript = config_dir.join("sessions/2026/07/18/fresh-profile.jsonl");
    let running = tmp.path().join("profile-runtime-running");
    let pane_pid = tmp.path().join("profile-pane.pid");
    let log = tmp.path().join("tmux.log");
    fs::create_dir_all(&cwd).unwrap();
    fs::create_dir_all(transcript.parent().unwrap()).unwrap();
    let launcher = fake_agent(tmp.path(), "codex-profile-bin");
    let transcript_line = json!({
        "timestamp":"2099-01-01T00:00:00Z",
        "type":"session_meta",
        "payload":{
            "id":"fresh-provider-id",
            "session_id":"fresh-provider-id",
            "cwd":cwd,
            "source":"cli",
            "timestamp":"2099-01-01T00:00:00Z"
        }
    })
    .to_string();
    let tmux = executable(
        &tmp.path().join("tmux"),
        &format!(
            r#"#!/usr/bin/env sh
printf '%s\n' "$*" >> {log}
case "$1" in
  has-session)
    if test -f {running}; then
      exit 0
    fi
    printf '%s\n' "can't find session: fresh-profile" >&2
    exit 1
    ;;
  new-session)
    printf '%s\n' {transcript_line} > {transcript}
    : > {running}
    heartbeat=''
    slot=0
    for arg in "$@"; do
      if [ "$slot" = 2 ]; then incarnation="$arg"; break
      elif [ "$slot" = 1 ]; then slot=2
      else case "$arg" in */coordination/heartbeat) heartbeat="$arg"; slot=1 ;; esac
      fi
    done
    if [ -n "$heartbeat" ] && [ -n "$incarnation" ]; then
      mkdir -p "$(dirname "$heartbeat")"
      printf '%s:%s\n' "$incarnation" "$(date +%s)" > "$heartbeat"
      chmod 600 "$heartbeat"
    fi
    printf '%s\t%s\t%s\n' '$77' '%77' "$(cat {pane_pid})"
    ;;
  display-message)
    printf '%s\t%s\t%s\n' '$77' '%77' "$(cat {pane_pid})"
    ;;
  if-shell)
    pid="$(cat {pane_pid})"
    kill -KILL "$pid" 2>/dev/null || true
    rm -f {running}
    ;;
  kill-session)
    rm -f {running}
    ;;
  *) exit 0 ;;
esac
"#,
            log = shell_words::quote(&log.to_string_lossy()),
            running = shell_words::quote(&running.to_string_lossy()),
            pane_pid = shell_words::quote(&pane_pid.to_string_lossy()),
            transcript = shell_words::quote(&transcript.to_string_lossy()),
            transcript_line = shell_words::quote(&transcript_line),
        ),
    );
    let profiles = AgentLaunchProfiles::from_json(
        &json!([{
            "id":"codex-profile",
            "label":"Codex Profile",
            "agent":"codex",
            "agent_bin":launcher,
            "provider_config_dir":config_dir,
            "auto_resume_supported":false,
        }])
        .to_string(),
    )
    .unwrap();
    let mut st = state(tmp.path(), Some(TOKEN), tmux.clone());
    Arc::get_mut(&mut st).unwrap().launch_profiles = profiles;
    let first_pane = TestProcessGroup::spawn();
    fs::write(&pane_pid, first_pane.pid().to_string()).unwrap();

    let (create_status, create_body) = call(
        router(st.clone()),
        post_json(
            "/sessions",
            Some(TOKEN),
            json!({
                "agent":"codex",
                "agent_profile":"codex-profile",
                "id":"fresh-profile",
                "cwd":cwd,
            }),
        ),
    )
    .await;
    assert_eq!(create_status, StatusCode::OK, "body={create_body}");
    assert_eq!(
        create_body["data"]["session"]["provider_resume"]["session_id"],
        "fresh-provider-id"
    );

    drop(first_pane);
    let stopped_record = load_session_record(&st.context, "fresh-profile").unwrap();
    crate::coordination::revoke(&st.context, &stopped_record).unwrap();
    {
        let mut locked = crate::coordination::lock_registry(&st.context).unwrap();
        let mut registry = serde_json::to_value(&locked.registry).unwrap();
        let broker = &mut registry["brokers"]["fresh-profile"];
        assert_eq!(broker["state"], "stopped");
        assert_eq!(broker["capability_digest"], "");
        assert_eq!(
            broker["runtime_identity"],
            serde_json::to_value(
                crate::persisted_tmux_runtime_identity(&stopped_record)
                    .unwrap()
                    .unwrap()
            )
            .unwrap()
        );
        if malformed_broker_identity {
            // Present but malformed evidence must not use the
            // compatibility path for a missing runtime identity.
            broker["runtime_identity"] = json!({});
            locked.registry = serde_json::from_value(registry).unwrap();
            locked.save().unwrap();
        }
    }
    let old_incarnation = crate::coordination::incarnation(&stopped_record).unwrap();
    let heartbeat = crate::coordination::heartbeat_path(&st.context.state_dir, "fresh-profile");
    fs::write(&heartbeat, format!("{old_incarnation}:0\n")).unwrap();
    assert!(heartbeat.exists(), "retain the expired heartbeat sidecar");
    assert!(!crate::coordination::broker::heartbeat_fresh(
        &st.context,
        "fresh-profile",
        &old_incarnation,
        0,
    ));
    fs::remove_file(&running).unwrap();
    let (list_status, list_body) = call(router(st.clone()), get("/sessions")).await;
    assert_eq!(list_status, StatusCode::OK, "body={list_body}");
    let stopped = &list_body["data"]["sessions"][0];
    assert_eq!(stopped["status"], "stopped");
    assert_eq!(stopped["resumable"], true);
    assert_eq!(stopped["cwd"], cwd.to_string_lossy().as_ref());
    assert_eq!(stopped["agent_profile"], "codex-profile");

    let resumed_pane = TestProcessGroup::spawn();
    fs::write(&pane_pid, resumed_pane.pid().to_string()).unwrap();
    // Real tmux reaps its pane child after kill-session. Keep a concurrent
    // waiter so macOS does not report the terminated child as a live zombie
    // process group while the resume path verifies the boundary is empty.
    let mut resumed_pane = resumed_pane.into_background_reaper();
    let (resume_status, resume_body) = call(
        router(st),
        post_json("/sessions/fresh-profile/resume", Some(TOKEN), json!({})),
    )
    .await;
    let calls = fs::read_to_string(log).unwrap();
    assert_eq!(
        calls
            .lines()
            .filter(|line| line.contains("new-session"))
            .count(),
        2,
        "fresh create and managed resume must each launch once: {calls:?}"
    );
    assert!(
        calls.contains(&format!("CODEX_HOME={}", config_dir.display())),
        "both launches must pin the effective profile root: {calls:?}"
    );
    assert!(
        calls.contains(launcher.to_string_lossy().as_ref()),
        "both launches must use the profile wrapper: {calls:?}"
    );
    assert!(
        calls.contains(cwd.to_string_lossy().as_ref()),
        "managed resume must retain the original worktree cwd: {calls:?}"
    );
    assert!(
        calls.contains("resume fresh-provider-id"),
        "managed resume must use the captured provider id: {calls:?}"
    );
    let supports_stopped_runtime_proof = cfg!(any(target_os = "linux", target_os = "macos"));
    if malformed_broker_identity || !supports_stopped_runtime_proof {
        assert_eq!(
            resume_status,
            StatusCode::INTERNAL_SERVER_ERROR,
            "body={resume_body}"
        );
        assert_eq!(
            resume_body["error"]["code"],
            "coordination-runtime-unverified"
        );
        assert!(
            resumed_pane.wait_for_exit(Duration::from_secs(1)),
            "failed-launch cleanup must reap the replacement pane"
        );
        assert!(
            !running.exists(),
            "failed-launch cleanup must clear tmux liveness"
        );
        let retained: Value = serde_json::from_str(
            &fs::read_to_string(tmp.path().join("sessions/fresh-profile/session.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(retained["cwd"], cwd.to_string_lossy().as_ref());
        assert_eq!(retained["runtime"]["agent_profile"], "codex-profile");
        assert_eq!(
            retained["provider_resume"]["session_id"],
            "fresh-provider-id"
        );
        let old_runtime = stopped_record.runtime.as_ref().unwrap();
        assert_eq!(retained["runtime"]["generation"], old_runtime.generation);
        assert_eq!(retained["runtime"]["launch_id"], old_runtime.launch_id);
    } else {
        assert_eq!(resume_status, StatusCode::OK, "body={resume_body}");
        assert_eq!(
            resume_body["data"]["session"]["cwd"],
            cwd.to_string_lossy().as_ref()
        );
        assert_eq!(
            resume_body["data"]["session"]["agent_profile"],
            "codex-profile"
        );
        let resumed = &resume_body["data"]["session"];
        assert_eq!(
            resumed["provider_resume"]["session_id"],
            "fresh-provider-id"
        );
        let retained: Value = serde_json::from_str(
            &fs::read_to_string(tmp.path().join("sessions/fresh-profile/session.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(retained["cwd"], cwd.to_string_lossy().as_ref());
        assert_eq!(retained["runtime"]["agent_profile"], "codex-profile");
        assert_eq!(
            retained["provider_resume"]["session_id"],
            "fresh-provider-id"
        );
        let old_runtime = stopped_record.runtime.as_ref().unwrap();
        assert_eq!(
            retained["runtime"]["generation"],
            old_runtime.generation + 1
        );
        assert_ne!(retained["runtime"]["launch_id"], old_runtime.launch_id);
        let terminate = ProcessCommand::new(&tmux)
            .args([
                "if-shell",
                "-F",
                "-t",
                "%77",
                "true",
                "kill-session -t $77",
                "false",
            ])
            .status()
            .unwrap();
        assert!(terminate.success());
        assert!(
            resumed_pane.wait_for_exit(Duration::from_secs(1)),
            "identity-bound fixture termination must stop the pane process group"
        );
        assert!(
            !running.exists(),
            "identity-bound fixture termination must clear tmux liveness"
        );
    }
}

#[tokio::test]
async fn create_rejects_unknown_mismatched_or_unready_launch_profiles() {
    let tmp = tempfile::TempDir::new().unwrap();
    let launcher = fake_agent(tmp.path(), "profile-agent");
    let profiles = AgentLaunchProfiles::from_json(
        &json!([{
            "id":"codex-profile",
            "label":"Codex Profile",
            "agent":"codex",
            "agent_bin":launcher,
            "readiness_args":["--check"],
        }])
        .to_string(),
    )
    .unwrap();
    let mut st = state(tmp.path(), Some(TOKEN), minimal_tmux(tmp.path()));
    Arc::get_mut(&mut st).unwrap().launch_profiles = profiles;

    let (status, body) = call(
        router(st.clone()),
        post_json(
            "/sessions",
            Some(TOKEN),
            json!({"agent":"claude","agent_profile":"missing"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "unknown-agent-profile");

    let (status, body) = call(
        router(st.clone()),
        post_json(
            "/sessions",
            Some(TOKEN),
            json!({"agent":"claude","agent_profile":"codex-profile"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "agent-profile-agent-conflict");

    fs::write(&launcher, "#!/usr/bin/env sh\nexit 1\n").unwrap();
    let (status, body) = call(
        router(st),
        post_json(
            "/sessions",
            Some(TOKEN),
            json!({"agent":"codex","agent_profile":"codex-profile"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(body["error"]["code"], "agent-profile-unavailable");
}

#[tokio::test]
async fn create_imports_codex_session_from_provider_resume_id() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().unwrap();
    let runtime_dir = tempfile::Builder::new()
        .prefix("cx-")
        .tempdir_in("/tmp")
        .unwrap();
    let cwd = tmp.path().join("repo");
    let codex_home = tmp.path().join("codex-home");
    fs::set_permissions(runtime_dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
    std::fs::create_dir_all(&cwd).unwrap();
    write_codex_session_meta(&codex_home, "external-codex-id", &cwd);
    let log = tmp.path().join("tmux.log");
    let tmux = logging_tmux(tmp.path(), &log);
    let codex = executable(
        &tmp.path().join("codex"),
        "#!/usr/bin/env sh\nif [ \"$1\" = --version ]; then printf '%s\\n' 'codex-cli 0.145.0'; exit 0; fi\nif [ \"$1\" = app-server ] && [ \"$2\" = --help ]; then printf '%s\\n' '  --listen <URL>  unix://'; exit 0; fi\nexit 1\n",
    );
    let _codex_home = EnvGuard::set(&lock, "CODEX_HOME", codex_home.to_str().unwrap());
    let _codex_bin = EnvGuard::set(&lock, "AGENT_SESSION_CODEX_BIN", codex.to_str().unwrap());
    let _runtime_preference = EnvGuard::set(&lock, "AGENT_SESSION_CODEX_RUNTIME", "auto");
    let _runtime_dir = EnvGuard::set(
        &lock,
        "XDG_RUNTIME_DIR",
        runtime_dir.path().to_str().unwrap(),
    );
    let _broker = EnvGuard::set(
        &lock,
        "AGENT_SESSION_CODEX_ACCOUNT_BROKER",
        r#"["/bin/false"]"#,
    );
    let st = state(tmp.path(), Some(TOKEN), tmux);

    let (status, body) = call(
        router(st.clone()),
        post_json(
            "/sessions",
            Some(TOKEN),
            json!({
                "agent": "codex",
                "id": "imported-codex",
                "provider_resume_id": "external-codex-id",
                "title_mode": "pinned",
                "title_state": {
                    "topic": "Imported Codex",
                    "topic_source": "user",
                    "references": ["#317"],
                    "activity": null
                }
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let session = &body["data"]["session"];
    assert_eq!(session["id"], "imported-codex");
    assert_eq!(session["title_mode"], "pinned");
    assert_eq!(session["display_revision"], 0);
    assert_eq!(session["agent"], "codex");
    assert_eq!(session["title"], "Imported Codex #317");
    assert_eq!(session["title_state"]["topic"], "Imported Codex");
    assert_eq!(session["title_state"]["topic_source"], "user");
    assert_eq!(session["title_state"]["references"], json!(["#317"]));
    assert_eq!(session["cwd"], cwd.to_string_lossy().as_ref());
    assert_eq!(session["status"], "running");
    assert_eq!(session["resumable"], true);
    assert_eq!(session["auto_resume"]["supported"], true);
    assert_eq!(session["codex_account"]["supported"], true);
    assert_eq!(
        session["provider_resume"]["session_id"],
        "external-codex-id"
    );
    assert_eq!(
        session["provider_resume"]["resume_args"],
        json!([
            "resume",
            "external-codex-id",
            "--cd",
            cwd.to_string_lossy(),
            "--no-alt-screen"
        ])
    );

    let calls = std::fs::read_to_string(&log).unwrap();
    assert!(
        calls.contains("-s hs-codex-imported-codex"),
        "import must create a tmux runtime: {calls:?}"
    );
    assert!(
        calls.contains("resume external-codex-id"),
        "import must use the external provider id: {calls:?}"
    );
    assert!(
        calls.contains("--cd") && calls.contains(cwd.to_string_lossy().as_ref()),
        "import must resume in the provider cwd: {calls:?}"
    );
    assert!(
        tmp.path()
            .join("sessions/imported-codex/resume.json")
            .is_file(),
        "import must persist durable resume metadata"
    );
    let record: Value = serde_json::from_slice(
        &fs::read(tmp.path().join("sessions/imported-codex/session.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(record["title_mode"], "pinned");
    assert_eq!(record["runtime"]["kind"], codex_app_server::RUNTIME_KIND);
    assert_eq!(
        record["runtime"][codex_app_server::PROTOCOL_KEY],
        codex_app_server::PROTOCOL_VERSION
    );
}

#[tokio::test]
async fn create_imports_unsupported_codex_session_with_standalone_resume() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().unwrap();
    let cwd = tmp.path().join("repo");
    let codex_home = tmp.path().join("codex-home");
    std::fs::create_dir_all(&cwd).unwrap();
    write_codex_session_meta(&codex_home, "standalone-codex-id", &cwd);
    let log = tmp.path().join("tmux.log");
    let tmux = logging_tmux(tmp.path(), &log);
    let codex = fake_agent(tmp.path(), "codex");
    let _codex_home = EnvGuard::set(&lock, "CODEX_HOME", codex_home.to_str().unwrap());
    let _codex_bin = EnvGuard::set(&lock, "AGENT_SESSION_CODEX_BIN", codex.to_str().unwrap());
    let _runtime_preference = EnvGuard::set(&lock, "AGENT_SESSION_CODEX_RUNTIME", "auto");
    let st = state(tmp.path(), Some(TOKEN), tmux);

    let (status, body) = call(
        router(st),
        post_json(
            "/sessions",
            Some(TOKEN),
            json!({
                "agent": "codex",
                "id": "standalone-codex",
                "provider_resume_id": "standalone-codex-id"
            }),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body={body}");
    let session = &body["data"]["session"];
    assert_eq!(session["title_mode"], "auto");
    assert_eq!(session["display_revision"], 0);
    assert_eq!(session["auto_resume"]["supported"], false);
    assert_eq!(session["codex_account"]["supported"], false);
    let record: Value = serde_json::from_slice(
        &fs::read(tmp.path().join("sessions/standalone-codex/session.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(record["title_mode"], "auto");
    assert_eq!(record["runtime"]["kind"], "tmux");
    let calls = fs::read_to_string(log).unwrap();
    assert!(
        calls.contains("resume standalone-codex-id"),
        "unsupported import must retain the standalone provider resume path: {calls:?}"
    );
    assert!(!calls.contains("agent-session-codex-app-server"));
}

#[tokio::test]
async fn create_imports_claude_session_from_resume_id_alias() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().unwrap();
    let cwd = tmp.path().join("repo");
    let claude_config = tmp.path().join("claude-config");
    std::fs::create_dir_all(&cwd).unwrap();
    write_claude_session_transcript(
        &claude_config,
        "-fixture-project",
        "external-claude-id",
        &cwd,
    );
    let log = tmp.path().join("tmux.log");
    let tmux = logging_tmux(tmp.path(), &log);
    let claude = fake_agent(tmp.path(), "claude");
    let _claude_config = EnvGuard::set(&lock, "CLAUDE_CONFIG_DIR", claude_config.to_str().unwrap());
    let _claude_bin = EnvGuard::set(&lock, "AGENT_SESSION_CLAUDE_BIN", claude.to_str().unwrap());
    let st = state(tmp.path(), Some(TOKEN), tmux);

    let (status, body) = call(
        router(st.clone()),
        post_json(
            "/sessions",
            Some(TOKEN),
            json!({
                "agent": "claude",
                "id": "imported-claude",
                "resume_id": "external-claude-id",
                "title": "Imported Claude"
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let session = &body["data"]["session"];
    assert_eq!(session["id"], "imported-claude");
    assert_eq!(session["agent"], "claude");
    assert_eq!(session["cwd"], cwd.to_string_lossy().as_ref());
    assert_eq!(session["status"], "running");
    assert_eq!(session["resumable"], true);
    assert_eq!(
        session["provider_resume"]["session_id"],
        "external-claude-id"
    );
    assert_eq!(
        session["provider_resume"]["resume_args"],
        json!(["--resume", "external-claude-id"])
    );

    let calls = std::fs::read_to_string(&log).unwrap();
    assert!(
        calls.contains("-s hs-claude-imported-claude"),
        "import must create a tmux runtime: {calls:?}"
    );
    assert!(
        calls.contains("--resume external-claude-id"),
        "import must use the external provider id: {calls:?}"
    );
    assert!(
        !calls.contains("--session-id"),
        "imported Claude resume must not create a new provider session: {calls:?}"
    );
    assert!(
        tmp.path()
            .join("sessions/imported-claude/resume.json")
            .is_file(),
        "import must persist durable resume metadata"
    );
}

#[tokio::test]
async fn create_imports_claude_session_only_from_selected_profile_root() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().unwrap();
    let expected_cwd = tmp.path().join("profile-repo");
    let wrong_cwd = tmp.path().join("base-repo");
    let base_config = tmp.path().join("base-claude");
    let profile_config = tmp.path().join("profile-claude");
    fs::create_dir_all(&expected_cwd).unwrap();
    fs::create_dir_all(&wrong_cwd).unwrap();
    write_claude_session_transcript(
        &base_config,
        "-base-project",
        "shared-claude-id",
        &wrong_cwd,
    );
    write_claude_session_transcript(
        &profile_config,
        "-profile-project",
        "shared-claude-id",
        &expected_cwd,
    );
    let launcher = fake_agent(tmp.path(), "profile-claude-bin");
    let profiles = AgentLaunchProfiles::from_json(
        &json!([{
            "id":"profile-claude",
            "label":"Profile Claude",
            "agent":"claude",
            "agent_bin":launcher,
            "provider_config_dir":profile_config,
            "auto_resume_supported":false,
        }])
        .to_string(),
    )
    .unwrap();
    let log = tmp.path().join("tmux.log");
    let tmux = logging_tmux(tmp.path(), &log);
    let _base_config = EnvGuard::set(&lock, "CLAUDE_CONFIG_DIR", base_config.to_str().unwrap());
    let mut st = state(tmp.path(), Some(TOKEN), tmux);
    Arc::get_mut(&mut st).unwrap().launch_profiles = profiles;

    let (status, body) = call(
        router(st),
        post_json(
            "/sessions",
            Some(TOKEN),
            json!({
                "agent":"claude",
                "agent_profile":"profile-claude",
                "id":"profile-import",
                "provider_resume_id":"shared-claude-id"
            }),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body={body}");
    let session = &body["data"]["session"];
    assert_eq!(session["cwd"], expected_cwd.to_string_lossy().as_ref());
    assert_eq!(session["agent_profile"], "profile-claude");
    let calls = fs::read_to_string(log).unwrap();
    assert!(
        calls.contains(&format!("CLAUDE_CONFIG_DIR={}", profile_config.display())),
        "profile import must pin the selected provider root: {calls:?}"
    );
    assert!(
        !calls.contains(wrong_cwd.to_string_lossy().as_ref()),
        "profile import must never recover cwd from the base root: {calls:?}"
    );
}

#[tokio::test]
async fn create_profile_import_does_not_fall_back_to_base_provider_root() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().unwrap();
    let base_cwd = tmp.path().join("base-repo");
    let base_config = tmp.path().join("base-claude");
    let profile_config = tmp.path().join("profile-claude");
    fs::create_dir_all(&base_cwd).unwrap();
    fs::create_dir_all(&profile_config).unwrap();
    write_claude_session_transcript(&base_config, "-base-project", "base-only-id", &base_cwd);
    let launcher = fake_agent(tmp.path(), "profile-claude-bin");
    let profiles = AgentLaunchProfiles::from_json(
        &json!([{
            "id":"profile-claude",
            "label":"Profile Claude",
            "agent":"claude",
            "agent_bin":launcher,
            "provider_config_dir":profile_config,
            "auto_resume_supported":false,
        }])
        .to_string(),
    )
    .unwrap();
    let log = tmp.path().join("tmux.log");
    let tmux = logging_tmux(tmp.path(), &log);
    let _base_config = EnvGuard::set(&lock, "CLAUDE_CONFIG_DIR", base_config.to_str().unwrap());
    let mut st = state(tmp.path(), Some(TOKEN), tmux);
    Arc::get_mut(&mut st).unwrap().launch_profiles = profiles;

    let (status, body) = call(
        router(st),
        post_json(
            "/sessions",
            Some(TOKEN),
            json!({
                "agent":"claude",
                "agent_profile":"profile-claude",
                "provider_resume_id":"base-only-id"
            }),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "body={body}");
    assert_eq!(body["error"]["code"], "provider-resume-not-found");
    assert_eq!(body["error"]["details"]["agent_profile"], "profile-claude");
    assert!(
        !log.exists() || !fs::read_to_string(log).unwrap().contains("new-session"),
        "a base-root-only provider session must not launch"
    );
}

#[tokio::test]
async fn create_provider_resume_id_rejects_cwd_and_prompt_overrides() {
    let tmp = tempfile::TempDir::new().unwrap();
    let st = state(tmp.path(), Some(TOKEN), minimal_tmux(tmp.path()));

    let (status, body) = call(
        router(st.clone()),
        post_json(
            "/sessions",
            Some(TOKEN),
            json!({
                "agent": "codex",
                "provider_resume_id": "external-id",
                "cwd": "/should-not-override"
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "provider-resume-cwd-conflict");

    let (status, body) = call(
        router(st.clone()),
        post_json(
            "/sessions",
            Some(TOKEN),
            json!({
                "agent": "codex",
                "provider_resume_id": "external-id",
                "prompt": "continue"
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "provider-resume-prompt-conflict");

    let (status, body) = call(
        router(st.clone()),
        post_json(
            "/sessions",
            Some(TOKEN),
            json!({
                "agent": "codex",
                "provider_resume_id": "external-id",
                "agent_args": ["--cd", "/should-not-override"]
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "provider-resume-agent-args-conflict");
}

#[tokio::test]
async fn create_provider_resume_id_reports_not_found() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().unwrap();
    let codex_home = tmp.path().join("codex-home");
    std::fs::create_dir_all(codex_home.join("sessions")).unwrap();
    let _codex_home = EnvGuard::set(&lock, "CODEX_HOME", codex_home.to_str().unwrap());
    let st = state(tmp.path(), Some(TOKEN), minimal_tmux(tmp.path()));

    let (status, body) = call(
        router(st.clone()),
        post_json(
            "/sessions",
            Some(TOKEN),
            json!({
                "agent": "codex",
                "id": "import-failure",
                "provider_resume_id": "missing-codex-id"
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["error"]["code"], "provider-resume-not-found");
    assert_eq!(
        body["error"]["details"]["provider_resume_id"],
        "missing-codex-id"
    );
    let journal = crate::lifecycle::read(&st.context, "import-failure", 100).unwrap();
    let rows = journal["records"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["operation"], "import");
    assert_eq!(rows[0]["caller"]["kind"], "serve");
    assert_eq!(rows[0]["result"]["code"], body["error"]["code"]);
    assert!(!journal.to_string().contains("missing-codex-id"));
}

#[tokio::test]
async fn create_provider_resume_id_rejects_invalid_values() {
    let tmp = tempfile::TempDir::new().unwrap();
    let st = state(tmp.path(), Some(TOKEN), minimal_tmux(tmp.path()));

    for provider_resume_id in ["", "bad\nid"] {
        let (status, body) = call(
            router(st.clone()),
            post_json(
                "/sessions",
                Some(TOKEN),
                json!({
                    "agent": "codex",
                    "provider_resume_id": provider_resume_id
                }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"]["code"], "invalid-provider-resume-id");
    }
}

#[tokio::test]
async fn create_provider_resume_id_reports_ambiguous_cwd_matches() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().unwrap();
    let codex_home = tmp.path().join("codex-home");
    let cwd_a = tmp.path().join("repo-a");
    let cwd_b = tmp.path().join("repo-b");
    std::fs::create_dir_all(&cwd_a).unwrap();
    std::fs::create_dir_all(&cwd_b).unwrap();
    write_codex_session_meta_file(&codex_home, "first.jsonl", "shared-codex-id", &cwd_a);
    write_codex_session_meta_file(&codex_home, "second.jsonl", "shared-codex-id", &cwd_b);
    let _codex_home = EnvGuard::set(&lock, "CODEX_HOME", codex_home.to_str().unwrap());
    let st = state(tmp.path(), Some(TOKEN), minimal_tmux(tmp.path()));

    let (status, body) = call(
        router(st.clone()),
        post_json(
            "/sessions",
            Some(TOKEN),
            json!({
                "agent": "codex",
                "provider_resume_id": "shared-codex-id"
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["error"]["code"], "provider-resume-ambiguous");
    assert_eq!(body["error"]["details"]["cwd_count"], 2);
}

#[tokio::test]
async fn create_provider_resume_id_reports_scan_truncation() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().unwrap();
    let codex_home = tmp.path().join("codex-home");
    let cwd = tmp.path().join("repo");
    std::fs::create_dir_all(&cwd).unwrap();
    write_codex_session_meta(&codex_home, "external-codex-id", &cwd);
    let _codex_home = EnvGuard::set(&lock, "CODEX_HOME", codex_home.to_str().unwrap());
    let _max_entries = EnvGuard::set(&lock, "AGENT_SESSION_CODEX_RESUME_SCAN_MAX_ENTRIES", "1");
    let st = state(tmp.path(), Some(TOKEN), minimal_tmux(tmp.path()));

    let (status, body) = call(
        router(st.clone()),
        post_json(
            "/sessions",
            Some(TOKEN),
            json!({
                "agent": "codex",
                "provider_resume_id": "external-codex-id"
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(body["error"]["code"], "provider-resume-scan-truncated");
}

#[tokio::test]
async fn create_imports_claude_session_after_oversized_transcript_line() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().unwrap();
    let cwd = tmp.path().join("repo");
    let claude_config = tmp.path().join("claude-config");
    let bad_transcript = claude_config
        .join("projects")
        .join("-fixture-project")
        .join("oversized.jsonl");
    std::fs::create_dir_all(&cwd).unwrap();
    std::fs::create_dir_all(bad_transcript.parent().expect("parent")).unwrap();
    std::fs::write(
        &bad_transcript,
        "x".repeat(nils_provider_resume::CLAUDE_SESSION_META_MAX_LINE_BYTES + 1),
    )
    .unwrap();
    write_claude_session_transcript(
        &claude_config,
        "-fixture-project",
        "external-claude-id",
        &cwd,
    );
    let log = tmp.path().join("tmux.log");
    let tmux = logging_tmux(tmp.path(), &log);
    let claude = fake_agent(tmp.path(), "claude");
    let _claude_config = EnvGuard::set(&lock, "CLAUDE_CONFIG_DIR", claude_config.to_str().unwrap());
    let _claude_bin = EnvGuard::set(&lock, "AGENT_SESSION_CLAUDE_BIN", claude.to_str().unwrap());
    let st = state(tmp.path(), Some(TOKEN), tmux);

    let (status, body) = call(
        router(st.clone()),
        post_json(
            "/sessions",
            Some(TOKEN),
            json!({
                "agent": "claude",
                "id": "imported-claude-oversized",
                "provider_resume_id": "external-claude-id"
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(
        body["data"]["session"]["cwd"],
        cwd.to_string_lossy().as_ref()
    );
}

#[tokio::test]
async fn create_provider_resume_id_rejects_retired_dsh_agent() {
    let tmp = tempfile::TempDir::new().unwrap();
    let st = state(tmp.path(), Some(TOKEN), minimal_tmux(tmp.path()));

    let (status, body) = call(
        router(st.clone()),
        post_json(
            "/sessions",
            Some(TOKEN),
            json!({ "agent": "hermes", "provider_resume_id": "external-id" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "invalid-agent");

    for agent in ["dsh"] {
        let (status, body) = call(
            router(st.clone()),
            post_json(
                "/sessions",
                Some(TOKEN),
                json!({
                    "agent": agent,
                    "provider_resume_id": "external-id"
                }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"]["code"], "unsupported-agent");
    }
}

#[tokio::test]
async fn send_with_token_delivers_without_leaking_text() {
    let tmp = tempfile::TempDir::new().unwrap();
    let tmux = minimal_tmux(tmp.path());
    seed_session(tmp.path(), "steer", "codex", "hs-codex-steer");
    let st = state(tmp.path(), Some(TOKEN), tmux);

    let secret = "serve-secret-approval";
    let (status, body) = call(
        router(st.clone()),
        post_json(
            "/sessions/steer/send",
            Some(TOKEN),
            json!({ "text": secret, "keys": ["enter"] }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["schema_version"], "cli.agent-session.serve.v1");
    assert_eq!(body["data"]["machine"], MACHINE);
    assert_eq!(body["data"]["sent"]["sent_text"], true);
    assert_eq!(body["data"]["sent"]["keys"][0], "enter");
    // The stub pane draws no recognizable composer, so the submission is
    // reported but cannot be proven either way.
    assert_eq!(
        body["data"]["sent"]["submission"],
        json!({ "outcome": "unverified", "enter_presses": 1 })
    );
    // The literal text is never echoed back into the response contract.
    assert!(
        !serde_json::to_string(&body).unwrap().contains(secret),
        "secret text leaked into serve response: {body}"
    );
}

#[test]
fn a_prompt_stuck_in_the_composer_is_a_conflict() {
    let response = envelope_err(CliError::runtime(
        "send-submit-stuck",
        "the text is still in the session's input box",
        Some(json!({ "id": "steer", "outcome": "stuck", "enter_presses": 2 })),
    ));
    assert_eq!(response.status(), StatusCode::CONFLICT);
}

#[path = "tests/send_key_contract.rs"]
mod send_key_contract;

/// A `send` body's key list is remote input whose length is bounded only by
/// the Axum body limit, so the handler must reject an oversized list before
/// it sizes any allocation from it. Every accepted name maps to one of nine
/// `SpecialKey` variants, so a request carrying more than
/// `MAX_SEND_KEYS` names is never a real caller.
/// `sympoies/nils-cli#1542`.
#[tokio::test]
async fn send_rejects_a_key_list_past_the_allocation_bound() {
    let tmp = tempfile::TempDir::new().unwrap();
    let tmux = minimal_tmux(tmp.path());
    seed_session(tmp.path(), "steer", "codex", "hs-codex-steer");
    let st = state(tmp.path(), Some(TOKEN), tmux);

    let keys = vec![json!("enter"); MAX_SEND_KEYS + 1];
    let (status, body) = call(
        router(st.clone()),
        post_json("/sessions/steer/send", Some(TOKEN), json!({ "keys": keys })),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST, "body={body}");
    assert_eq!(body["error"]["code"], "too-many-keys");
    assert_eq!(body["error"]["details"]["limit"], MAX_SEND_KEYS);
}

/// The bound must admit the whole documented range: a request at exactly the
/// limit is still a legitimate caller. `sympoies/nils-cli#1542`.
#[tokio::test]
async fn send_admits_a_key_list_at_the_allocation_bound() {
    let tmp = tempfile::TempDir::new().unwrap();
    let tmux = minimal_tmux(tmp.path());
    seed_session(tmp.path(), "steer", "codex", "hs-codex-steer");
    let st = state(tmp.path(), Some(TOKEN), tmux);

    let keys = vec![json!("enter"); MAX_SEND_KEYS];
    let (status, body) = call(
        router(st.clone()),
        post_json("/sessions/steer/send", Some(TOKEN), json!({ "keys": keys })),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(
        body["data"]["sent"]["keys"]
            .as_array()
            .map(Vec::len)
            .unwrap_or_default(),
        MAX_SEND_KEYS
    );
}

/// A liveness-plus-journal tmux stub for the blocked-input tests. It answers
/// `has-session`, `capture-pane` and `show-buffer` and records every verb it
/// is called with; it deliberately does *not* implement `minimal_tmux`'s
/// `new-session` heartbeat or `display-message` pane identity, so a flow
/// needing session creation must use `minimal_tmux` instead of this.
///
/// The journal exists because a blocked-input refusal is only worth anything
/// if it happens *before* the daemon writes to the live pane, so these tests
/// assert the absence of a terminal write rather than trusting the response
/// code alone.
fn recording_tmux(dir: &Path) -> (PathBuf, PathBuf) {
    let bin = dir.join("recording-tmux");
    let journal = dir.join("tmux-invocations");
    let pane_stub = live_pane_stub_prelude(dir);
    std::fs::write(
            &bin,
            format!(
                "#!/usr/bin/env sh\n{pane_stub}\nprintf '%s\\n' \"$1\" >> '{journal}'\ncase \"$1\" in\n  has-session) exit 0 ;;\n  capture-pane) printf 'pane\\n'; exit 0 ;;\n  show-buffer) printf 'buffered selection\\n'; exit 0 ;;\n  *) exit 0 ;;\nesac\n",
                journal = journal.display()
            ),
        )
        .unwrap();
    let mut perms = std::fs::metadata(&bin).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&bin, perms).unwrap();
    (bin, journal)
}

/// Whether the stub observed a pane write. `expect`s the journal rather than
/// treating its absence as "no write": a missing file means the stub never
/// ran at all, which would silently satisfy every negative assertion and
/// leave a test passing for the wrong reason.
fn tmux_wrote_to_the_pane(journal: &Path) -> bool {
    let log = std::fs::read_to_string(journal)
        .expect("tmux journal: the recording stub was never invoked");
    log.lines()
        .any(|verb| matches!(verb, "send-keys" | "load-buffer" | "paste-buffer"))
}

/// The stronger claim: tmux was not invoked at all. A refusal that lands
/// before the daemon reaches for the terminal leaves no journal, and saying
/// so explicitly keeps that distinct from "ran and wrote nothing".
fn tmux_never_ran(journal: &Path) -> bool {
    !journal.exists()
}

/// Drive a seeded session into `NeedsInput` the only way the daemon ever
/// reaches it: a provider `attention_requested` lifecycle event.
fn block_on_provider_attention(state: &Arc<ServeState>, id: &str, provider: &str) {
    let record = load_session_record(&state.context, id).unwrap();
    crate::activity::activate_runtime(&state.context, &record).unwrap();
    let event = serde_json::from_value(json!({
        "schema_version": crate::activity::TURN_EVENT_VERSION,
        "event_id": format!("{id}-attention"),
        "runtime_id": format!("launch-{id}"),
        "provider": provider,
        "kind": "attention_requested",
        "confidence": "authoritative",
        "attention_id": format!("{id}-approval"),
        "attention_kind": "approval"
    }))
    .expect("attention event");
    crate::activity::ingest_event(&state.context, id, event).expect("ingest attention");
    let phase = crate::activity::activity_status(&state.context, id)
        .expect("activity status")
        .turn_state
        .phase;
    assert_eq!(
        phase,
        crate::activity::TurnPhase::NeedsInput,
        "fixture must actually be blocked"
    );
}

/// Typing literal text into a pane that is showing an approval or question
/// dialog is the exact hazard this contract exists to stop: the characters
/// land in the dialog, not in a prompt box. Refuse before touching the pane
/// and tell the caller what to do instead.
#[tokio::test]
async fn send_refuses_literal_text_while_the_agent_is_blocked() {
    let tmp = tempfile::TempDir::new().unwrap();
    let (tmux, journal) = recording_tmux(tmp.path());
    seed_session_with_runtime(tmp.path(), "blocked", "codex", "hs-codex-blocked");
    let st = state(tmp.path(), Some(TOKEN), tmux);
    block_on_provider_attention(&st, "blocked", "codex");

    let (status, body) = call(
        router(st.clone()),
        post_json(
            "/sessions/blocked/send",
            Some(TOKEN),
            json!({ "text": "run the migration", "keys": ["enter"] }),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::CONFLICT, "body={body}");
    assert_eq!(body["error"]["code"], "agent-blocked");
    assert_eq!(body["error"]["details"]["phase"], "needs_input");
    assert!(
        !tmux_wrote_to_the_pane(&journal),
        "a refused send must not reach the pane"
    );
}

/// Answering the dialog is the whole point of leaving the session usable
/// while it is blocked, and answering means special keys — including the
/// Enter that confirms a highlighted choice. Those must stay admitted.
#[tokio::test]
async fn send_admits_special_keys_while_the_agent_is_blocked() {
    let tmp = tempfile::TempDir::new().unwrap();
    let (tmux, journal) = recording_tmux(tmp.path());
    seed_session_with_runtime(tmp.path(), "blocked", "codex", "hs-codex-blocked");
    let st = state(tmp.path(), Some(TOKEN), tmux);
    block_on_provider_attention(&st, "blocked", "codex");

    let (status, body) = call(
        router(st.clone()),
        post_json(
            "/sessions/blocked/send",
            Some(TOKEN),
            json!({ "keys": ["down", "enter"] }),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["data"]["sent"]["sent_text"], false);
    assert!(
        tmux_wrote_to_the_pane(&journal),
        "answering the dialog must still reach the pane"
    );
}

/// A caller that genuinely means to type into the dialog's own text field
/// keeps an explicit opt-in. The guard is a default, not a wall.
#[tokio::test]
async fn send_admits_literal_text_while_blocked_when_the_caller_opts_in() {
    let tmp = tempfile::TempDir::new().unwrap();
    let (tmux, journal) = recording_tmux(tmp.path());
    seed_session_with_runtime(tmp.path(), "blocked", "codex", "hs-codex-blocked");
    let st = state(tmp.path(), Some(TOKEN), tmux);
    block_on_provider_attention(&st, "blocked", "codex");

    let (status, body) = call(
        router(st.clone()),
        post_json(
            "/sessions/blocked/send",
            Some(TOKEN),
            json!({ "text": "custom answer", "allow_blocked": true }),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["data"]["sent"]["sent_text"], true);
    assert!(
        tmux_wrote_to_the_pane(&journal),
        "an opted-in send must still reach the pane"
    );
}

/// The Claude prompt route pastes into the pane, so it carries the hazard
/// and is refused. This fixture is the one that would actually deliver: with
/// the guard removed the prompt reaches `load-buffer`/`paste-buffer`, so the
/// journal assertion has real signal.
#[tokio::test]
async fn structured_prompt_refuses_while_a_claude_pane_is_blocked() {
    let tmp = tempfile::TempDir::new().unwrap();
    let (tmux, journal) = recording_tmux(tmp.path());
    seed_claude_prompt_session(tmp.path(), "blocked");
    let st = state(tmp.path(), Some(TOKEN), tmux);
    block_on_provider_attention(&st, "blocked", "claude");

    let (status, body) = call(
        router(st.clone()),
        post_json(
            "/sessions/blocked/prompt",
            Some(TOKEN),
            json!({ "text": "please continue" }),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::CONFLICT, "body={body}");
    assert_eq!(body["error"]["code"], "agent-blocked");
    assert!(
        tmux_never_ran(&journal),
        "a refused prompt must be rejected before the daemon reaches for tmux"
    );
}

/// The Codex app-server route submits over the control channel and never
/// writes to the pane. Read-only composer admission preserves owner drafts;
/// a blocked turn with an empty composer still admits a reply. In-turn coordination
/// checkpoint delivers to a `NeedsInput` Codex session over the same
/// channel; refusing callers for that would contradict it.
#[tokio::test]
async fn structured_prompt_admits_a_blocked_codex_app_server_session() {
    let tmp = tempfile::TempDir::new().unwrap();
    let launch_id = seed_codex_app_server_session(tmp.path(), "blocked");
    let st = state(tmp.path(), Some(TOKEN), empty_composer_tmux(tmp.path()));
    block_on_provider_attention(&st, "blocked", "codex");
    let (handle, mut commands) = codex_app_server::control_channel();
    st.codex_controls.lock().unwrap().insert(
        "blocked".to_string(),
        CodexControlEntry {
            launch_id: launch_id.clone(),
            handle,
        },
    );
    let responder = tokio::spawn(async move {
        let Some(codex_app_server::ControlCommand::Prompt { response, .. }) = commands.recv().await
        else {
            panic!("the prompt did not reach the Codex control plane");
        };
        response.send(Ok("turn-blocked".to_string())).unwrap();
    });

    let (status, body) = call(
        router(st.clone()),
        post_json(
            "/sessions/blocked/prompt",
            Some(TOKEN),
            json!({ "text": "please continue" }),
        ),
    )
    .await;

    responder.await.unwrap();
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_ne!(body["error"]["code"], "agent-blocked");
}

/// The guard must fire on `needs_input` and nothing else. Every other send
/// test seeds a session with no activity document at all, so without this
/// case a regression that refused whenever a turn state exists would break
/// ordinary steering in production while the suite stayed green.
#[tokio::test]
async fn send_admits_literal_text_while_the_agent_is_working() {
    let tmp = tempfile::TempDir::new().unwrap();
    let (tmux, journal) = recording_tmux(tmp.path());
    seed_claude_prompt_session(tmp.path(), "busy");
    let st = state(tmp.path(), Some(TOKEN), tmux);
    let record = load_session_record(&st.context, "busy").unwrap();
    crate::activity::activate_runtime(&st.context, &record).unwrap();
    crate::activity::ingest_event(
        &st.context,
        "busy",
        claude_turn_started_event("launch-busy", "busy-started", "turn-busy"),
    )
    .expect("ingest turn start");
    assert_eq!(
        crate::activity::activity_status(&st.context, "busy")
            .expect("activity status")
            .turn_state
            .phase,
        crate::activity::TurnPhase::Working,
        "fixture must have a live non-blocked turn state"
    );

    let (status, body) = call(
        router(st.clone()),
        post_json(
            "/sessions/busy/send",
            Some(TOKEN),
            json!({ "text": "keep going", "keys": ["enter"] }),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body={body}");
    assert!(
        tmux_wrote_to_the_pane(&journal),
        "a working session must still accept typed input"
    );
}

/// `send_input_unlocked` delivers a bare newline as an Enter keypress rather
/// than pasted characters, so the contract admits it while blocked. Without
/// this case, tightening the predicate to "any text" would silently remove a
/// documented way to answer a dialog.
#[tokio::test]
async fn send_admits_a_bare_newline_while_the_agent_is_blocked() {
    let tmp = tempfile::TempDir::new().unwrap();
    let (tmux, journal) = recording_tmux(tmp.path());
    seed_session_with_runtime(tmp.path(), "blocked", "codex", "hs-codex-blocked");
    let st = state(tmp.path(), Some(TOKEN), tmux);
    block_on_provider_attention(&st, "blocked", "codex");

    let (status, body) = call(
        router(st.clone()),
        post_json(
            "/sessions/blocked/send",
            Some(TOKEN),
            json!({ "text": "\n" }),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body={body}");
    assert!(
        tmux_wrote_to_the_pane(&journal),
        "a bare newline is an Enter keypress and must still answer the dialog"
    );
}

/// The Claude title rename projects `/rename <title>` plus Enter into the
/// pane, so it carries the same hazard as `send` and is refused while
/// blocked. The persisted title still advances: only the pane-side
/// projection is suppressed.
#[tokio::test]
async fn title_update_does_not_project_into_a_blocked_claude_pane() {
    let tmp = tempfile::TempDir::new().unwrap();
    let (tmux, journal) = recording_tmux(tmp.path());
    seed_claude_prompt_session(tmp.path(), "blocked");
    let st = state(tmp.path(), Some(TOKEN), tmux);
    block_on_provider_attention(&st, "blocked", "claude");

    let (status, body) = call(
        router(st.clone()),
        patch_json(
            "/sessions/blocked",
            Some(TOKEN),
            json!({ "title": "renamed while blocked" }),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body={body}");
    assert!(
        !tmux_wrote_to_the_pane(&journal),
        "the rename projection must not type into a blocked pane"
    );
    assert_eq!(
        load_session_record(&st.context, "blocked")
            .unwrap()
            .title
            .as_deref(),
        Some("renamed while blocked"),
        "the persisted title must still advance"
    );
}

#[tokio::test]
async fn structured_prompt_submits_exact_multiline_without_terminal_paste() {
    let tmp = tempfile::TempDir::new().unwrap();
    let launch_id = seed_codex_app_server_session(tmp.path(), "structured");
    let st = state(tmp.path(), Some(TOKEN), empty_composer_tmux(tmp.path()));
    let record = load_session_record(&st.context, "structured").unwrap();
    crate::activity::activate_runtime(&st.context, &record).unwrap();
    auto_resume::set_enabled(&st.context, &record.id, true, "2030-01-01T00:00:00Z").unwrap();
    let (handle, mut commands) = codex_app_server::control_channel();
    st.codex_controls.lock().unwrap().insert(
        "structured".to_string(),
        CodexControlEntry {
            launch_id: launch_id.clone(),
            handle,
        },
    );
    let prompt = "inspect first line\nthen second line";
    let responder_context = st.context.clone();
    let responder_record = record.clone();
    let responder = tokio::spawn(async move {
        let Some(codex_app_server::ControlCommand::Prompt { message, response }) =
            commands.recv().await
        else {
            panic!("structured prompt did not reach the Codex control plane");
        };
        assert_eq!(message, prompt);
        let view = auto_resume::view_for_record(&responder_context, &responder_record);
        assert!(view.enabled);
        assert_eq!(view.state, "enabled");
        response.send(Ok("turn-structured".to_string())).unwrap();
    });

    let (status, body) = call(
        router(st.clone()),
        post_json(
            "/sessions/structured/prompt",
            None,
            json!({ "text": prompt }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "body={body}");

    let (status, body) = call(
        router(st.clone()),
        post_json(
            "/sessions/structured/prompt/v2",
            Some(TOKEN),
            json!({
                "text": prompt,
                "expected_session_incarnation": launch_id,
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["data"]["submitted"], true);
    assert_eq!(body["data"]["session_incarnation"], "launch-structured");
    assert!(body.pointer("/data/turn_id").is_none());
    responder.await.unwrap();
}

fn seed_claude_prompt_session(state_dir: &Path, id: &str) -> String {
    seed_session_with_runtime(state_dir, id, "claude", &format!("hs-claude-{id}"));
    format!("launch-{id}")
}

fn claude_turn_started_event(
    launch_id: &str,
    event_id: &str,
    turn_id: &str,
) -> crate::activity::TurnEvent {
    serde_json::from_value(json!({
        "schema_version": crate::activity::TURN_EVENT_VERSION,
        "event_id": event_id,
        "runtime_id": launch_id,
        "provider": "claude",
        "provider_turn_id": turn_id,
        "kind": "turn_started",
        "confidence": "observed"
    }))
    .unwrap()
}

async fn response_parts(response: Response) -> (StatusCode, Value) {
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

#[tokio::test]
async fn claude_structured_prompt_reports_submitted_after_provider_turn() {
    let tmp = tempfile::TempDir::new().unwrap();
    let launch_id = seed_claude_prompt_session(tmp.path(), "claude-structured");
    let (tmux, entered) = empty_composer_tmux_marking_enter(tmp.path());
    let st = state(tmp.path(), Some(TOKEN), tmux);
    let record = load_session_record(&st.context, "claude-structured").unwrap();
    crate::activity::activate_runtime(&st.context, &record).unwrap();

    let ack_context = st.context.clone();
    let ack_id = record.id.clone();
    let ack_launch_id = launch_id.clone();
    let acknowledger = tokio::spawn(async move {
        wait_for_file(&entered).await;
        tokio::task::spawn_blocking(move || {
            crate::activity::ingest_event(
                &ack_context,
                &ack_id,
                claude_turn_started_event(&ack_launch_id, "claude-start", "turn-claude"),
            )
            .unwrap();
        })
        .await
        .unwrap();
    });

    let (status, body) = call(
        router(st.clone()),
        post_json(
            "/sessions/claude-structured/prompt/v2",
            Some(TOKEN),
            json!({
                "text": "inspect the created session",
                "expected_session_incarnation": launch_id,
            }),
        ),
    )
    .await;
    acknowledger.await.unwrap();
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["data"]["submitted"], true);
    assert!(body["data"].get("queued").is_none());
    assert_eq!(body["data"]["session_incarnation"], launch_id);
}

#[tokio::test]
async fn claude_structured_prompt_without_provider_turn_reports_unknown_outcome() {
    let tmp = tempfile::TempDir::new().unwrap();
    let launch_id = seed_claude_prompt_session(tmp.path(), "claude-unacknowledged");
    let st = state(tmp.path(), Some(TOKEN), empty_composer_tmux(tmp.path()));
    let record = load_session_record(&st.context, "claude-unacknowledged").unwrap();
    crate::activity::activate_runtime(&st.context, &record).unwrap();

    let (status, body) = response_parts(
        submit_claude_structured_prompt(
            &st,
            &record,
            &launch_id,
            Some(&launch_id),
            "inspect the created session",
            Duration::from_millis(200),
        )
        .await,
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body={body}");
    assert_eq!(body["error"]["code"], "structured-prompt-outcome-unknown");
    assert!(body.pointer("/data/submitted").is_none());
}

const QUEUED_CLAUDE_PANE: &str = "  Second queued message.
  ctrl+x ctrl+s to send now
· Galloping… (running PreToolUse hook · 4s)
────────────────────────────────────────
❯\u{a0}Press up to edit queued messages
────────────────────────────────────────
  ⏵⏵ bypass permissions on (shift+tab to cycle) · ← for agents
";

#[tokio::test]
async fn claude_structured_prompt_queued_behind_a_running_turn_reports_submitted_and_queued() {
    let tmp = tempfile::TempDir::new().unwrap();
    let launch_id = seed_claude_prompt_session(tmp.path(), "claude-queued");
    let st = state(
        tmp.path(),
        Some(TOKEN),
        minimal_tmux_showing(tmp.path(), QUEUED_CLAUDE_PANE),
    );
    let record = load_session_record(&st.context, "claude-queued").unwrap();
    crate::activity::activate_runtime(&st.context, &record).unwrap();

    let (status, body) = response_parts(
        submit_claude_structured_prompt(
            &st,
            &record,
            &launch_id,
            Some(&launch_id),
            "Second queued message.",
            Duration::from_millis(200),
        )
        .await,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["data"]["submitted"], true);
    assert_eq!(body["data"]["queued"], true);
    assert_eq!(body["data"]["session_incarnation"], launch_id);
}

#[tokio::test]
async fn claude_structured_prompt_is_unknown_when_the_queue_lists_another_message() {
    let tmp = tempfile::TempDir::new().unwrap();
    let launch_id = seed_claude_prompt_session(tmp.path(), "claude-queue-other");
    let st = state(
        tmp.path(),
        Some(TOKEN),
        minimal_tmux_showing(tmp.path(), QUEUED_CLAUDE_PANE),
    );
    let record = load_session_record(&st.context, "claude-queue-other").unwrap();
    crate::activity::activate_runtime(&st.context, &record).unwrap();

    let (status, body) = response_parts(
        submit_claude_structured_prompt(
            &st,
            &record,
            &launch_id,
            Some(&launch_id),
            "A prompt the queue never received",
            Duration::from_millis(200),
        )
        .await,
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body={body}");
    assert_eq!(body["error"]["code"], "structured-prompt-outcome-unknown");
}

#[tokio::test]
async fn claude_structured_prompt_ignores_progress_on_an_existing_turn() {
    let tmp = tempfile::TempDir::new().unwrap();
    let launch_id = seed_claude_prompt_session(tmp.path(), "claude-in-flight");
    let st = state(tmp.path(), Some(TOKEN), empty_composer_tmux(tmp.path()));
    let record = load_session_record(&st.context, "claude-in-flight").unwrap();
    crate::activity::activate_runtime(&st.context, &record).unwrap();
    crate::activity::ingest_event(
        &st.context,
        &record.id,
        claude_turn_started_event(&launch_id, "in-flight-start", "turn-in-flight"),
    )
    .unwrap();

    let ack_context = st.context.clone();
    let ack_id = record.id.clone();
    let ack_launch_id = launch_id.clone();
    let progress = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(50)).await;
        tokio::task::spawn_blocking(move || {
            let event: crate::activity::TurnEvent = serde_json::from_value(json!({
                "schema_version": crate::activity::TURN_EVENT_VERSION,
                "event_id": "in-flight-progress",
                "runtime_id": ack_launch_id,
                "provider": "claude",
                "provider_turn_id": "turn-in-flight",
                "kind": "progress",
                "confidence": "observed"
            }))
            .unwrap();
            crate::activity::ingest_event(&ack_context, &ack_id, event).unwrap();
        })
        .await
        .unwrap();
    });

    let (status, body) = response_parts(
        submit_claude_structured_prompt(
            &st,
            &record,
            &launch_id,
            Some(&launch_id),
            "queue another instruction",
            Duration::from_millis(400),
        )
        .await,
    )
    .await;
    progress.await.unwrap();
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body={body}");
    assert_eq!(body["error"]["code"], "structured-prompt-outcome-unknown");
}

#[tokio::test]
async fn structured_prompt_compatibility_route_honors_stale_expected_incarnation() {
    let tmp = tempfile::TempDir::new().unwrap();
    let launch_id = seed_codex_app_server_session(tmp.path(), "structured-compat-stale");
    let st = state(tmp.path(), Some(TOKEN), minimal_tmux(tmp.path()));
    let record = load_session_record(&st.context, "structured-compat-stale").unwrap();
    crate::activity::activate_runtime(&st.context, &record).unwrap();
    let (handle, mut commands) = codex_app_server::control_channel();
    st.codex_controls
        .lock()
        .unwrap()
        .insert(record.id.clone(), CodexControlEntry { launch_id, handle });
    let dispatch = tokio::spawn(async move {
        let Ok(Some(codex_app_server::ControlCommand::Prompt { response, .. })) =
            tokio::time::timeout(Duration::from_millis(250), commands.recv()).await
        else {
            return false;
        };
        response
            .send(Ok("turn-must-not-submit".to_string()))
            .unwrap();
        true
    });

    let (status, body) = call(
        router(st),
        post_json(
            "/sessions/structured-compat-stale/prompt",
            Some(TOKEN),
            json!({
                "text": "must honor the optional compatibility fence",
                "expected_session_incarnation": "launch-structured-compat-previous",
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "body={body}");
    assert_eq!(body["error"]["code"], "session-incarnation-conflict");
    assert!(
        !dispatch.await.unwrap(),
        "stale compatibility prompt reached provider control"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn structured_prompt_fences_runtime_replacement_under_record_lock() {
    let tmp = tempfile::TempDir::new().unwrap();
    let launch_id = seed_codex_app_server_session(tmp.path(), "structured-stale");
    let st = state(tmp.path(), Some(TOKEN), minimal_tmux(tmp.path()));
    let record = load_session_record(&st.context, "structured-stale").unwrap();
    crate::activity::activate_runtime(&st.context, &record).unwrap();
    let (handle, mut commands) = codex_app_server::control_channel();
    st.codex_controls.lock().unwrap().insert(
        "structured-stale".to_string(),
        CodexControlEntry {
            launch_id: launch_id.clone(),
            handle,
        },
    );
    let record_lock = crate::acquire_session_record_lock(&st.context, "structured-stale").unwrap();
    let expected_launch_id = launch_id.clone();
    let request_state = st.clone();
    let request = tokio::spawn(async move {
        call(
            router(request_state),
            post_json(
                "/sessions/structured-stale/prompt/v2",
                Some(TOKEN),
                json!({
                    "text": "must not reach the replacement runtime",
                    "expected_session_incarnation": expected_launch_id,
                }),
            ),
        )
        .await
    });

    sync_waits::wait_for_record_lock_waiter(tmp.path(), "structured-stale").await;
    assert!(
        !request.is_finished(),
        "request must wait for the record lock"
    );
    let record_path = tmp.path().join("sessions/structured-stale/session.json");
    let mut replacement: Value =
        serde_json::from_str(&std::fs::read_to_string(&record_path).unwrap()).unwrap();
    replacement["runtime"]["launch_id"] = json!("launch-structured-stale-v2");
    std::fs::write(
        &record_path,
        serde_json::to_vec_pretty(&replacement).unwrap(),
    )
    .unwrap();
    drop(record_lock);

    let (status, body) = request.await.unwrap();
    assert_eq!(status, StatusCode::CONFLICT, "body={body}");
    assert_eq!(body["error"]["code"], "session-incarnation-conflict");
    assert_eq!(
        body["error"]["details"]["actual_session_incarnation"],
        "launch-structured-stale-v2"
    );
    assert!(
        matches!(
            commands.try_recv(),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty)
        ),
        "stale prompt reached provider control"
    );
}

#[tokio::test]
async fn structured_prompt_control_wait_rechecks_runtime_replacement() {
    for (id, explicit_fence) in [
        ("structured-wait-replace-v2", true),
        ("structured-wait-replace-compat", false),
    ] {
        let tmp = tempfile::TempDir::new().unwrap();
        let launch_id = seed_codex_app_server_session(tmp.path(), id);
        let st = state(tmp.path(), Some(TOKEN), minimal_tmux(tmp.path()));
        let expected_session_incarnation = explicit_fence.then_some(launch_id.as_str());
        let record = load_structured_prompt_record_locked(&st, id, expected_session_incarnation)
            .await
            .expect("initial structured prompt preflight");
        assert_eq!(
            record
                .runtime
                .as_ref()
                .map(|runtime| runtime.launch_id.as_str()),
            Some(launch_id.as_str())
        );

        let replacement_launch_id = format!("{launch_id}-replacement");
        let record_path = tmp.path().join(format!("sessions/{id}/session.json"));
        let mut replacement: Value =
            serde_json::from_str(&std::fs::read_to_string(&record_path).unwrap()).unwrap();
        replacement["runtime"]["launch_id"] = json!(replacement_launch_id);
        std::fs::write(
            &record_path,
            serde_json::to_vec_pretty(&replacement).unwrap(),
        )
        .unwrap();

        let response = match wait_for_structured_prompt_control(
            &st,
            id,
            &launch_id,
            expected_session_incarnation,
            Duration::ZERO,
        )
        .await
        {
            Ok(_) => panic!("replacement must supersede unavailable control"),
            Err(response) => response,
        };
        assert_eq!(response.status(), StatusCode::CONFLICT);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["error"]["code"], "session-incarnation-conflict");
        assert_eq!(
            body["error"]["details"]["actual_session_incarnation"], replacement_launch_id,
            "explicit_fence={explicit_fence}"
        );
    }
}

#[tokio::test]
async fn structured_prompt_waits_for_control_startup_before_record_lock() {
    use tokio_tungstenite::tungstenite::Message;
    let tmp = tempfile::tempdir_in("/tmp").unwrap();
    let id = "startup-prompt";
    let launch_id = seed_codex_app_server_session(tmp.path(), id);
    let st = state(tmp.path(), Some(TOKEN), empty_composer_tmux(tmp.path()));
    let record = load_session_record(&st.context, id).unwrap();
    let listener = tokio::net::UnixListener::bind(tmp.path().join("startup-prompt.sock")).unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        while let Some(Ok(message)) = socket.next().await {
            let Message::Text(text) = message else {
                continue;
            };
            let request: Value = serde_json::from_str(&text).unwrap();
            let Some(request_id) = request.get("id") else {
                continue;
            };
            let result = match request["method"].as_str().unwrap() {
                "initialize" | "thread/resume" => json!({}),
                "thread/loaded/list" => json!({"data": ["startup-thread"]}),
                "account/rateLimits/read" => json!({"rateLimits": {}}),
                "turn/start" => json!({"turn": {"id": "startup-turn"}}),
                method => panic!("unexpected request: {method}"),
            };
            socket
                .send(Message::Text(
                    json!({"id": request_id, "result": result})
                        .to_string()
                        .into(),
                ))
                .await
                .unwrap();
        }
    });
    let (handle, commands, ready) = codex_app_server::starting_control_channel();
    let submit_state = st.clone();
    let submission = tokio::spawn(async move {
        tokio::time::timeout(
            Duration::from_secs(2),
            submit_structured_prompt_locked(
                &submit_state,
                id,
                &launch_id,
                StructuredPromptGuards::default(),
                &handle,
                "first prompt",
            ),
        )
        .await
    });
    // Registering a control is not startup readiness. Queue the first
    // request before the real worker persists its loaded thread, as a
    // create can do immediately after account binding.
    tokio::time::timeout(Duration::from_secs(2), async {
        while commands.is_empty() && ready.receiver_count() == 1 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let control = tokio::spawn(codex_app_server::run_control(
        st.context.clone(),
        record,
        commands,
        ready,
    ));
    let result = submission.await.unwrap();
    control.abort();
    let _ = control.await;
    server.abort();
    let _ = server.await;
    assert!(
        matches!(result, Ok(Ok(_))),
        "first prompt must acknowledge without blocking startup record persistence: {result:?}"
    );
    assert_eq!(
        load_session_record(&st.context, id)
            .unwrap()
            .provider_resume
            .unwrap()
            .session_id,
        "startup-thread"
    );
}

#[tokio::test]
async fn structured_prompt_final_lock_rejects_replacement_after_initial_read() {
    let tmp = tempfile::TempDir::new().unwrap();
    let launch_id = seed_codex_app_server_session(tmp.path(), "structured-final-fence");
    let st = state(tmp.path(), Some(TOKEN), minimal_tmux(tmp.path()));
    let (handle, mut commands) = codex_app_server::control_channel();
    let record_path = tmp
        .path()
        .join("sessions/structured-final-fence/session.json");
    let mut replacement: Value =
        serde_json::from_str(&std::fs::read_to_string(&record_path).unwrap()).unwrap();
    replacement["runtime"]["launch_id"] = json!("launch-structured-final-fence-v2");
    std::fs::write(
        &record_path,
        serde_json::to_vec_pretty(&replacement).unwrap(),
    )
    .unwrap();

    let response = submit_structured_prompt_locked(
        &st,
        "structured-final-fence",
        &launch_id,
        StructuredPromptGuards {
            expected_session_incarnation: Some(&launch_id),
            ..Default::default()
        },
        &handle,
        "must not cross the final locked recheck",
    )
    .await
    .unwrap_err();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["error"]["code"], "session-incarnation-conflict");
    assert_eq!(
        body["error"]["details"]["actual_session_incarnation"],
        "launch-structured-final-fence-v2"
    );
    assert!(matches!(
        commands.try_recv(),
        Err(tokio::sync::mpsc::error::TryRecvError::Empty)
    ));
}

#[tokio::test]
async fn coordination_review_notification_final_lock_rejects_nonwaiting_activity() {
    let tmp = tempfile::TempDir::new().unwrap();
    let launch_id = seed_codex_app_server_session(tmp.path(), "notification-final-fence");
    let st = state(tmp.path(), Some(TOKEN), minimal_tmux(tmp.path()));
    let (handle, mut commands) = codex_app_server::control_channel();
    let response = submit_structured_prompt_locked(
        &st,
        "notification-final-fence",
        &launch_id,
        StructuredPromptGuards {
            expected_session_incarnation: Some(&launch_id),
            notification: Some(CoordinationNotificationFence {
                candidate: crate::coordination::NotificationCandidate {
                    prompt: None,
                    target_session_id: "notification-final-fence".to_string(),
                    target_incarnation: launch_id.clone(),
                    generation: 1,
                    queued_at_epoch: 0,
                    attempted_at_epoch: 0,
                    attempted_at: None,
                },
            }),
            ..Default::default()
        },
        &handle,
        "fixed coordination notification",
    )
    .await
    .unwrap_err();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        body["error"]["code"],
        "coordination-notification-superseded"
    );
    assert!(matches!(
        commands.try_recv(),
        Err(tokio::sync::mpsc::error::TryRecvError::Empty)
    ));
}

#[test]
fn coordination_notifications_refuse_providers_without_a_delivery_path() {
    let tmp = tempfile::TempDir::new().unwrap();
    let context = crate::CliContext {
        state_dir: tmp.path().to_path_buf(),
        host: None,
    };
    for (id, agent, runtime_kind, unsupported) in [
        ("retained-hermes", "hermes", "tmux", true),
        ("dsh-pane", "dsh", "tmux", false),
        ("codex-pane", "codex", "tmux", false),
        ("claude-pane", "claude", "tmux", false),
    ] {
        seed_session_with_runtime(tmp.path(), id, agent, &format!("hs-{id}"));
        let record_path = tmp.path().join("sessions").join(id).join("session.json");
        let mut record: Value =
            serde_json::from_str(&std::fs::read_to_string(&record_path).unwrap()).unwrap();
        record["runtime"]["kind"] = json!(runtime_kind);
        std::fs::write(&record_path, serde_json::to_vec_pretty(&record).unwrap()).unwrap();
        let record = load_session_record(&context, id).unwrap();
        assert_eq!(
            coordination_notification_provider_unsupported(&record),
            unsupported,
            "agent={agent} runtime={runtime_kind}"
        );
    }
}

#[tokio::test]
async fn coordination_review_wait_workers_are_strictly_bounded() {
    let tmp = tempfile::TempDir::new().unwrap();
    let st = state(tmp.path(), Some(TOKEN), minimal_tmux(tmp.path()));
    let mut permits = Vec::new();
    for _ in 0..COORDINATION_WAIT_WORKER_LIMIT {
        permits.push(
            st.coordination_wait_workers
                .clone()
                .try_acquire_owned()
                .expect("bounded permit"),
        );
    }
    assert!(
        st.coordination_wait_workers
            .clone()
            .try_acquire_owned()
            .is_err()
    );
    drop(permits);
}

#[tokio::test]
async fn codex_create_prompt_control_wait_honors_startup_budget() {
    assert!(CODEX_CREATE_PROMPT_CONTROL_WAIT > STRUCTURED_PROMPT_CONTROL_WAIT);

    let tmp = tempfile::TempDir::new().unwrap();
    let st = state(tmp.path(), Some(TOKEN), minimal_tmux(tmp.path()));
    let launch_id = "delayed-create-control".to_string();
    let delayed_state = st.clone();
    let delayed_launch_id = launch_id.clone();
    let registration = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(75)).await;
        let (handle, _commands) = codex_app_server::control_channel();
        delayed_state.codex_controls.lock().unwrap().insert(
            "delayed-create".to_string(),
            CodexControlEntry {
                launch_id: delayed_launch_id,
                handle,
            },
        );
    });

    let handle = wait_for_codex_control_within(
        &st,
        "delayed-create",
        &launch_id,
        Duration::from_millis(250),
    )
    .await;

    registration.await.unwrap();
    assert!(
        handle.is_some(),
        "control registered within the startup budget"
    );
}

#[tokio::test]
async fn codex_account_switch_queues_during_a_working_turn_without_control_dispatch() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().unwrap();
    let _broker = codex_listing_broker(&lock, tmp.path(), &["acct1", "omega"]);
    let launch_id = seed_codex_app_server_session(tmp.path(), "account-switch-queued");
    let st = state(tmp.path(), Some(TOKEN), minimal_tmux(tmp.path()));
    let mut record = load_session_record(&st.context, "account-switch-queued").unwrap();
    crate::codex_account::set_initial_binding(&mut record, Some("acct1")).unwrap();
    crate::write_session_record(&st.context, &record).unwrap();
    crate::codex_account::finish_binding(&st.context, &record.id, &launch_id, "acct1", 1, Ok(()))
        .unwrap();
    record = load_session_record(&st.context, &record.id).unwrap();
    crate::activity::activate_runtime(&st.context, &record).unwrap();
    let event = serde_json::from_value(json!({
        "schema_version": crate::activity::TURN_EVENT_VERSION,
        "event_id": "turn-start",
        "runtime_id": launch_id,
        "provider": "codex",
        "provider_turn_id": "turn-account-switch-queued",
        "kind": "turn_started",
        "confidence": "authoritative"
    }))
    .unwrap();
    crate::activity::ingest_event(&st.context, &record.id, event).unwrap();

    let (handle, mut commands) = codex_app_server::control_channel();
    st.codex_controls.lock().unwrap().insert(
        record.id.clone(),
        CodexControlEntry {
            launch_id: launch_id.clone(),
            handle,
        },
    );

    let (status, body) = call(
        router(st.clone()),
        put_json(
            "/sessions/account-switch-queued/account",
            Some(TOKEN),
            json!({ "account": "omega", "expected_session_incarnation": launch_id }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["data"]["codex_account"]["selected_account"], "acct1");
    assert_eq!(body["data"]["codex_account"]["next"]["account"], "omega");
    assert_eq!(body["data"]["codex_account"]["next"]["state"], "queued");
    assert!(matches!(
        commands.try_recv(),
        Err(tokio::sync::mpsc::error::TryRecvError::Empty)
    ));

    let persisted = load_session_record(&st.context, &record.id).unwrap();
    let view = crate::codex_account::view_for_record(&persisted);
    assert_eq!(view.selected_account.as_deref(), Some("acct1"));
    assert_eq!(view.next.unwrap().account.as_deref(), Some("omega"));
}

#[tokio::test]
async fn codex_account_switch_to_the_bound_account_skips_the_broker_list_check() {
    let lock = GlobalStateLock::new();
    let _no_claude = EnvGuard::remove(&lock, "AGENT_SESSION_CLAUDE_ACCOUNT_BROKER");
    let tmp = tempfile::TempDir::new().unwrap();
    // The broker lists `omega` but not the bound `acct1`.
    let listing = codex_listing_broker(&lock, tmp.path(), &["omega"]);
    let launch_id = seed_codex_app_server_session(tmp.path(), "account-switch-reselect");
    let st = state(tmp.path(), Some(TOKEN), minimal_tmux(tmp.path()));
    let mut record = load_session_record(&st.context, "account-switch-reselect").unwrap();
    crate::codex_account::set_initial_binding(&mut record, Some("acct1")).unwrap();
    crate::write_session_record(&st.context, &record).unwrap();
    crate::codex_account::finish_binding(&st.context, &record.id, &launch_id, "acct1", 1, Ok(()))
        .unwrap();
    record = load_session_record(&st.context, &record.id).unwrap();
    crate::activity::activate_runtime(&st.context, &record).unwrap();
    let event = serde_json::from_value(json!({
        "schema_version": crate::activity::TURN_EVENT_VERSION,
        "event_id": "turn-start",
        "runtime_id": launch_id,
        "provider": "codex",
        "provider_turn_id": "turn-account-switch-reselect",
        "kind": "turn_started",
        "confidence": "authoritative"
    }))
    .unwrap();
    crate::activity::ingest_event(&st.context, &record.id, event).unwrap();
    let (handle, mut commands) = codex_app_server::control_channel();
    st.codex_controls.lock().unwrap().insert(
        record.id.clone(),
        CodexControlEntry {
            launch_id: launch_id.clone(),
            handle,
        },
    );
    let switch = |account: &str| {
        put_json(
            "/sessions/account-switch-reselect/account",
            Some(TOKEN),
            json!({ "account": account, "expected_session_incarnation": launch_id }),
        )
    };

    let (status, body) = call(router(st.clone()), switch("omega")).await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["data"]["codex_account"]["next"]["account"], "omega");

    // Re-selecting the bound account must not consult the broker list: a
    // failing broker that records any call replaces the listing one.
    drop(listing);
    let calls = tmp.path().join("failing-broker-calls");
    let failing = executable(
        &tmp.path().join("failing-broker"),
        &format!(
            "#!/usr/bin/env sh\nprintf '%s\\n' \"$*\" >> {}\nexit 1\n",
            shell_words::quote(&calls.to_string_lossy())
        ),
    );
    let _failing = EnvGuard::set(
        &lock,
        "AGENT_SESSION_CODEX_ACCOUNT_BROKER",
        &serde_json::to_string(&vec![failing.to_string_lossy().into_owned()]).unwrap(),
    );
    let (status, body) = call(router(st.clone()), switch("acct1")).await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["data"]["codex_account"]["selected_account"], "acct1");
    assert!(
        body["data"]["codex_account"].get("next").is_none(),
        "re-selecting the bound account cancels the queued intent: {body}"
    );
    assert!(!calls.exists(), "the broker list must not be consulted");
    assert!(matches!(
        commands.try_recv(),
        Err(tokio::sync::mpsc::error::TryRecvError::Empty)
    ));
    let persisted = load_session_record(&st.context, &record.id).unwrap();
    assert!(
        crate::codex_account::view_for_record(&persisted)
            .next
            .is_none()
    );
}

#[tokio::test]
async fn codex_account_switch_queues_an_unbound_session_during_a_working_turn() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().unwrap();
    let _broker = codex_listing_broker(&lock, tmp.path(), &["acct1", "omega"]);
    let launch_id = seed_codex_app_server_session(tmp.path(), "unbound-account-switch-queued");
    let st = state(tmp.path(), Some(TOKEN), minimal_tmux(tmp.path()));
    let record = load_session_record(&st.context, "unbound-account-switch-queued").unwrap();
    crate::activity::activate_runtime(&st.context, &record).unwrap();
    let event = serde_json::from_value(json!({
        "schema_version": crate::activity::TURN_EVENT_VERSION,
        "event_id": "unbound-turn-start",
        "runtime_id": launch_id,
        "provider": "codex",
        "provider_turn_id": "turn-unbound-account-switch-queued",
        "kind": "turn_started",
        "confidence": "authoritative"
    }))
    .unwrap();
    crate::activity::ingest_event(&st.context, &record.id, event).unwrap();

    let (handle, mut commands) = codex_app_server::control_channel();
    st.codex_controls.lock().unwrap().insert(
        record.id.clone(),
        CodexControlEntry {
            launch_id: launch_id.clone(),
            handle,
        },
    );

    let (status, body) = call(
        router(st.clone()),
        put_json(
            "/sessions/unbound-account-switch-queued/account",
            Some(TOKEN),
            json!({ "account": "omega", "expected_session_incarnation": launch_id }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body={body}");
    assert_eq!(body["error"]["code"], "codex-account-not-bound");
    assert!(
        crate::codex_account::view_for_record(
            &load_session_record(&st.context, &record.id).unwrap()
        )
        .next
        .is_none()
    );

    let (status, body) = call(
        router(st.clone()),
        put_json(
            "/sessions/unbound-account-switch-queued/account",
            Some(TOKEN),
            json!({
                "account": "omega",
                "expected_session_incarnation": launch_id,
                "supports_unbound_account_queue": true
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["data"]["session_incarnation"], launch_id);
    assert_eq!(body["data"]["codex_account"]["state"], "unbound");
    assert_eq!(body["data"]["codex_account"]["revision"], 0);
    assert_eq!(body["data"]["codex_account"]["next"]["account"], "omega");
    assert_eq!(body["data"]["codex_account"]["next"]["state"], "queued");
    assert!(matches!(
        commands.try_recv(),
        Err(tokio::sync::mpsc::error::TryRecvError::Empty)
    ));
}

#[tokio::test]
async fn codex_account_switch_refuses_an_account_the_broker_does_not_list() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().unwrap();
    let _broker = codex_listing_broker(&lock, tmp.path(), &["acct1"]);
    let launch_id = seed_codex_app_server_session(tmp.path(), "account-switch-unknown");
    let st = state(tmp.path(), Some(TOKEN), minimal_tmux(tmp.path()));
    let record = load_session_record(&st.context, "account-switch-unknown").unwrap();
    crate::activity::activate_runtime(&st.context, &record).unwrap();
    let (handle, mut commands) = codex_app_server::control_channel();
    st.codex_controls.lock().unwrap().insert(
        record.id.clone(),
        CodexControlEntry {
            launch_id: launch_id.clone(),
            handle,
        },
    );

    for (account, status_code, code) in [
        ("delta", StatusCode::BAD_REQUEST, "codex-account-unknown"),
        ("--format", StatusCode::BAD_REQUEST, "invalid-codex-account"),
    ] {
        let (status, body) = call(
            router(st.clone()),
            put_json(
                "/sessions/account-switch-unknown/account",
                Some(TOKEN),
                json!({ "account": account, "expected_session_incarnation": launch_id }),
            ),
        )
        .await;
        assert_eq!(status, status_code, "body={body}");
        assert_eq!(body["error"]["code"], code);
    }
    assert!(matches!(
        commands.try_recv(),
        Err(tokio::sync::mpsc::error::TryRecvError::Empty)
    ));
    let persisted = load_session_record(&st.context, &record.id).unwrap();
    assert!(!crate::codex_account::binding_is_present(&persisted));
}

#[tokio::test]
async fn codex_account_switch_serializes_full_id_and_prefix_as_one_transaction() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().unwrap();
    let _broker = codex_listing_broker(&lock, tmp.path(), &["acct1", "omega"]);
    let launch_id = seed_codex_app_server_session(tmp.path(), "account-switch-serialized");
    let st = state(tmp.path(), Some(TOKEN), minimal_tmux(tmp.path()));
    let record = load_session_record(&st.context, "account-switch-serialized").unwrap();
    crate::activity::activate_runtime(&st.context, &record).unwrap();
    for (event_id, kind) in [
        ("turn-start", "turn_started"),
        ("turn-done", "turn_completed"),
    ] {
        let event = serde_json::from_value(json!({
            "schema_version": crate::activity::TURN_EVENT_VERSION,
            "event_id": event_id,
            "runtime_id": launch_id,
            "provider": "codex",
            "provider_turn_id": "turn-account-switch-serialized",
            "kind": kind,
            "confidence": "authoritative"
        }))
        .unwrap();
        crate::activity::ingest_event(&st.context, &record.id, event).unwrap();
    }
    let (handle, mut commands) = codex_app_server::control_channel();
    st.codex_controls.lock().unwrap().insert(
        record.id.clone(),
        CodexControlEntry {
            launch_id: launch_id.clone(),
            handle,
        },
    );

    let first_launch_id = launch_id.clone();
    let first = tokio::spawn(call(
        router(st.clone()),
        put_json(
            "/sessions/account-switch-serialized/account",
            Some(TOKEN),
            json!({
                "account": "acct1",
                "expected_session_incarnation": first_launch_id
            }),
        ),
    ));
    let Some(codex_app_server::ControlCommand::BindAccount {
        account: first_account,
        revision: first_revision,
        response: first_response,
    }) = tokio::time::timeout(Duration::from_secs(1), commands.recv())
        .await
        .expect("first switch must reach the control plane")
    else {
        panic!("first switch sent an unexpected control command");
    };
    assert_eq!(first_account, "acct1");
    assert_eq!(first_revision, 1);

    let second_launch_id = launch_id.clone();
    let second = tokio::spawn(call(
        router(st.clone()),
        put_json(
            "/sessions/account-switch-ser/account",
            Some(TOKEN),
            json!({
                "account": "omega",
                "expected_session_incarnation": second_launch_id
            }),
        ),
    ));
    assert!(
        tokio::time::timeout(Duration::from_millis(100), commands.recv())
            .await
            .is_err(),
        "the second same-session switch must not enter the control plane before the first finishes"
    );
    first_response
        .send(Ok(crate::codex_account::CodexAccountView {
            schema_version: crate::codex_account::VIEW_SCHEMA_VERSION,
            supported: true,
            applicable: true,
            state: "bound",
            selected_account: Some(first_account.clone()),
            effective_account: Some(first_account),
            selection_source: Some("explicit".to_string()),
            revision: first_revision,
            applied_runtime_id: Some(launch_id.clone()),
            failure_reason: None,
            next: None,
        }))
        .unwrap();
    assert_eq!(first.await.unwrap().0, StatusCode::OK);
    let Some(codex_app_server::ControlCommand::BindAccount {
        account: second_account,
        revision: second_revision,
        response: second_response,
    }) = tokio::time::timeout(Duration::from_secs(1), commands.recv())
        .await
        .expect("second switch must reach the control plane after the first finishes")
    else {
        panic!("second switch sent an unexpected control command");
    };
    assert_eq!(second_account, "omega");
    assert_eq!(second_revision, 2);
    second_response
        .send(Ok(crate::codex_account::CodexAccountView {
            schema_version: crate::codex_account::VIEW_SCHEMA_VERSION,
            supported: true,
            applicable: true,
            state: "bound",
            selected_account: Some(second_account.clone()),
            effective_account: Some(second_account),
            selection_source: Some("explicit".to_string()),
            revision: second_revision,
            applied_runtime_id: Some(launch_id),
            failure_reason: None,
            next: None,
        }))
        .unwrap();
    assert_eq!(second.await.unwrap().0, StatusCode::OK);
}

#[tokio::test]
async fn create_binding_wait_rejects_a_superseding_account_revision() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().unwrap();
    let _broker = EnvGuard::set(
        &lock,
        "AGENT_SESSION_CODEX_ACCOUNT_BROKER",
        r#"["/bin/false"]"#,
    );
    let launch_id = seed_codex_app_server_session(tmp.path(), "create-binding-race");
    let st = state(tmp.path(), Some(TOKEN), minimal_tmux(tmp.path()));
    let mut record = load_session_record(&st.context, "create-binding-race").unwrap();
    crate::codex_account::set_initial_binding(&mut record, Some("omega")).unwrap();
    crate::write_session_record(&st.context, &record).unwrap();
    crate::codex_account::finish_binding(&st.context, &record.id, &launch_id, "omega", 1, Ok(()))
        .unwrap();

    let response = wait_for_account_binding(&st, &record.id, &launch_id, "acct1", 1)
        .await
        .unwrap_err();
    assert_eq!(response.status(), StatusCode::CONFLICT);

    let (handle, mut commands) = codex_app_server::control_channel();
    let response = submit_structured_prompt_locked(
        &st,
        &record.id,
        &launch_id,
        StructuredPromptGuards {
            expected_binding: Some(("acct1", 1)),
            ..Default::default()
        },
        &handle,
        "must not reach superseding account",
    )
    .await
    .unwrap_err();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert!(matches!(
        commands.try_recv(),
        Err(tokio::sync::mpsc::error::TryRecvError::Empty)
    ));
}

#[tokio::test]
async fn structured_prompt_cancels_armed_auto_resume_before_provider_control() {
    let tmp = tempfile::TempDir::new().unwrap();
    let launch_id = seed_codex_app_server_session(tmp.path(), "structured-armed");
    let st = state(tmp.path(), Some(TOKEN), empty_composer_tmux(tmp.path()));
    let record = load_session_record(&st.context, "structured-armed").unwrap();
    crate::activity::activate_runtime(&st.context, &record).unwrap();
    auto_resume::set_enabled(&st.context, &record.id, true, "2030-01-01T00:00:00Z").unwrap();
    crate::activity::ingest_codex_app_server_failure(
        &st.context,
        &record.id,
        &launch_id,
        "thread-armed",
        "turn-armed",
    )
    .unwrap();
    assert_eq!(
        auto_resume::view_for_record(&st.context, &record).state,
        "armed"
    );

    let (handle, mut commands) = codex_app_server::control_channel();
    st.codex_controls
        .lock()
        .unwrap()
        .insert(record.id.clone(), CodexControlEntry { launch_id, handle });
    let responder_context = st.context.clone();
    let responder_record = record.clone();
    let responder = tokio::spawn(async move {
        let Some(codex_app_server::ControlCommand::Prompt { response, .. }) = commands.recv().await
        else {
            panic!("structured prompt did not reach the Codex control plane");
        };
        let view = auto_resume::view_for_record(&responder_context, &responder_record);
        assert!(!view.enabled);
        assert_eq!(view.state, "cancelled");
        assert_eq!(view.failure_reason.as_deref(), Some("manual_input"));
        response
            .send(Ok("turn-armed-cancelled".to_string()))
            .unwrap();
    });

    let (status, body) = call(
        router(st),
        post_json(
            "/sessions/structured-armed/prompt",
            Some(TOKEN),
            json!({ "text": "new\nmanual task" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["data"]["submitted"], true);
    assert_eq!(
        body["data"]["session_incarnation"],
        "launch-structured-armed"
    );
    responder.await.unwrap();
}

#[test]
fn account_switch_cancels_and_versions_codex_auto_resume_evidence() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().unwrap();
    let _broker = EnvGuard::set(
        &lock,
        "AGENT_SESSION_CODEX_ACCOUNT_BROKER",
        r#"["/bin/false"]"#,
    );
    let launch_id = seed_codex_app_server_session(tmp.path(), "account-auto-resume");
    let context = CliContext {
        state_dir: tmp.path().to_path_buf(),
        host: None,
    };
    let mut record = load_session_record(&context, "account-auto-resume").unwrap();
    crate::codex_account::set_initial_binding(&mut record, Some("acct1")).unwrap();
    crate::write_session_record(&context, &record).unwrap();
    crate::codex_account::finish_binding(&context, &record.id, &launch_id, "acct1", 1, Ok(()))
        .unwrap();
    record = load_session_record(&context, &record.id).unwrap();
    crate::activity::activate_runtime(&context, &record).unwrap();
    auto_resume::set_enabled(&context, &record.id, true, "2030-01-01T00:00:00Z").unwrap();
    crate::activity::ingest_codex_app_server_failure(
        &context, &record.id, &launch_id, "thread-a", "turn-a",
    )
    .unwrap();
    let account_a = crate::codex_account::binding_snapshot(&record);

    crate::codex_account::begin_switch_binding(&context, &record.id, &launch_id, "omega").unwrap();
    let mut submissions = 0;
    let outcome = auto_resume::tick_for_runtime_and_binding(
        &context,
        &record.id,
        &launch_id,
        auto_resume::RuntimeBindingTick {
            binding: &account_a,
            failover_account: None,
        },
        1_893_456_000,
        &UsageSnapshot {
            authoritative: true,
            has_exhausted_windows: false,
            exhausted_reset_epochs: Vec::new(),
            soonest_reset_epoch: None,
        },
        |_| {
            submissions += 1;
            Ok(())
        },
    )
    .unwrap();

    assert_eq!(outcome, auto_resume::TickOutcome::Unchanged);
    assert_eq!(submissions, 0);
    let view = auto_resume::view_for_record(&context, &record);
    assert_eq!(view.state, "cancelled");
    assert_eq!(view.failure_reason.as_deref(), Some("account_switch"));
}

#[tokio::test]
async fn structured_prompt_rejects_unsupported_sessions_before_terminal_input() {
    let tmp = tempfile::TempDir::new().unwrap();
    seed_session(tmp.path(), "claude", "claude", "hs-claude-structured");
    let st = state(
        tmp.path(),
        Some(TOKEN),
        PathBuf::from("/terminal-transport-must-not-run"),
    );

    let (status, body) = call(
        router(st),
        post_json(
            "/sessions/claude/prompt",
            Some(TOKEN),
            json!({ "text": "first\nsecond" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "body={body}");
    assert_eq!(body["error"]["code"], "structured-prompt-unsupported");
}

#[tokio::test]
async fn structured_prompt_reports_stale_fence_before_unsupported_runtime() {
    let tmp = tempfile::TempDir::new().unwrap();
    seed_session_with_runtime(
        tmp.path(),
        "unsupported-replacement",
        "claude",
        "hs-claude-unsupported-replacement",
    );
    let st = state(tmp.path(), Some(TOKEN), minimal_tmux(tmp.path()));

    let (status, body) = call(
        router(st),
        post_json(
            "/sessions/unsupported-replacement/prompt/v2",
            Some(TOKEN),
            json!({
                "text": "must not be classified as readiness",
                "expected_session_incarnation": "launch-unsupported-previous",
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "body={body}");
    assert_eq!(body["error"]["code"], "session-incarnation-conflict");
    assert_eq!(
        body["error"]["details"]["actual_session_incarnation"],
        "launch-unsupported-replacement"
    );
}

#[tokio::test]
async fn structured_prompt_rejects_malformed_expected_incarnation() {
    let tmp = tempfile::TempDir::new().unwrap();
    let st = state(tmp.path(), Some(TOKEN), minimal_tmux(tmp.path()));

    for expected in ["".to_string(), "   ".to_string(), "x".repeat(129)] {
        let (status, body) = call(
            router(st.clone()),
            post_json(
                "/sessions/missing/prompt",
                Some(TOKEN),
                json!({
                    "text": "inspect the current session",
                    "expected_session_incarnation": expected,
                }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "body={body}");
        assert_eq!(body["error"]["code"], "invalid-session-incarnation");
    }
}

#[tokio::test]
async fn structured_prompt_rejects_terminal_controls() {
    let tmp = tempfile::TempDir::new().unwrap();
    let launch_id = seed_codex_app_server_session(tmp.path(), "controls");
    let st = state(tmp.path(), Some(TOKEN), minimal_tmux(tmp.path()));
    let (handle, _commands) = codex_app_server::control_channel();
    st.codex_controls.lock().unwrap().insert(
        "controls".to_string(),
        CodexControlEntry { launch_id, handle },
    );

    for text in ["carriage\rreturn", "tab\tkey", "escape\u{1b}sequence"] {
        let (status, body) = call(
            router(st.clone()),
            post_json(
                "/sessions/controls/prompt",
                Some(TOKEN),
                json!({ "text": text }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "body={body}");
        assert_eq!(body["error"]["code"], "unsafe-prompt-control");
    }
}

#[tokio::test]
async fn resume_recreates_missing_runtime_with_token() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().unwrap();
    let cwd = tmp.path().join("repo");
    std::fs::create_dir_all(&cwd).unwrap();
    let log = tmp.path().join("tmux.log");
    let tmux = resume_tmux(tmp.path(), &log);
    let codex = fake_agent(tmp.path(), "codex");
    let _codex_bin = EnvGuard::set(&lock, "AGENT_SESSION_CODEX_BIN", codex.to_str().unwrap());
    seed_resumable_session(
        tmp.path(),
        "recover",
        "codex",
        "hs-codex-recover",
        &cwd,
        &[
            "resume",
            "resume-session-id",
            "--cd",
            cwd.to_str().unwrap(),
            "--no-alt-screen",
        ],
    );
    let st = state(tmp.path(), Some(TOKEN), tmux);

    let (status, body) = call(
        router(st.clone()),
        post_json("/sessions/recover/resume", None, json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"]["code"], "unauthorized");

    let (status, body) = call(
        router(st.clone()),
        post_json("/sessions/recover/resume", Some(TOKEN), json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let session = &body["data"]["session"];
    assert_eq!(session["id"], "recover");
    assert_eq!(session["status"], "running");
    assert_eq!(session["resumable"], true);

    let calls = std::fs::read_to_string(&log).unwrap();
    assert!(
        calls.contains("-s hs-codex-recover"),
        "resume must create a tmux runtime: {calls:?}"
    );
    assert!(
        calls.contains("resume resume-session-id"),
        "resume must use the exact provider id: {calls:?}"
    );
}

#[tokio::test]
async fn resume_rejects_a_profile_removed_from_the_server_registry() {
    let tmp = tempfile::TempDir::new().unwrap();
    let cwd = tmp.path().join("repo");
    fs::create_dir_all(&cwd).unwrap();
    let log = tmp.path().join("tmux.log");
    let tmux = resume_tmux(tmp.path(), &log);
    let launcher = fake_agent(tmp.path(), "claude-gpt");
    seed_resumable_session(
        tmp.path(),
        "revoked-profile",
        "claude",
        "hs-claude-revoked-profile",
        &cwd,
        &["--resume", "resume-session-id"],
    );
    let record_path = tmp.path().join("sessions/revoked-profile/session.json");
    let mut record: Value = serde_json::from_slice(&fs::read(&record_path).unwrap()).unwrap();
    record["agent_bin"] = json!(launcher);
    record["runtime"]["agent_profile"] = json!("claude-gpt");
    fs::write(&record_path, serde_json::to_vec_pretty(&record).unwrap()).unwrap();

    let st = state(tmp.path(), Some(TOKEN), tmux);
    let (list_status, list_body) = call(router(st.clone()), get("/sessions")).await;
    assert_eq!(list_status, StatusCode::OK, "body={list_body}");
    assert_eq!(list_body["data"]["sessions"][0]["resumable"], false);
    assert_eq!(
        list_body["data"]["sessions"][0]["resume_blocked_reason"],
        "agent-profile-unavailable"
    );

    let (status, body) = call(
        router(st),
        post_json("/sessions/revoked-profile/resume", Some(TOKEN), json!({})),
    )
    .await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body={body}");
    assert_eq!(body["error"]["code"], "agent-profile-unavailable");
    let calls = fs::read_to_string(log).unwrap_or_default();
    assert!(
        !calls.contains("new-session"),
        "body={body}, calls={calls:?}"
    );
}

#[tokio::test]
async fn resume_rejects_stale_profile_auto_resume_capability() {
    let tmp = tempfile::TempDir::new().unwrap();
    let cwd = tmp.path().join("repo");
    let config_dir = tmp.path().join("claude-gpt-config");
    fs::create_dir_all(&cwd).unwrap();
    fs::create_dir_all(&config_dir).unwrap();
    let log = tmp.path().join("tmux.log");
    let tmux = resume_tmux(tmp.path(), &log);
    let launcher = fake_agent(tmp.path(), "claude-gpt");
    seed_resumable_session(
        tmp.path(),
        "stale-profile-capability",
        "claude",
        "hs-claude-stale-profile-capability",
        &cwd,
        &["--resume", "resume-session-id"],
    );
    let record_path = tmp
        .path()
        .join("sessions/stale-profile-capability/session.json");
    let mut record: Value = serde_json::from_slice(&fs::read(&record_path).unwrap()).unwrap();
    record["agent_bin"] = json!(launcher);
    record["runtime"]["agent_profile"] = json!("claude-gpt");
    record["runtime"]["agent_profile_provider_config_dir"] = json!(config_dir);
    record["runtime"]["agent_profile_auto_resume_supported"] = json!(true);
    fs::write(&record_path, serde_json::to_vec_pretty(&record).unwrap()).unwrap();
    let profiles = AgentLaunchProfiles::from_json(
        &json!([{
            "id":"claude-gpt",
            "label":"Claude GPT",
            "agent":"claude",
            "agent_bin":launcher,
            "provider_config_dir":config_dir,
            "auto_resume_supported":false,
        }])
        .to_string(),
    )
    .unwrap();
    let mut st = state(tmp.path(), Some(TOKEN), tmux);
    Arc::get_mut(&mut st).unwrap().launch_profiles = profiles;

    let (status, body) = call(
        router(st),
        post_json(
            "/sessions/stale-profile-capability/resume",
            Some(TOKEN),
            json!({}),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body={body}");
    assert_eq!(body["error"]["code"], "agent-profile-context-mismatch");
    let calls = fs::read_to_string(log).unwrap_or_default();
    assert!(
        !calls.contains("new-session"),
        "body={body}, calls={calls:?}"
    );
}

#[tokio::test]
async fn resume_rejects_profile_provider_root_drift() {
    let tmp = tempfile::TempDir::new().unwrap();
    let cwd = tmp.path().join("repo");
    let durable_config_dir = tmp.path().join("profile-root-before");
    let drifted_config_dir = tmp.path().join("profile-root-after");
    fs::create_dir_all(&cwd).unwrap();
    fs::create_dir_all(&durable_config_dir).unwrap();
    fs::create_dir_all(&drifted_config_dir).unwrap();
    let log = tmp.path().join("tmux.log");
    let tmux = resume_tmux(tmp.path(), &log);
    let launcher = fake_agent(tmp.path(), "claude-gpt");
    seed_resumable_session(
        tmp.path(),
        "profile-root-drift",
        "claude",
        "hs-claude-profile-root-drift",
        &cwd,
        &["--resume", "resume-session-id"],
    );
    let record_path = tmp.path().join("sessions/profile-root-drift/session.json");
    let mut record: Value = serde_json::from_slice(&fs::read(&record_path).unwrap()).unwrap();
    record["agent_bin"] = json!(launcher);
    record["runtime"]["agent_profile"] = json!("claude-gpt");
    record["runtime"]["agent_profile_provider_config_dir"] = json!(durable_config_dir);
    record["runtime"]["agent_profile_auto_resume_supported"] = json!(false);
    fs::write(&record_path, serde_json::to_vec_pretty(&record).unwrap()).unwrap();
    let profiles = AgentLaunchProfiles::from_json(
        &json!([{
            "id":"claude-gpt",
            "label":"Claude GPT",
            "agent":"claude",
            "agent_bin":launcher,
            "provider_config_dir":drifted_config_dir,
            "auto_resume_supported":false,
        }])
        .to_string(),
    )
    .unwrap();
    let mut st = state(tmp.path(), Some(TOKEN), tmux);
    Arc::get_mut(&mut st).unwrap().launch_profiles = profiles;

    let (status, body) = call(
        router(st),
        post_json(
            "/sessions/profile-root-drift/resume",
            Some(TOKEN),
            json!({}),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body={body}");
    assert_eq!(body["error"]["code"], "agent-profile-context-mismatch");
    let calls = fs::read_to_string(log).unwrap_or_default();
    assert!(
        !calls.contains("new-session"),
        "provider-root drift must fail before launch: {calls:?}"
    );
}

#[tokio::test]
async fn resume_rechecks_profile_readiness_and_fails_before_launch() {
    let tmp = tempfile::TempDir::new().unwrap();
    let cwd = tmp.path().join("repo");
    let config_dir = tmp.path().join("claude-gpt-config");
    fs::create_dir_all(&cwd).unwrap();
    fs::create_dir_all(&config_dir).unwrap();
    let log = tmp.path().join("tmux.log");
    let tmux = resume_tmux(tmp.path(), &log);
    let launcher = executable(
        &tmp.path().join("claude-gpt"),
        "#!/usr/bin/env sh\nexit 23\n",
    );
    seed_resumable_session(
        tmp.path(),
        "profile-readiness-failed",
        "claude",
        "hs-claude-profile-readiness-failed",
        &cwd,
        &["--resume", "resume-session-id"],
    );
    let record_path = tmp
        .path()
        .join("sessions/profile-readiness-failed/session.json");
    let mut record: Value = serde_json::from_slice(&fs::read(&record_path).unwrap()).unwrap();
    record["agent_bin"] = json!(launcher);
    record["runtime"]["agent_profile"] = json!("claude-gpt");
    record["runtime"]["agent_profile_provider_config_dir"] = json!(config_dir);
    record["runtime"]["agent_profile_auto_resume_supported"] = json!(false);
    fs::write(&record_path, serde_json::to_vec_pretty(&record).unwrap()).unwrap();
    let profiles = AgentLaunchProfiles::from_json(
        &json!([{
            "id":"claude-gpt",
            "label":"Claude GPT",
            "agent":"claude",
            "agent_bin":launcher,
            "provider_config_dir":config_dir,
            "readiness_args":["--check"],
            "auto_resume_supported":false,
        }])
        .to_string(),
    )
    .unwrap();
    let mut st = state(tmp.path(), Some(TOKEN), tmux);
    Arc::get_mut(&mut st).unwrap().launch_profiles = profiles;

    let (status, body) = call(
        router(st),
        post_json(
            "/sessions/profile-readiness-failed/resume",
            Some(TOKEN),
            json!({}),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body={body}");
    assert_eq!(body["error"]["code"], "agent-profile-unavailable");
    let calls = fs::read_to_string(log).unwrap_or_default();
    assert!(
        !calls.contains("new-session"),
        "failed readiness must stop before tmux launch: {calls:?}"
    );
}

#[tokio::test]
async fn list_fails_closed_immediately_after_a_profile_root_is_removed() {
    let tmp = tempfile::TempDir::new().unwrap();
    let cwd = tmp.path().join("repo");
    let config_dir = tmp.path().join("claude-gpt-config");
    fs::create_dir_all(&cwd).unwrap();
    fs::create_dir_all(&config_dir).unwrap();
    let log = tmp.path().join("tmux.log");
    let tmux = resume_tmux(tmp.path(), &log);
    let launcher = fake_agent(tmp.path(), "claude-gpt");
    seed_resumable_session(
        tmp.path(),
        "profile-root-removed",
        "claude",
        "hs-claude-profile-root-removed",
        &cwd,
        &["--resume", "resume-session-id"],
    );
    let record_path = tmp
        .path()
        .join("sessions/profile-root-removed/session.json");
    let mut record: Value = serde_json::from_slice(&fs::read(&record_path).unwrap()).unwrap();
    record["agent_bin"] = json!(launcher);
    record["runtime"]["agent_profile"] = json!("claude-gpt");
    record["runtime"]["agent_profile_provider_config_dir"] = json!(config_dir);
    record["runtime"]["agent_profile_auto_resume_supported"] = json!(false);
    fs::write(&record_path, serde_json::to_vec_pretty(&record).unwrap()).unwrap();
    let profiles = AgentLaunchProfiles::from_json(
        &json!([{
            "id":"claude-gpt",
            "label":"Claude GPT",
            "agent":"claude",
            "agent_bin":launcher,
            "provider_config_dir":config_dir,
            "auto_resume_supported":false,
        }])
        .to_string(),
    )
    .unwrap();
    let mut st = state(tmp.path(), Some(TOKEN), tmux);
    Arc::get_mut(&mut st).unwrap().launch_profiles = profiles;

    let (ready_status, ready_body) = call(router(st.clone()), get("/sessions")).await;
    assert_eq!(ready_status, StatusCode::OK, "body={ready_body}");
    assert_eq!(
        ready_body["data"]["agent_profiles"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(ready_body["data"]["sessions"][0]["resumable"], true);

    fs::remove_dir(&config_dir).unwrap();
    let (removed_status, removed_body) = call(router(st), get("/sessions")).await;
    assert_eq!(removed_status, StatusCode::OK, "body={removed_body}");
    assert!(
        removed_body["data"]["agent_profiles"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(removed_body["data"]["sessions"][0]["resumable"], false);
    assert_eq!(
        removed_body["data"]["sessions"][0]["resume_blocked_reason"],
        "agent-profile-unavailable"
    );
}

#[tokio::test]
async fn resume_profile_pins_durable_provider_root_and_launcher() {
    let tmp = tempfile::TempDir::new().unwrap();
    let cwd = tmp.path().join("repo");
    let config_dir = tmp.path().join("profile-claude");
    fs::create_dir_all(&cwd).unwrap();
    fs::create_dir_all(&config_dir).unwrap();
    let log = tmp.path().join("tmux.log");
    let tmux = resume_tmux(tmp.path(), &log);
    let launcher = fake_agent(tmp.path(), "profile-claude-bin");
    seed_resumable_session(
        tmp.path(),
        "profile-resume",
        "claude",
        "hs-claude-profile-resume",
        &cwd,
        &["--resume", "resume-session-id"],
    );
    let record_path = tmp.path().join("sessions/profile-resume/session.json");
    let mut record: Value = serde_json::from_slice(&fs::read(&record_path).unwrap()).unwrap();
    record["agent_bin"] = json!(launcher);
    record["runtime"]["agent_profile"] = json!("profile-claude");
    record["runtime"]["agent_profile_provider_config_dir"] = json!(config_dir);
    record["runtime"]["agent_profile_auto_resume_supported"] = json!(false);
    fs::write(&record_path, serde_json::to_vec_pretty(&record).unwrap()).unwrap();
    let profiles = AgentLaunchProfiles::from_json(
        &json!([{
            "id":"profile-claude",
            "label":"Profile Claude",
            "agent":"claude",
            "agent_bin":launcher,
            "provider_config_dir":config_dir,
            "auto_resume_supported":false,
        }])
        .to_string(),
    )
    .unwrap();
    let mut st = state(tmp.path(), Some(TOKEN), tmux);
    Arc::get_mut(&mut st).unwrap().launch_profiles = profiles;

    let (status, body) = call(
        router(st),
        post_json("/sessions/profile-resume/resume", Some(TOKEN), json!({})),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["data"]["session"]["auto_resume"]["supported"], false);
    let calls = fs::read_to_string(log).unwrap();
    assert!(
        calls.contains(&format!("CLAUDE_CONFIG_DIR={}", config_dir.display())),
        "managed resume must pin the durable provider root: {calls:?}"
    );
    assert!(
        calls.contains(launcher.to_string_lossy().as_ref()),
        "managed resume must use the durable launcher: {calls:?}"
    );
    assert_serve_lifecycle(tmp.path(), "profile-resume", &[("resume", 1)]);
}

#[tokio::test]
async fn resume_api_rejects_unprovable_pre_upgrade_codex_and_claude_runtimes() {
    let tmp = tempfile::TempDir::new().unwrap();
    let cwd = tmp.path().join("repo");
    std::fs::create_dir_all(&cwd).unwrap();
    let log = tmp.path().join("tmux.log");
    let tmux = resume_tmux(tmp.path(), &log);

    for agent in ["codex", "claude"] {
        for runtime_shape in ["identity-free", "omitted"] {
            let id = format!("unprovable-{agent}-{runtime_shape}");
            let tmux_session = format!("hs-{agent}-unprovable-{runtime_shape}");
            let resume_args = match agent {
                "codex" => vec![
                    "resume",
                    "resume-session-id",
                    "--cd",
                    cwd.to_str().unwrap(),
                    "--no-alt-screen",
                ],
                "claude" => vec!["--resume", "resume-session-id"],
                _ => unreachable!(),
            };
            seed_resumable_session(tmp.path(), &id, agent, &tmux_session, &cwd, &resume_args);
            let record_path = tmp.path().join("sessions").join(&id).join("session.json");
            let mut record: Value =
                serde_json::from_slice(&std::fs::read(&record_path).unwrap()).unwrap();
            let record = record.as_object_mut().unwrap();
            record.remove("startup");
            record.remove("tmux_runtime_never_launched");
            if runtime_shape == "omitted" {
                record.remove("runtime");
            }
            std::fs::write(&record_path, serde_json::to_vec_pretty(&record).unwrap()).unwrap();
            let before = std::fs::read(&record_path).unwrap();
            let st = state(tmp.path(), Some(TOKEN), tmux.clone());

            let (status, body) = call(
                router(st),
                post_json(&format!("/sessions/{id}/resume"), Some(TOKEN), json!({})),
            )
            .await;

            assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body={body}");
            assert_eq!(body["schema_version"], "cli.agent-session.serve.v1");
            assert_eq!(body["ok"], false);
            assert_eq!(body["error"]["code"], "session-termination-failed");
            assert_eq!(
                body["error"]["details"]["reason"],
                "runtime-identity-unavailable"
            );
            assert_eq!(body["error"]["details"]["retryable"], false);
            assert_eq!(
                body["error"]["details"]["action"],
                "manual-runtime-verification-required"
            );
            assert_eq!(std::fs::read(&record_path).unwrap(), before);
        }
    }

    let calls = std::fs::read_to_string(&log).unwrap();
    assert!(
        !calls.contains("new-session"),
        "serve resume must not replace unprovable runtimes: {calls:?}"
    );
}

#[tokio::test]
async fn retired_external_runtime_maintenance_and_delete_preserve_durable_state() {
    use pretty_assertions::assert_eq;
    let tmp = tempfile::TempDir::new().unwrap();
    let context = CliContext {
        state_dir: tmp.path().to_path_buf(),
        host: None,
    };
    let (record, files) =
        crate::tests::retired_external_record_fixture(&context, "retained-external");
    let tmux = executable(
        &tmp.path().join("absent-retired-tmux"),
        "#!/bin/sh\nprintf '%s\\n' \"can't find session: absent\" >&2\nexit 1\n",
    );
    let _probe =
        crate::tmux_probe_fixture::install(&tmux, crate::tmux_probe_fixture::Probe::Stopped);
    let st = state(tmp.path(), Some(TOKEN), tmux);
    for (operation, actions) in [
        (
            "delete",
            vec![
                "remove_console_record",
                "retry_delete",
                "terminate_runtime_then_delete",
            ],
        ),
        ("resume", vec!["terminate_runtime_then_resume"]),
    ] {
        let (status, body) = call(router(st.clone()), get_auth(
                &format!("/sessions/{}/maintenance?operation={operation}&schema_version=agent-session.session-maintenance.v2", record.id), Some(TOKEN),
            )).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let preview = &body["data"]["maintenance"];
        assert_eq!(preview["state"], "blocked");
        assert!(
            preview["actions"]
                .as_array()
                .unwrap()
                .iter()
                .all(|action| action["destructive"] == false)
        );
        for action in actions {
            let (status, body) = call(router(st.clone()), post_json(
                    &format!("/sessions/{}/maintenance/actions", record.id), Some(TOKEN),
                    json!({
                        "schema_version": "agent-session.session-maintenance.v2", "operation": operation,
                        "action": action, "expected_session_incarnation": preview["session_incarnation"],
                        "expected_session_generation": preview["session_generation"],
                        "expected_preview_digest": preview["preview_digest"], "confirmed": true
                    }),
                )).await;
            assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{body}");
            assert_eq!(body["error"]["code"], "session-maintenance-failed");
            assert_eq!(
                body["error"]["details"]["kind"],
                "runtime_identity_unavailable"
            );
            assert_eq!(body["error"]["details"]["session_metadata_retained"], true);
        }
    }
    let request = Request::builder()
        .method("DELETE")
        .uri(format!("/sessions/{}", record.id))
        .header("authorization", format!("Bearer {TOKEN}"))
        .body(Body::empty())
        .unwrap();
    let (status, body) = call(router(st.clone()), request).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{body}");
    assert_eq!(body["error"]["code"], "session-termination-failed");
    let (status, body) = call(
        router(st),
        post_json(
            &format!("/sessions/{}/archive", record.id),
            Some(TOKEN),
            json!({ "expected_session_incarnation": record.runtime.as_ref().unwrap().launch_id }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{body}");
    assert_eq!(body["error"]["code"], "session-termination-failed");
    for (path, bytes) in files {
        assert_eq!(fs::read(path).unwrap(), bytes);
    }
    assert!(crate::board::closed_reasons_for_test(&context).is_empty());
    assert!(!provider_history::archive_root(&context.state_dir).exists());
}

#[tokio::test]
async fn maintenance_preview_is_authenticated_and_projects_safe_runtime_facts() {
    let tmp = tempfile::TempDir::new().unwrap();
    let cwd = tmp.path().join("repo");
    std::fs::create_dir_all(&cwd).unwrap();
    let id = "maintenance-preview";
    seed_resumable_session(
        tmp.path(),
        id,
        "codex",
        "hs-codex-maintenance-preview",
        &cwd,
        &[
            "resume",
            "resume-session-id",
            "--cd",
            cwd.to_str().unwrap(),
            "--no-alt-screen",
        ],
    );
    let record_path = tmp.path().join("sessions").join(id).join("session.json");
    let mut record: Value = serde_json::from_slice(&std::fs::read(&record_path).unwrap()).unwrap();
    record["runtime"]["launch_id"] = json!("maintenance-preview-launch");
    let pane = TestProcessGroup::spawn();
    record["delete_tmux_identity"] = json!({
        "launch_id": "maintenance-preview-launch",
        "session_id": "$98",
        "pane_id": "%98",
        "pane_pid": pane.pid(),
        "process_group_id": pane.pid(),
    });
    std::fs::write(&record_path, serde_json::to_vec_pretty(&record).unwrap()).unwrap();
    let tmux = executable(
        &tmp.path().join("stopped-maintenance-tmux"),
        "#!/usr/bin/env sh\nprintf '%s\\n' \"can't find session: stopped\" >&2\nexit 1\n",
    );
    let st = state(tmp.path(), Some(TOKEN), tmux);

    let (status, body) = call(
        router(st.clone()),
        get_auth(
            &format!("/sessions/{id}/maintenance?operation=resume"),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "body={body}");

    let (status, body) = call(
        router(st),
        get_auth(
            &format!("/sessions/{id}/maintenance?operation=resume"),
            Some(TOKEN),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let maintenance = &body["data"]["maintenance"];
    assert_eq!(
        maintenance["schema_version"],
        "agent-session.session-maintenance.v1"
    );
    assert_eq!(maintenance["state"], "blocked");
    assert_eq!(
        maintenance["session_incarnation"],
        "maintenance-preview-launch"
    );
    assert_eq!(maintenance["session_generation"], 1);
    assert_eq!(maintenance["issue"]["kind"], "process_boundary_live");
    assert_eq!(maintenance["issue"]["operation"], "resume");
    assert_eq!(maintenance["boundary"]["kind"], "process_group");
    assert!(
        maintenance["boundary"]["safe_process_count"]
            .as_u64()
            .unwrap()
            >= 1
    );
    assert_eq!(maintenance["actions"][0]["id"], "retry_resume");
    assert_eq!(maintenance["actions"][0]["destructive"], false);
    let digest = maintenance["preview_digest"].as_str().unwrap();
    assert!(digest.starts_with("sha256:"));
    assert_eq!(digest.len(), 71);

    let serialized = maintenance.to_string();
    for forbidden in [
        tmp.path().to_str().unwrap(),
        "tmux_session",
        "process_group_id",
        "control_group",
        "argv",
        "environment",
        "transcript",
    ] {
        assert!(
            !serialized.contains(forbidden),
            "maintenance preview leaked {forbidden}: {serialized}"
        );
    }
}

#[tokio::test]
async fn maintenance_preview_lock_admission_is_bounded_and_session_scoped() {
    let tmp = tempfile::TempDir::new().unwrap();
    let cwd = tmp.path().join("repo");
    std::fs::create_dir_all(&cwd).unwrap();
    for id in ["maintenance-busy", "maintenance-independent"] {
        seed_resumable_session(
            tmp.path(),
            id,
            "codex",
            &format!("hs-codex-{id}"),
            &cwd,
            &[
                "resume",
                "resume-session-id",
                "--cd",
                cwd.to_str().unwrap(),
                "--no-alt-screen",
            ],
        );
    }
    let log = tmp.path().join("tmux.log");
    let tmux = resume_tmux(tmp.path(), &log);
    let st = state(tmp.path(), Some(TOKEN), tmux);
    let _held = crate::acquire_session_record_lock(&st.context, "maintenance-busy")
        .expect("hold busy session lock");

    let started = Instant::now();
    let (busy, independent) = tokio::join!(
        call(
            router(st.clone()),
            get_auth(
                "/sessions/maintenance-busy/maintenance?operation=resume",
                Some(TOKEN),
            ),
        ),
        call(
            router(st),
            get_auth(
                "/sessions/maintenance-independent/maintenance?operation=resume",
                Some(TOKEN),
            ),
        ),
    );

    assert_eq!(busy.0, StatusCode::CONFLICT, "body={}", busy.1);
    assert_eq!(busy.1["error"]["code"], "maintenance-session-busy");
    assert_eq!(busy.1["error"]["details"]["retryable"], true);
    assert_eq!(independent.0, StatusCode::OK, "body={}", independent.1);
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "maintenance lock admission must stay bounded"
    );
}

#[tokio::test]
async fn maintenance_action_rejects_a_stale_preview_before_retrying() {
    let tmp = tempfile::TempDir::new().unwrap();
    let cwd = tmp.path().join("repo");
    std::fs::create_dir_all(&cwd).unwrap();
    let id = "maintenance-stale";
    seed_resumable_session(
        tmp.path(),
        id,
        "codex",
        "hs-codex-maintenance-stale",
        &cwd,
        &[
            "resume",
            "resume-session-id",
            "--cd",
            cwd.to_str().unwrap(),
            "--no-alt-screen",
        ],
    );
    let record_path = tmp.path().join("sessions").join(id).join("session.json");
    let mut record: Value = serde_json::from_slice(&std::fs::read(&record_path).unwrap()).unwrap();
    record["runtime"]["launch_id"] = json!("maintenance-stale-launch");
    record["tmux_runtime_never_launched"] = json!(true);
    std::fs::write(&record_path, serde_json::to_vec_pretty(&record).unwrap()).unwrap();
    let log = tmp.path().join("tmux.log");
    let tmux = resume_tmux(tmp.path(), &log);
    let st = state(tmp.path(), Some(TOKEN), tmux);

    let (status, preview) = call(
        router(st.clone()),
        get_auth(
            &format!("/sessions/{id}/maintenance?operation=resume"),
            Some(TOKEN),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={preview}");
    let maintenance = &preview["data"]["maintenance"];

    let (status, body) = call(
            router(st),
            post_json(
                &format!("/sessions/{id}/maintenance/actions"),
                Some(TOKEN),
                json!({
                    "operation": "resume",
                    "action": "retry_resume",
                    "expected_session_incarnation": maintenance["session_incarnation"],
                    "expected_session_generation": maintenance["session_generation"],
                    "expected_preview_digest": "sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"
                }),
            ),
        )
        .await;

    assert_eq!(status, StatusCode::CONFLICT, "body={body}");
    assert_eq!(body["error"]["code"], "maintenance-preview-stale");
    let calls = std::fs::read_to_string(&log).unwrap_or_default();
    assert!(
        !calls.contains("new-session"),
        "stale action must not replace the runtime: {calls:?}"
    );
}

#[tokio::test]
async fn maintenance_action_rejects_durable_generation_drift_without_mutation() {
    let tmp = tempfile::TempDir::new().unwrap();
    let cwd = tmp.path().join("repo");
    std::fs::create_dir_all(&cwd).unwrap();
    let id = "maintenance-generation-drift";
    seed_resumable_session(
        tmp.path(),
        id,
        "codex",
        "hs-codex-maintenance-generation-drift",
        &cwd,
        &[
            "resume",
            "resume-session-id",
            "--cd",
            cwd.to_str().unwrap(),
            "--no-alt-screen",
        ],
    );
    let record_path = tmp.path().join("sessions").join(id).join("session.json");
    let log = tmp.path().join("tmux.log");
    let tmux = resume_tmux(tmp.path(), &log);
    let st = state(tmp.path(), Some(TOKEN), tmux);

    let (status, preview) = call(
        router(st.clone()),
        get_auth(
            &format!("/sessions/{id}/maintenance?operation=resume"),
            Some(TOKEN),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={preview}");
    let maintenance = &preview["data"]["maintenance"];

    let mut drifted: Value = serde_json::from_slice(&std::fs::read(&record_path).unwrap()).unwrap();
    drifted["runtime"]["generation"] = json!(2);
    std::fs::write(&record_path, serde_json::to_vec_pretty(&drifted).unwrap()).unwrap();
    let after_drift = std::fs::read(&record_path).unwrap();

    let (status, body) = call(
        router(st),
        post_json(
            &format!("/sessions/{id}/maintenance/actions"),
            Some(TOKEN),
            json!({
                "operation": "resume",
                "action": "retry_resume",
                "expected_session_incarnation": maintenance["session_incarnation"],
                "expected_session_generation": maintenance["session_generation"],
                "expected_preview_digest": maintenance["preview_digest"]
            }),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::CONFLICT, "body={body}");
    assert_eq!(body["error"]["code"], "maintenance-preview-stale");
    assert_eq!(std::fs::read(&record_path).unwrap(), after_drift);
    assert!(record_path.exists());
    let calls = std::fs::read_to_string(&log).unwrap_or_default();
    assert!(
        !calls.contains("new-session"),
        "state drift must fence runtime replacement: {calls:?}"
    );
}

#[tokio::test]
async fn maintenance_resume_accepts_a_pre_runtime_record_with_paired_null_identity() {
    let tmp = tempfile::TempDir::new().unwrap();
    let id = "maintenance-pre-runtime-paired-null";
    seed_session(
        tmp.path(),
        id,
        "codex",
        "hs-codex-maintenance-pre-runtime-paired-null",
    );
    let st = state(tmp.path(), Some(TOKEN), minimal_tmux(tmp.path()));

    let (status, preview) = call(
        router(st.clone()),
        get_auth(
            &format!("/sessions/{id}/maintenance?operation=resume"),
            Some(TOKEN),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={preview}");
    let maintenance = &preview["data"]["maintenance"];

    let (status, body) = call(
        router(st),
        post_json(
            &format!("/sessions/{id}/maintenance/actions"),
            Some(TOKEN),
            json!({
                "operation": "resume",
                "action": "retry_resume",
                "expected_session_incarnation": null,
                "expected_session_generation": null,
                "expected_preview_digest": maintenance["preview_digest"]
            }),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body={body}");
    let result = &body["data"]["maintenance_result"];
    assert_eq!(result["outcome"], "resumed");
    assert_eq!(result["status"], "running");
    assert_eq!(result["session_incarnation"], Value::Null);
    assert_eq!(result["session_generation"], Value::Null);
}

#[tokio::test]
async fn maintenance_delete_by_prefix_cleans_the_canonical_runtime_registry() {
    let tmp = tempfile::TempDir::new().unwrap();
    let cwd = tmp.path().join("repo");
    std::fs::create_dir_all(&cwd).unwrap();
    let id = "maintenance-prefix-delete-canonical";
    let prefix = "maintenance-prefix-delete-can";
    seed_resumable_session(
        tmp.path(),
        id,
        "codex",
        "hs-codex-maintenance-prefix-delete-canonical",
        &cwd,
        &[
            "resume",
            "resume-session-id",
            "--cd",
            cwd.to_str().unwrap(),
            "--no-alt-screen",
        ],
    );
    let tmux = executable(
        &tmp.path().join("stopped-prefix-delete-tmux"),
        "#!/usr/bin/env sh\nprintf '%s\\n' \"can't find session: stopped\" >&2\nexit 1\n",
    );
    let st = state(tmp.path(), Some(TOKEN), tmux);
    let (handle, _commands) = codex_app_server::control_channel();
    st.codex_controls.lock().unwrap().insert(
        id.to_string(),
        CodexControlEntry {
            launch_id: "never-launched-fixture".to_string(),
            handle,
        },
    );

    let (status, preview) = call(
        router(st.clone()),
        get_auth(
            &format!("/sessions/{prefix}/maintenance?operation=delete"),
            Some(TOKEN),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={preview}");
    let maintenance = &preview["data"]["maintenance"];

    let (status, body) = call(
        router(st.clone()),
        post_json(
            &format!("/sessions/{prefix}/maintenance/actions"),
            Some(TOKEN),
            json!({
                "operation": "delete",
                "action": "retry_delete",
                "expected_session_incarnation": maintenance["session_incarnation"],
                "expected_session_generation": maintenance["session_generation"],
                "expected_preview_digest": maintenance["preview_digest"],
                "confirmed": true
            }),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["data"]["maintenance_result"]["session_id"], id);
    assert!(
        !st.codex_controls.lock().unwrap().contains_key(id),
        "prefix deletion must clean the canonical runtime registry entry"
    );
}

#[tokio::test]
async fn maintenance_inspect_reuses_the_bounded_preview_status_probe() {
    let tmp = tempfile::TempDir::new().unwrap();
    let cwd = tmp.path().join("repo");
    std::fs::create_dir_all(&cwd).unwrap();
    let id = "maintenance-inspect-one-probe";
    seed_resumable_session(
        tmp.path(),
        id,
        "codex",
        "hs-codex-maintenance-inspect-one-probe",
        &cwd,
        &[
            "resume",
            "resume-session-id",
            "--cd",
            cwd.to_str().unwrap(),
            "--no-alt-screen",
        ],
    );
    let calls = tmp.path().join("tmux-status-calls");
    let tmux = executable(
        &tmp.path().join("stopped-inspect-tmux"),
        &format!(
            "#!/usr/bin/env sh\nprintf '%s\\n' \"$*\" >> {}\nprintf '%s\\n' \"no server running on fixture\" >&2\nexit 1\n",
            shell_words::quote(&crate::display_path(&calls)),
        ),
    );
    let st = state(tmp.path(), Some(TOKEN), tmux);

    let (status, preview) = call(
        router(st.clone()),
        get_auth(
            &format!("/sessions/{id}/maintenance?operation=inspect"),
            Some(TOKEN),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={preview}");
    let maintenance = &preview["data"]["maintenance"];
    std::fs::write(&calls, b"").unwrap();

    let (status, body) = call(
        router(st),
        post_json(
            &format!("/sessions/{id}/maintenance/actions"),
            Some(TOKEN),
            json!({
                "operation": "inspect",
                "action": "inspect",
                "expected_session_incarnation": maintenance["session_incarnation"],
                "expected_session_generation": maintenance["session_generation"],
                "expected_preview_digest": maintenance["preview_digest"]
            }),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["data"]["maintenance_result"]["status"], "stopped");
    assert_eq!(
        std::fs::read_to_string(&calls).unwrap().lines().count(),
        1,
        "the action must reuse the prepared preview status instead of probing tmux twice"
    );
}

#[tokio::test]
async fn maintenance_retry_resume_uses_the_preview_and_returns_a_safe_result() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().unwrap();
    let cwd = tmp.path().join("repo");
    std::fs::create_dir_all(&cwd).unwrap();
    let id = "maintenance-retry-resume";
    seed_resumable_session(
        tmp.path(),
        id,
        "codex",
        "hs-codex-maintenance-retry-resume",
        &cwd,
        &[
            "resume",
            "resume-session-id",
            "--cd",
            cwd.to_str().unwrap(),
            "--no-alt-screen",
        ],
    );
    let log = tmp.path().join("tmux.log");
    let tmux = resume_tmux(tmp.path(), &log);
    let codex = fake_agent(tmp.path(), "codex");
    let _codex_bin = EnvGuard::set(&lock, "AGENT_SESSION_CODEX_BIN", codex.to_str().unwrap());
    let st = state(tmp.path(), Some(TOKEN), tmux);

    let (status, preview) = call(
        router(st.clone()),
        get_auth(
            &format!("/sessions/{id}/maintenance?operation=resume"),
            Some(TOKEN),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={preview}");
    let maintenance = &preview["data"]["maintenance"];

    let (status, body) = call(
        router(st),
        post_json(
            &format!("/sessions/{id}/maintenance/actions"),
            Some(TOKEN),
            json!({
                "operation": "resume",
                "action": "retry_resume",
                "expected_session_incarnation": maintenance["session_incarnation"],
                "expected_session_generation": maintenance["session_generation"],
                "expected_preview_digest": maintenance["preview_digest"]
            }),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body={body}");
    let result = &body["data"]["maintenance_result"];
    assert_eq!(
        result["schema_version"],
        "agent-session.session-maintenance.v1"
    );
    assert_eq!(result["session_id"], id);
    assert_eq!(result["operation"], "resume");
    assert_eq!(result["action"], "retry_resume");
    assert_eq!(result["outcome"], "resumed");
    assert_eq!(result["status"], "running");
    assert_eq!(result["session_generation"], 2);
    assert_eq!(result["cleanup_pending"], false);
    let serialized = result.to_string();
    for forbidden in [
        tmp.path().to_str().unwrap(),
        "tmux_session",
        "cwd",
        "resume-session-id",
        "argv",
        "environment",
        "transcript",
    ] {
        assert!(
            !serialized.contains(forbidden),
            "maintenance result leaked {forbidden}: {serialized}"
        );
    }
    assert!(
        std::fs::read_to_string(&log)
            .unwrap()
            .contains("new-session")
    );
}

#[tokio::test]
async fn maintenance_retry_resume_preserves_claude_resume_arguments() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().unwrap();
    let cwd = tmp.path().join("repo");
    std::fs::create_dir_all(&cwd).unwrap();
    let id = "maintenance-claude-retry-resume";
    seed_resumable_session(
        tmp.path(),
        id,
        "claude",
        "hs-claude-maintenance-retry-resume",
        &cwd,
        &["--resume", "resume-session-id"],
    );
    let log = tmp.path().join("tmux.log");
    let tmux = resume_tmux(tmp.path(), &log);
    let claude = fake_agent(tmp.path(), "claude");
    let _claude_bin = EnvGuard::set(&lock, "AGENT_SESSION_CLAUDE_BIN", claude.to_str().unwrap());
    let st = state(tmp.path(), Some(TOKEN), tmux);

    let (status, preview) = call(
        router(st.clone()),
        get_auth(
            &format!("/sessions/{id}/maintenance?operation=resume"),
            Some(TOKEN),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={preview}");
    let maintenance = &preview["data"]["maintenance"];

    let (status, body) = call(
        router(st),
        post_json(
            &format!("/sessions/{id}/maintenance/actions"),
            Some(TOKEN),
            json!({
                "operation": "resume",
                "action": "retry_resume",
                "expected_session_incarnation": maintenance["session_incarnation"],
                "expected_session_generation": maintenance["session_generation"],
                "expected_preview_digest": maintenance["preview_digest"]
            }),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["data"]["maintenance_result"]["session_generation"], 2);
    let calls = std::fs::read_to_string(&log).unwrap();
    assert!(
        calls.contains("--resume resume-session-id"),
        "maintenance must preserve Claude's exact resume form: {calls:?}"
    );
}

#[tokio::test]
async fn maintenance_resume_actions_reject_a_removed_profile_before_mutation() {
    for (suffix, action, confirmed) in [
        ("retry", "retry_resume", false),
        ("terminate", "terminate_runtime_then_resume", true),
    ] {
        let tmp = tempfile::TempDir::new().unwrap();
        let cwd = tmp.path().join("repo");
        fs::create_dir_all(&cwd).unwrap();
        let id = format!("maintenance-revoked-profile-{suffix}");
        seed_resumable_session(
            tmp.path(),
            &id,
            "claude",
            &format!("hs-claude-maintenance-revoked-{suffix}"),
            &cwd,
            &["--resume", "resume-session-id"],
        );
        let record_path = tmp.path().join("sessions").join(&id).join("session.json");
        let mut record: Value = serde_json::from_slice(&fs::read(&record_path).unwrap()).unwrap();
        record["agent_bin"] = json!(fake_agent(tmp.path(), &format!("claude-gpt-{suffix}")));
        record["runtime"]["agent_profile"] = json!("claude-gpt");
        record["runtime"]["agent_profile_auto_resume_supported"] = json!(false);
        fs::write(&record_path, serde_json::to_vec_pretty(&record).unwrap()).unwrap();
        let log = tmp.path().join("tmux.log");
        let st = state(tmp.path(), Some(TOKEN), resume_tmux(tmp.path(), &log));

        let (status, preview) = call(
            router(st.clone()),
            get_auth(
                &format!("/sessions/{id}/maintenance?operation=resume"),
                Some(TOKEN),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "body={preview}");
        let maintenance = &preview["data"]["maintenance"];

        let (status, body) = call(
            router(st),
            post_json(
                &format!("/sessions/{id}/maintenance/actions"),
                Some(TOKEN),
                json!({
                    "operation": "resume",
                    "action": action,
                    "expected_session_incarnation": maintenance["session_incarnation"],
                    "expected_session_generation": maintenance["session_generation"],
                    "expected_preview_digest": maintenance["preview_digest"],
                    "confirmed": confirmed,
                }),
            ),
        )
        .await;

        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body={body}");
        assert_eq!(body["error"]["code"], "agent-profile-unavailable");
        let calls = fs::read_to_string(log).unwrap_or_default();
        assert!(
            !calls.contains("new-session"),
            "body={body}, calls={calls:?}"
        );
    }
}

#[tokio::test]
async fn maintenance_rejects_destructive_repair_for_an_unverified_process_group() {
    let tmp = tempfile::TempDir::new().unwrap();
    let cwd = tmp.path().join("repo");
    std::fs::create_dir_all(&cwd).unwrap();
    let id = "maintenance-unverified-boundary";
    seed_resumable_session(
        tmp.path(),
        id,
        "codex",
        "hs-codex-maintenance-unverified-boundary",
        &cwd,
        &[
            "resume",
            "resume-session-id",
            "--cd",
            cwd.to_str().unwrap(),
            "--no-alt-screen",
        ],
    );
    let record_path = tmp.path().join("sessions").join(id).join("session.json");
    let mut record: Value = serde_json::from_slice(&std::fs::read(&record_path).unwrap()).unwrap();
    record["runtime"]["launch_id"] = json!("maintenance-unverified-launch");
    let pane = TestProcessGroup::spawn();
    record["delete_tmux_identity"] = json!({
        "launch_id": "maintenance-unverified-launch",
        "session_id": "$98",
        "pane_id": "%98",
        "pane_pid": pane.pid(),
        "process_group_id": pane.pid(),
    });
    std::fs::write(&record_path, serde_json::to_vec_pretty(&record).unwrap()).unwrap();
    let tmux = executable(
        &tmp.path().join("stopped-unverified-tmux"),
        "#!/usr/bin/env sh\nprintf '%s\\n' \"can't find session: stopped\" >&2\nexit 1\n",
    );
    let st = state(tmp.path(), Some(TOKEN), tmux);

    let (status, preview) = call(
        router(st.clone()),
        get_auth(
            &format!("/sessions/{id}/maintenance?operation=resume"),
            Some(TOKEN),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={preview}");
    let maintenance = &preview["data"]["maintenance"];
    assert_eq!(maintenance["state"], "blocked");
    assert_eq!(maintenance["boundary"]["kind"], "process_group");

    let (status, body) = call(
        router(st),
        post_json(
            &format!("/sessions/{id}/maintenance/actions"),
            Some(TOKEN),
            json!({
                "operation": "resume",
                "action": "terminate_runtime_then_resume",
                "expected_session_incarnation": maintenance["session_incarnation"],
                "expected_session_generation": maintenance["session_generation"],
                "expected_preview_digest": maintenance["preview_digest"],
                "confirmed": true
            }),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::CONFLICT, "body={body}");
    assert_eq!(body["error"]["code"], "maintenance-preview-stale");
    assert_eq!(
        crate::process_group_status(pane.pid() as libc::pid_t),
        crate::ProcessGroupStatus::Running,
        "an unverified process group must never be signalled"
    );
    assert!(record_path.exists());
}

#[tokio::test]
async fn maintenance_retry_delete_requires_confirmation_and_redacts_safe_failure_details() {
    let tmp = tempfile::TempDir::new().unwrap();
    let cwd = tmp.path().join("repo");
    std::fs::create_dir_all(&cwd).unwrap();
    let id = "maintenance-delete-confirmation";
    seed_resumable_session(
        tmp.path(),
        id,
        "codex",
        "hs-codex-maintenance-delete-confirmation",
        &cwd,
        &[
            "resume",
            "resume-session-id",
            "--cd",
            cwd.to_str().unwrap(),
            "--no-alt-screen",
        ],
    );
    let record_path = tmp.path().join("sessions").join(id).join("session.json");
    let mut record: Value = serde_json::from_slice(&std::fs::read(&record_path).unwrap()).unwrap();
    record["runtime"]["launch_id"] = json!("maintenance-delete-confirmation-launch");
    let pane = TestProcessGroup::spawn();
    record["delete_tmux_identity"] = json!({
        "launch_id": "maintenance-delete-confirmation-launch",
        "session_id": "$97",
        "pane_id": "%97",
        "pane_pid": pane.pid(),
        "process_group_id": pane.pid(),
    });
    std::fs::write(&record_path, serde_json::to_vec_pretty(&record).unwrap()).unwrap();
    let tmux = executable(
        &tmp.path().join("stopped-delete-confirmation-tmux"),
        "#!/usr/bin/env sh\nprintf '%s\\n' \"can't find session: stopped\" >&2\nexit 1\n",
    );
    let st = state(tmp.path(), Some(TOKEN), tmux);

    let (status, preview) = call(
        router(st.clone()),
        get_auth(
            &format!("/sessions/{id}/maintenance?operation=delete"),
            Some(TOKEN),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={preview}");
    let maintenance = &preview["data"]["maintenance"];
    assert_eq!(maintenance["state"], "blocked");
    assert_eq!(maintenance["boundary"]["kind"], "process_group");
    assert_eq!(maintenance["actions"][0]["id"], "retry_delete");
    assert_eq!(maintenance["actions"][0]["destructive"], true);
    assert_eq!(maintenance["actions"][0]["requires_confirmation"], true);
    let request = json!({
        "operation": "delete",
        "action": "retry_delete",
        "expected_session_incarnation": maintenance["session_incarnation"],
        "expected_session_generation": maintenance["session_generation"],
        "expected_preview_digest": maintenance["preview_digest"]
    });

    let (status, body) = call(
        router(st.clone()),
        post_json(
            &format!("/sessions/{id}/maintenance/actions"),
            Some(TOKEN),
            request.clone(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "body={body}");
    assert_eq!(body["error"]["code"], "maintenance-confirmation-required");
    assert!(record_path.exists());
    assert_eq!(
        crate::process_group_status(pane.pid() as libc::pid_t),
        crate::ProcessGroupStatus::Running
    );

    let mut confirmed = request;
    confirmed["confirmed"] = json!(true);
    let (status, body) = call(
        router(st),
        post_json(
            &format!("/sessions/{id}/maintenance/actions"),
            Some(TOKEN),
            confirmed,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body={body}");
    assert_eq!(body["error"]["code"], "session-maintenance-failed");
    let details = body["error"]["details"].as_object().unwrap();
    let mut keys = details.keys().map(String::as_str).collect::<Vec<_>>();
    keys.sort_unstable();
    assert_eq!(
        keys,
        vec![
            "id",
            "kind",
            "operation",
            "retryable",
            "session_metadata_retained"
        ]
    );
    assert_eq!(details["kind"], "process_boundary_live");
    assert_eq!(details["session_metadata_retained"], true);
    assert!(record_path.exists());
    assert_eq!(
        crate::process_group_status(pane.pid() as libc::pid_t),
        crate::ProcessGroupStatus::Running,
        "retry delete must not signal an unverified process boundary"
    );
    let serialized = body.to_string();
    for forbidden in [
        tmp.path().to_str().unwrap(),
        "tmux_session",
        "pane_pid",
        "process_group_id",
        "cause",
    ] {
        assert!(
            !serialized.contains(forbidden),
            "maintenance failure leaked {forbidden}: {serialized}"
        );
    }
}

fn reused_numeric_tmux_fixture(root: &Path) -> (Arc<ServeState>, PathBuf) {
    let id = "reused-numeric-tmux";
    seed_resumable_session(
        root,
        id,
        "codex",
        "hs-codex-reused-numeric-tmux",
        root,
        &["resume", "resume-session-id"],
    );
    let path = root.join("sessions").join(id).join("session.json");
    let mut record: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    record.as_object_mut().unwrap().remove("startup");
    record
        .as_object_mut()
        .unwrap()
        .remove("tmux_runtime_never_launched");
    record["runtime"]["launch_id"] = json!("pre-boot-launch");
    record["delete_tmux_identity"] = json!({
        "launch_id": "pre-boot-launch", "session_id": "$91", "pane_id": "%91",
        "pane_pid": 99999999, "process_group_id": 99999999,
    });
    std::fs::write(path, serde_json::to_vec_pretty(&record).unwrap()).unwrap();
    let calls = root.join("tmux-calls");
    let tmux = executable(
        &root.join("reused-tmux"),
        &format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\nif [ \"$1\" = has-session ]; then\n  if [ \"$3\" = '$91' ]; then exit 0; fi\n  if [ \"$3\" = '=hs-codex-reused-numeric-tmux' ] && [ -f '{}' ]; then exit 0; fi\nfi\nprintf '%s\\n' \"can't find session: managed\" >&2\nexit 1\n",
            calls.display(),
            root.join("managed-name-present").display(),
        ),
    );
    (state(root, Some(TOKEN), tmux), calls)
}

#[tokio::test]
async fn maintenance_reused_numeric_tmux_allows_fenced_retry_delete() {
    assert_fenced_maintenance_removal(false).await;
    assert_fenced_maintenance_removal(true).await;
}

async fn assert_fenced_maintenance_removal(record_only: bool) {
    let tmp = tempfile::TempDir::new().unwrap();
    let (mut st, calls) = reused_numeric_tmux_fixture(tmp.path());
    if record_only {
        // Missing identity evidence still requires the v2 no-signal fallback.
        let path = tmp.path().join("sessions/reused-numeric-tmux/session.json");
        let mut record: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        record
            .as_object_mut()
            .unwrap()
            .remove("delete_tmux_identity");
        std::fs::write(path, serde_json::to_vec_pretty(&record).unwrap()).unwrap();
    }
    let action = if record_only {
        "remove_console_record"
    } else {
        "retry_delete"
    };
    let provider_root = tmp.path().join("provider/sessions");
    std::fs::create_dir_all(&provider_root).unwrap();
    let provider_history = provider_root.join("rollout.jsonl");
    let transcript = format!(
        "{}\n{}\n",
        json!({"timestamp":"2026-09-01T00:00:00Z","type":"session_meta","payload":{
            "id":"resume-session-id","cwd":tmp.path(),"source":"cli","timestamp":"2026-09-01T00:00:00Z"
        }}),
        json!({"timestamp":"2026-09-01T00:00:01Z","type":"response_item","payload":{
            "type":"message","role":"assistant","content":[{
                "type":"output_text","text":"retained provider history"
            }]
        }}),
    );
    std::fs::write(&provider_history, &transcript).unwrap();
    Arc::get_mut(&mut st).unwrap().history_catalog = Arc::new(HistoryCatalog::new(
        vec![HistorySource {
            provider: "codex".to_string(),
            agent_profile: None,
            root: provider_root,
        }],
        provider_history::archive_root(tmp.path()),
        provider_history::star_root(tmp.path()),
    ));
    let history_route = "/history/sessions?provider=codex";
    let (status, history_before) =
        call(router(st.clone()), get_auth(history_route, Some(TOKEN))).await;
    assert_eq!(status, StatusCode::OK, "{history_before}");
    let histories = history_before["data"]["sessions"].as_array().unwrap();
    assert_eq!(histories.len(), 1, "{history_before}");
    let history_id = histories[0]["id"].as_str().unwrap();
    assert_eq!(histories[0]["provider_session_id"], "resume-session-id");
    assert_eq!(histories[0]["resumable"], true);
    let messages_route =
        format!("/history/sessions/{history_id}/messages?direction=forward&limit=20");
    let (status, messages_before) =
        call(router(st.clone()), get_auth(&messages_route, Some(TOKEN))).await;
    assert_eq!(status, StatusCode::OK, "{messages_before}");
    assert_eq!(
        messages_before["data"]["messages"][0]["text"],
        "retained provider history"
    );
    let route = "/sessions/reused-numeric-tmux/maintenance";
    let (status, body) = call(
        router(st.clone()),
        get_auth(
            &format!(
                "{route}?operation=delete&schema_version=agent-session.session-maintenance.v2"
            ),
            Some(TOKEN),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let preview = &body["data"]["maintenance"];
    if record_only {
        assert_eq!(preview["state"], "blocked", "{preview}");
        assert_eq!(preview["issue"]["kind"], "runtime_identity_unavailable");
    } else {
        assert_eq!(preview["state"], "healthy", "{preview}");
        assert!(preview["issue"].is_null());
        assert_eq!(preview["boundary"]["kind"], "none");
    }
    assert_eq!(preview["boundary"]["safe_process_count"], 0);
    assert!(
        preview["actions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a["id"] == action)
    );
    let action_route = "/sessions/reused-numeric-tmux/maintenance/actions";
    let mut request = json!({
        "schema_version": "agent-session.session-maintenance.v2", "operation": "delete",
        "action": action, "expected_session_incarnation": preview["session_incarnation"],
        "expected_session_generation": preview["session_generation"], "expected_preview_digest": preview["preview_digest"],
        "confirmed": false,
    });
    let (status, body) = call(
        router(st.clone()),
        post_json(action_route, Some(TOKEN), request.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["error"]["code"], "maintenance-confirmation-required");
    request["confirmed"] = json!(true);
    request["expected_session_generation"] = json!(2);
    let (status, body) = call(
        router(st.clone()),
        post_json(action_route, Some(TOKEN), request.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"]["code"], "maintenance-preview-stale");
    request["expected_session_generation"] = preview["session_generation"].clone();
    let managed_present = tmp.path().join("managed-name-present");
    std::fs::write(&managed_present, b"replacement exists").unwrap();
    let (status, body) = call(
        router(st.clone()),
        post_json(action_route, Some(TOKEN), request.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"]["code"], "maintenance-preview-stale");
    assert!(
        tmp.path()
            .join("sessions/reused-numeric-tmux/session.json")
            .exists()
    );
    std::fs::remove_file(managed_present).unwrap();
    let (status, body) = call(
        router(st.clone()),
        post_json(action_route, Some(TOKEN), request),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let expected = if record_only {
        "record_removed"
    } else {
        "deleted"
    };
    assert_eq!(body["data"]["maintenance_result"]["outcome"], expected);
    assert_eq!(body["data"]["maintenance_result"]["status"], expected);
    assert!(!tmp.path().join("sessions/reused-numeric-tmux").exists());
    st.history_catalog.invalidate();
    let (status, history_after) =
        call(router(st.clone()), get_auth(history_route, Some(TOKEN))).await;
    assert_eq!(status, StatusCode::OK, "{history_after}");
    let histories = history_after["data"]["sessions"].as_array().unwrap();
    assert_eq!(histories.len(), 1, "{history_after}");
    assert_eq!(histories[0]["id"], history_id);
    assert_eq!(histories[0]["provider_session_id"], "resume-session-id");
    assert_eq!(histories[0]["resumable"], true);
    let (status, messages_after) = call(router(st), get_auth(&messages_route, Some(TOKEN))).await;
    assert_eq!(status, StatusCode::OK, "{messages_after}");
    assert_eq!(
        messages_after["data"]["messages"],
        messages_before["data"]["messages"]
    );
    assert_eq!(
        std::fs::read_to_string(provider_history).unwrap(),
        transcript
    );
    let calls = std::fs::read_to_string(calls).unwrap();
    for forbidden in ["if-shell", "kill-session", "send-keys", "new-session"] {
        assert!(!calls.contains(forbidden), "unexpected mutation: {calls}");
    }
}

#[tokio::test]
async fn maintenance_reused_numeric_tmux_uses_safe_retry_in_both_contracts() {
    let tmp = tempfile::TempDir::new().unwrap();
    let (st, _) = reused_numeric_tmux_fixture(tmp.path());
    for (operation, schema) in [
        ("delete", "agent-session.session-maintenance.v1"),
        ("resume", "agent-session.session-maintenance.v2"),
        ("attach", "agent-session.session-maintenance.v2"),
        ("inspect", "agent-session.session-maintenance.v2"),
    ] {
        let (status, body) = call(router(st.clone()), get_auth(
                &format!("/sessions/reused-numeric-tmux/maintenance?operation={operation}&schema_version={schema}"), Some(TOKEN),
            )).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let preview = &body["data"]["maintenance"];
        assert_eq!(preview["state"], "healthy", "{preview}");
        assert!(
            !preview["actions"]
                .as_array()
                .unwrap()
                .iter()
                .any(|a| a["id"] == "remove_console_record")
        );
    }
    assert!(
        tmp.path()
            .join("sessions/reused-numeric-tmux/session.json")
            .exists()
    );
}

#[tokio::test]
async fn resume_api_reports_retry_resume_for_a_surviving_runtime() {
    let tmp = tempfile::TempDir::new().unwrap();
    let cwd = tmp.path().join("repo");
    std::fs::create_dir_all(&cwd).unwrap();
    let id = "resume-surviving-runtime";
    seed_resumable_session(
        tmp.path(),
        id,
        "codex",
        "hs-codex-resume-surviving-runtime",
        &cwd,
        &[
            "resume",
            "resume-session-id",
            "--cd",
            cwd.to_str().unwrap(),
            "--no-alt-screen",
        ],
    );
    let record_path = tmp.path().join("sessions").join(id).join("session.json");
    let mut record: Value = serde_json::from_slice(&std::fs::read(&record_path).unwrap()).unwrap();
    record["runtime"]["launch_id"] = json!("surviving-launch");
    let pane = TestProcessGroup::spawn();
    record["delete_tmux_identity"] = json!({
        "launch_id": "surviving-launch",
        "session_id": "$98",
        "pane_id": "%98",
        "pane_pid": pane.pid(),
        "process_group_id": pane.pid(),
    });
    std::fs::write(&record_path, serde_json::to_vec_pretty(&record).unwrap()).unwrap();
    let tmux = executable(
        &tmp.path().join("stopped-runtime-tmux"),
        "#!/usr/bin/env sh\nprintf '%s\\n' \"can't find session: stopped\" >&2\nexit 1\n",
    );
    let st = state(tmp.path(), Some(TOKEN), tmux);

    let (status, body) = call(
        router(st),
        post_json(&format!("/sessions/{id}/resume"), Some(TOKEN), json!({})),
    )
    .await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body={body}");
    assert_eq!(body["error"]["code"], "session-termination-failed");
    assert_eq!(body["error"]["details"]["reason"], "process-still-running");
    assert_eq!(body["error"]["details"]["retryable"], true);
    assert_eq!(body["error"]["details"]["action"], "retry-resume");
    assert!(record_path.exists());
}

#[tokio::test]
async fn resume_api_reports_retry_resume_for_a_surviving_prior_runtime() {
    let tmp = tempfile::TempDir::new().unwrap();
    let cwd = tmp.path().join("repo");
    std::fs::create_dir_all(&cwd).unwrap();
    let id = "resume-surviving-prior-runtime";
    seed_resumable_session(
        tmp.path(),
        id,
        "codex",
        "hs-codex-resume-surviving-prior-runtime",
        &cwd,
        &[
            "resume",
            "resume-session-id",
            "--cd",
            cwd.to_str().unwrap(),
            "--no-alt-screen",
        ],
    );
    let record_path = tmp.path().join("sessions").join(id).join("session.json");
    let mut record: Value = serde_json::from_slice(&std::fs::read(&record_path).unwrap()).unwrap();
    record["runtime"]["launch_id"] = json!("surviving-prior-launch");
    let prior_pane = TestProcessGroup::spawn();
    record["delete_tmux_identity"] = json!({
        "launch_id": "surviving-prior-launch",
        "session_id": "$98",
        "pane_id": "%98",
        "pane_pid": 99999999,
        "process_group_id": 99999999,
    });
    record["delete_tmux_prior_identities"] = json!([{
        "launch_id": "surviving-prior-launch",
        "session_id": "$98",
        "pane_id": "%98",
        "pane_pid": prior_pane.pid(),
        "process_group_id": prior_pane.pid(),
    }]);
    std::fs::write(&record_path, serde_json::to_vec_pretty(&record).unwrap()).unwrap();
    let tmux = executable(
        &tmp.path().join("stopped-prior-runtime-tmux"),
        "#!/usr/bin/env sh\nprintf '%s\\n' \"can't find session: stopped\" >&2\nexit 1\n",
    );
    let st = state(tmp.path(), Some(TOKEN), tmux);

    let (status, body) = call(
        router(st),
        post_json(&format!("/sessions/{id}/resume"), Some(TOKEN), json!({})),
    )
    .await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body={body}");
    assert_eq!(body["error"]["code"], "session-termination-failed");
    assert_eq!(body["error"]["details"]["reason"], "process-still-running");
    assert_eq!(body["error"]["details"]["retryable"], true);
    assert_eq!(body["error"]["details"]["action"], "retry-resume");
    let retained: Value = serde_json::from_slice(&std::fs::read(&record_path).unwrap()).unwrap();
    assert_eq!(
        retained["delete_tmux_prior_identities"][0]["process_group_id"],
        prior_pane.pid()
    );
}

#[tokio::test]
async fn resume_refuses_non_resumable_session_with_token() {
    let tmp = tempfile::TempDir::new().unwrap();
    let log = tmp.path().join("tmux.log");
    let tmux = resume_tmux(tmp.path(), &log);
    seed_session(tmp.path(), "plain", "codex", "hs-codex-plain");
    let st = state(tmp.path(), Some(TOKEN), tmux);

    let (status, body) = call(
        router(st.clone()),
        post_json("/sessions/plain/resume", Some(TOKEN), json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["schema_version"], "cli.agent-session.serve.v1");
    assert_eq!(body["ok"], false);
    assert_eq!(body["error"]["code"], "session-not-resumable");
    let calls = std::fs::read_to_string(&log).unwrap_or_default();
    assert!(
        !calls.contains("new-session"),
        "non-resumable sessions must not create tmux runtimes: {calls:?}"
    );
}

#[tokio::test]
async fn display_metadata_preserves_immutable_forge_role_binding() {
    use pretty_assertions::assert_eq;
    for role in [Some("reviewer"), None] {
        let tmp = tempfile::TempDir::new().unwrap();
        seed_session_with_runtime(tmp.path(), "bound-display", "codex", "hs-bound-display");
        let st = state(tmp.path(), Some(TOKEN), minimal_tmux(tmp.path()));
        let mut original = load_session_record(&st.context, "bound-display").unwrap();
        original.role = role.map(str::to_string);
        original.lineage = Some(
            serde_json::from_value(json!({
                "schema_version":"agent-session.session-lineage.v1", "machine":MACHINE,
                "parent":null, "root":{"machine":MACHINE,"session_id":original.id,
                    "session_created_at":original.created_at}, "depth":0,
                "starter":{"kind":"operator","via":"cli"},
                "forge_context":{"initiator":"operator","role":role}
            }))
            .unwrap(),
        );
        crate::write_session_record(&st.context, &original).unwrap();
        let path = tmp.path().join("sessions/bound-display/session.json");
        let before = fs::read(&path).unwrap();
        let launch = crate::forge_identity::context_of(&original)
            .unwrap()
            .unwrap();
        let requested_role = if role.is_some() { "tester" } else { "reviewer" };
        let (status, body) = call(
            router(st.clone()),
            post_json(
                "/sessions/bound-display/display-metadata",
                Some(TOKEN),
                json!({
                    "expected_session_created_at":original.created_at, "expected_revision":0,
                    "role":requested_role, "title_mode":"pinned"
                }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        assert_eq!(body["error"]["code"], "display-role-bound");
        assert_eq!(body["error"]["details"]["retryable"], false);
        assert_eq!(body["error"]["details"]["next_action"], "start_new_session");
        assert_eq!(fs::read(&path).unwrap(), before);
        let record = load_session_record(&st.context, &original.id).unwrap();
        assert_eq!(
            crate::forge_identity::context_of(&record).unwrap().unwrap(),
            launch
        );
        assert!(
            crate::lineage::LineageSeed::child_of(MACHINE, MACHINE, &record, "session", "cli")
                .is_ok()
        );

        // The same role and mode-only edits remain legal on bound sessions.
        let mut request = json!({"expected_session_created_at":original.created_at,
                "expected_revision":0, "title_mode":"pinned"});
        if let Some(role) = role {
            request["role"] = json!(role);
        }
        let (status, body) = call(
            router(st.clone()),
            post_json(
                "/sessions/bound-display/display-metadata",
                Some(TOKEN),
                request.clone(),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["data"]["session"]["title_mode"], "pinned");
        assert_eq!(body["data"]["session"]["display_revision"], 1);
        let updated = load_session_record(&st.context, &original.id).unwrap();
        assert_eq!(updated.role, original.role);
        assert_eq!(
            serde_json::to_value(&updated.lineage).unwrap(),
            serde_json::to_value(&original.lineage).unwrap()
        );
        assert_eq!(
            crate::forge_identity::context_of(&updated)
                .unwrap()
                .unwrap(),
            launch
        );
        assert!(
            crate::lineage::LineageSeed::child_of(MACHINE, MACHINE, &updated, "session", "cli")
                .is_ok()
        );
        request["expected_revision"] = json!(1);
        let before_noop = fs::read(&path).unwrap();
        let (status, body) = call(
            router(st),
            post_json(
                "/sessions/bound-display/display-metadata",
                Some(TOKEN),
                request,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["data"]["session"]["display_revision"], 1);
        assert_eq!(fs::read(&path).unwrap(), before_noop);
    }
}

#[tokio::test]
async fn display_metadata_domain_coordinator_requires_root_without_writing() {
    use pretty_assertions::assert_eq;
    let tmp = tempfile::TempDir::new().unwrap();
    seed_session_with_runtime(tmp.path(), "domain-display", "codex", "hs-domain-display");
    let st = state(tmp.path(), Some(TOKEN), minimal_tmux(tmp.path()));
    let mut record = load_session_record(&st.context, "domain-display").unwrap();
    let reference = json!({"machine":"fixture-machine","session_id":"parent-session",
            "session_created_at":"2000-01-01T00:00:00Z"});
    record.lineage = Some(
        serde_json::from_value(json!({
            "schema_version":"agent-session.session-lineage.v1", "machine":MACHINE,
            "parent":reference, "root":reference, "depth":1,
            "starter":{"kind":"session","via":"cli"}
        }))
        .unwrap(),
    );
    crate::write_session_record(&st.context, &record).unwrap();
    let path = tmp.path().join("sessions/domain-display/session.json");
    let before = fs::read(&path).unwrap();
    let (status, body) = call(
        router(st),
        post_json(
            "/sessions/domain-display/display-metadata",
            Some(TOKEN),
            json!({
                "expected_session_created_at":record.created_at, "expected_revision":0,
                "role":"domain-coordinator", "title_mode":"pinned"
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["error"]["code"], "role-requires-root");
    assert_eq!(
        body["error"]["message"],
        "role domain-coordinator needs a root start: use --no-parent, or a start with no parent"
    );
    assert_eq!(fs::read(path).unwrap(), before);
}

#[tokio::test]
async fn display_metadata_is_exact_authenticated_fenced_and_persistent() {
    let tmp = tempfile::TempDir::new().unwrap();
    seed_session_with_runtime(tmp.path(), "display", "codex", "hs-display");
    let st = state(tmp.path(), Some(TOKEN), minimal_tmux(tmp.path()));
    let original = load_session_record(&st.context, "display").unwrap();
    let original_title = original.title.clone();
    for token in [None, Some(TOKEN)] {
        let (status, body) = call(
            router(st.clone()),
            post_json("/sessions/display/display-metadata", token, json!({})),
        )
        .await;
        assert_eq!(
            status,
            if token.is_none() {
                StatusCode::UNAUTHORIZED
            } else {
                StatusCode::BAD_REQUEST
            }
        );
        assert_eq!(body["schema_version"], "cli.agent-session.serve.v1");
    }
    let request = json!({"expected_session_created_at":"2000-01-01T00:00:00Z",
            "expected_revision":0, "role":"reviewer", "title_mode":"pinned"});
    let (status, _) = call(
        router(st.clone()),
        post_json("/sessions/display/display-metadata", None, request.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, body) = call(
        router(st.clone()),
        post_json(
            "/sessions/display/display-metadata",
            Some(TOKEN),
            request.clone(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["session"]["role"], "reviewer");
    assert_eq!(body["data"]["session"]["title_mode"], "pinned");
    assert_eq!(body["data"]["session"]["display_revision"], 1);
    let record = load_session_record(&st.context, "display").unwrap();
    assert_eq!(record.extra["title_mode"], "pinned");
    assert_eq!(record.title_revision, original.title_revision + 1);
    assert_eq!(record.runtime.as_ref().unwrap().launch_id, "launch-display");
    let (status, _) = call(
        router(st.clone()),
        post_json("/sessions/display/display-metadata", Some(TOKEN), request),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    let (status, _) = call(router(st.clone()), post_json("/sessions/dis/display-metadata", Some(TOKEN), json!({"expected_session_created_at":"2000-01-01T00:00:00Z", "expected_revision":1,"title_mode":"auto"}))).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, body) = call(router(st.clone()), post_json("/sessions/display/display-metadata", Some(TOKEN), json!({"expected_session_created_at":"2000-01-01T00:00:00Z", "expected_revision":1,"title_mode":"auto"}))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["session"]["title_mode"], "auto");
    assert_eq!(body["data"]["session"]["role"], "reviewer");
    let record = load_session_record(&st.context, "display").unwrap();
    assert_eq!(record.title_revision, original.title_revision + 2);
    let (status, body) = call(router(st.clone()), post_json("/sessions/display/retitle", Some(TOKEN), json!({
            "schema_version":"agent-session.session-retitle.request.v2", "trigger":"manual",
            "idempotency_key":"stale-display-mode-request", "expected_session_incarnation":"launch-display",
            "expected_title_revision":original.title_revision
        }))).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"]["code"], "title-revision-conflict");
    assert_eq!(
        load_session_record(&st.context, "display").unwrap().title,
        original_title
    );
}

#[tokio::test]
async fn update_session_title_persists_and_clears_custom_title() {
    let tmp = tempfile::TempDir::new().unwrap();
    let tmux = minimal_tmux(tmp.path());
    seed_session(tmp.path(), "steer", "codex", "hs-codex-steer");
    let st = state(tmp.path(), Some(TOKEN), tmux);

    let (status, body) = call(
        router(st.clone()),
        patch_json(
            "/sessions/steer",
            Some(TOKEN),
            json!({ "title": "Reviewed title" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["schema_version"], "cli.agent-session.serve.v1");
    assert_eq!(body["data"]["machine"], MACHINE);
    assert_eq!(body["data"]["session"]["title"], "Reviewed title");

    let record_path = tmp.path().join("sessions/steer/session.json");
    let record: Value =
        serde_json::from_str(&std::fs::read_to_string(&record_path).unwrap()).unwrap();
    assert_eq!(record["title"], "Reviewed title");
    assert_ne!(record["updated_at"], "2000-01-01T00:00:00Z");

    let (status, body) = call(
        router(st.clone()),
        patch_json("/sessions/steer", Some(TOKEN), json!({ "title": null })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["data"]["session"]["title"], Value::Null);

    let (status, body) = call(
        router(st.clone()),
        patch_json("/sessions/steer", Some(TOKEN), json!({ "title": "" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["data"]["session"]["title"], Value::Null);

    let (status, body) = call(
        router(st.clone()),
        patch_json("/sessions/steer", Some(TOKEN), json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "missing-title");

    let too_long = "x".repeat(121);
    let (status, body) = call(
        router(st.clone()),
        patch_json("/sessions/steer", Some(TOKEN), json!({ "title": too_long })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "title-too-long");

    let (status, body) = call(
        router(st.clone()),
        patch_json("/sessions/ghost", Some(TOKEN), json!({ "title": "Ghost" })),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["code"], "session-not-found");
}

#[tokio::test]
async fn update_session_title_state_renders_and_persists_without_delimiter_inference() {
    let tmp = tempfile::TempDir::new().unwrap();
    let tmux = minimal_tmux(tmp.path());
    seed_session_with_runtime(
        tmp.path(),
        "structured-title",
        "codex",
        "hs-codex-structured-title",
    );
    let st = state(tmp.path(), Some(TOKEN), tmux);

    let user_state = json!({
        "topic": "Retitle #317 rules",
        "topic_source": "user",
        "references": ["#317"],
        "activity": "Implement daemon contract"
    });
    let (status, body) = call(
        router(st.clone()),
        patch_json(
            "/sessions/structured-title",
            Some(TOKEN),
            json!({
                "title": null,
                "title_state": user_state,
                "expected_title_revision": 0,
                "expected_session_incarnation": "launch-structured-title",
                "expected_session_title": null
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body={body}");
    assert_eq!(body["error"]["code"], "title-state-mismatch");

    let (status, body) = call(
        router(st.clone()),
        patch_json(
            "/sessions/structured-title",
            Some(TOKEN),
            json!({
                "title_state": user_state,
                "expected_title_revision": 0,
                "expected_session_incarnation": "launch-structured-title",
                "expected_session_title": null
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(
        body["data"]["session"]["title"],
        "Retitle #317 rules - Implement daemon contract"
    );
    assert_eq!(body["data"]["session"]["title_state"], user_state);
    assert_eq!(body["data"]["session"]["title_state_supported"], true);
    assert_eq!(body["data"]["session"]["title_revision"], 1);

    let activity_only_state = json!({
        "topic": null,
        "topic_source": "none",
        "references": [],
        "activity": "中文說明"
    });
    let (status, body) = call(
        router(st.clone()),
        patch_json(
            "/sessions/structured-title",
            Some(TOKEN),
            json!({
                "title_state": activity_only_state,
                "expected_title_revision": 1,
                "expected_session_incarnation": "launch-structured-title",
                "expected_session_title": "Retitle #317 rules - Implement daemon contract"
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["data"]["session"]["title"], "中文說明");
    assert_eq!(body["data"]["session"]["title_state"], activity_only_state);
    assert_eq!(body["data"]["session"]["title_revision"], 2);

    let (status, body) = call(
        router(st),
        patch_json(
            "/sessions/structured-title",
            Some(TOKEN),
            json!({
                "title": "Direct rewrite",
                "expected_title_revision": 2,
                "expected_session_incarnation": "launch-structured-title",
                "expected_session_title": "中文說明"
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["data"]["session"]["title"], "Direct rewrite");
    assert!(body["data"]["session"].get("title_state").is_none());
    assert_eq!(body["data"]["session"]["title_revision"], 3);

    let record_path = tmp.path().join("sessions/structured-title/session.json");
    let record: Value =
        serde_json::from_str(&std::fs::read_to_string(&record_path).unwrap()).unwrap();
    assert_eq!(record["title"], "Direct rewrite");
    assert!(record.get("title_state").is_none());
    assert_eq!(record["title_revision"], 3);
}

#[tokio::test]
async fn create_rejects_explicit_null_with_nonempty_title_state_in_both_modes() {
    let tmp = tempfile::TempDir::new().unwrap();
    let st = state(tmp.path(), Some(TOKEN), minimal_tmux(tmp.path()));
    let title_state = json!({
        "topic": "Explicit topic",
        "topic_source": "user",
        "references": [],
        "activity": null
    });
    let cases = [
        json!({
            "agent": "codex",
            "id": "null-title-fresh",
            "title": null,
            "title_state": title_state
        }),
        json!({
            "agent": "codex",
            "id": "null-title-provider-resume",
            "provider_resume_id": "missing-provider-session",
            "title": null,
            "title_state": title_state
        }),
    ];

    for body in cases {
        let (status, body) = call(
            router(st.clone()),
            post_json("/sessions", Some(TOKEN), body),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "body={body}");
        assert_eq!(body["error"]["code"], "title-state-mismatch");
    }
}

#[tokio::test]
async fn expected_session_title_uses_the_unicode_title_limit() {
    let tmp = tempfile::TempDir::new().unwrap();
    let tmux = minimal_tmux(tmp.path());
    seed_session_with_runtime(
        tmp.path(),
        "unicode-title",
        "codex",
        "hs-codex-unicode-title",
    );
    let st = state(tmp.path(), Some(TOKEN), tmux);
    let observed_title = "標".repeat(120);

    let (status, body) = call(
        router(st.clone()),
        patch_json(
            "/sessions/unicode-title",
            Some(TOKEN),
            json!({ "title": observed_title }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");

    let (status, body) = call(
        router(st),
        patch_json(
            "/sessions/unicode-title",
            Some(TOKEN),
            json!({
                "title": "精簡中文標題",
                "expected_title_revision": 1,
                "expected_session_created_at": "2000-01-01T00:00:00Z",
                "expected_session_incarnation": "launch-unicode-title",
                "expected_session_title": observed_title
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["data"]["session"]["title"], "精簡中文標題");
    assert_eq!(body["data"]["session"]["title_revision"], 2);
}

#[tokio::test]
async fn expected_session_title_repairs_a_long_existing_title() {
    let tmp = tempfile::TempDir::new().unwrap();
    let tmux = minimal_tmux(tmp.path());
    seed_session_with_runtime(tmp.path(), "long-title", "codex", "hs-codex-long-title");
    let record_path = tmp.path().join("sessions/long-title/session.json");
    let long_existing_title = "previous raw prompt ".repeat(20);
    let mut record: Value =
        serde_json::from_str(&std::fs::read_to_string(&record_path).unwrap()).unwrap();
    record["title"] = json!(long_existing_title);
    std::fs::write(&record_path, serde_json::to_vec_pretty(&record).unwrap()).unwrap();
    let st = state(tmp.path(), Some(TOKEN), tmux);

    let (status, body) = call(
        router(st),
        patch_json(
            "/sessions/long-title",
            Some(TOKEN),
            json!({
                "title": "Concise repaired title",
                "expected_title_revision": 0,
                "expected_session_created_at": "2000-01-01T00:00:00Z",
                "expected_session_incarnation": "launch-long-title",
                "expected_session_title": long_existing_title
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["data"]["session"]["title"], "Concise repaired title");
    assert_eq!(body["data"]["session"]["title_revision"], 1);
}

#[tokio::test]
async fn update_session_title_rejects_stale_revision() {
    let tmp = tempfile::TempDir::new().unwrap();
    let tmux = minimal_tmux(tmp.path());
    seed_session_with_runtime(tmp.path(), "revision", "codex", "hs-codex-revision");
    let st = state(tmp.path(), Some(TOKEN), tmux);

    let (status, body) = call(
        router(st.clone()),
        patch_json(
            "/sessions/revision",
            Some(TOKEN),
            json!({
                "title": "First title",
                "expected_title_revision": 0,
                "expected_session_created_at": "2000-01-01T00:00:00Z",
                "expected_session_incarnation": "launch-revision",
                "expected_session_title": null
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["data"]["session"]["title"], "First title");
    assert_eq!(body["data"]["session"]["title_revision"], 1);

    let (status, body) = call(
        router(st.clone()),
        patch_json(
            "/sessions/revision",
            Some(TOKEN),
            json!({
                "title": "Stale title",
                "expected_title_revision": 0,
                "expected_session_created_at": "2000-01-01T00:00:00Z"
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "body={body}");
    assert_eq!(body["error"]["code"], "title-revision-conflict");
    assert_eq!(body["error"]["details"]["expected_title_revision"], 0);
    assert_eq!(body["error"]["details"]["actual_title_revision"], 1);

    let (status, body) = call(
        router(st.clone()),
        patch_json(
            "/sessions/revision",
            Some(TOKEN),
            json!({
                "title": "Second title",
                "expected_title_revision": 1,
                "expected_session_created_at": "2000-01-01T00:00:00Z"
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["data"]["session"]["title"], "Second title");
    assert_eq!(body["data"]["session"]["title_revision"], 2);

    let (status, body) = call(
        router(st),
        patch_json(
            "/sessions/revision",
            Some(TOKEN),
            json!({
                "title": "Second title",
                "expected_title_revision": 1,
                "expected_session_created_at": "2000-01-01T00:00:00Z"
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "body={body}");
    assert_eq!(body["error"]["code"], "title-revision-conflict");
    assert_eq!(body["error"]["details"]["actual_title_revision"], 2);

    let record_path = tmp.path().join("sessions/revision/session.json");
    let record: Value =
        serde_json::from_str(&std::fs::read_to_string(&record_path).unwrap()).unwrap();
    assert_eq!(record["title"], "Second title");
    assert_eq!(record["title_revision"], 2);
}

#[tokio::test]
async fn update_session_title_allows_one_concurrent_writer_per_revision() {
    let tmp = tempfile::TempDir::new().unwrap();
    let tmux = minimal_tmux(tmp.path());
    seed_session(
        tmp.path(),
        "concurrent-revision",
        "codex",
        "hs-codex-concurrent-revision",
    );
    let st = state(tmp.path(), Some(TOKEN), tmux);
    let barrier = Arc::new(tokio::sync::Barrier::new(3));

    let spawn_writer = |topic: &'static str, activity: &'static str| {
        let st = st.clone();
        let barrier = barrier.clone();
        tokio::spawn(async move {
            barrier.wait().await;
            call(
                router(st),
                patch_json(
                    "/sessions/concurrent-revision",
                    Some(TOKEN),
                    json!({
                        "title_state": {
                            "topic": topic,
                            "topic_source": "auto",
                            "references": [],
                            "activity": activity
                        },
                        "expected_title_revision": 0
                    }),
                ),
            )
            .await
        })
    };
    let first = spawn_writer("First topic", "First contender");
    let second = spawn_writer("Second topic", "Second contender");
    barrier.wait().await;
    let (first, second) = tokio::join!(first, second);
    let responses = [first.expect("first writer"), second.expect("second writer")];
    let successes = responses
        .iter()
        .filter(|(status, _)| *status == StatusCode::OK)
        .count();
    let conflicts = responses
        .iter()
        .filter(|(status, body)| {
            *status == StatusCode::CONFLICT && body["error"]["code"] == "title-revision-conflict"
        })
        .count();
    assert_eq!(successes, 1, "responses={responses:?}");
    assert_eq!(conflicts, 1, "responses={responses:?}");

    let record_path = tmp.path().join("sessions/concurrent-revision/session.json");
    let record: Value =
        serde_json::from_str(&std::fs::read_to_string(&record_path).unwrap()).unwrap();
    let first_state = json!({
        "topic": "First topic",
        "topic_source": "auto",
        "references": [],
        "activity": "First contender"
    });
    let second_state = json!({
        "topic": "Second topic",
        "topic_source": "auto",
        "references": [],
        "activity": "Second contender"
    });
    assert!(
        (record["title"] == "First topic - First contender"
            && record["title_state"] == first_state)
            || (record["title"] == "Second topic - Second contender"
                && record["title_state"] == second_state),
        "record={record:?}"
    );
    assert_eq!(record["title_revision"], 1);
}

#[tokio::test]
async fn update_session_title_rejects_a_replaced_session_incarnation() {
    let tmp = tempfile::TempDir::new().unwrap();
    let tmux = minimal_tmux(tmp.path());
    seed_session_with_runtime(tmp.path(), "recreated", "codex", "hs-codex-recreated");
    std::fs::remove_dir_all(tmp.path().join("sessions/recreated")).unwrap();
    seed_session_with_runtime(tmp.path(), "recreated", "codex", "hs-codex-recreated");
    let record_path = tmp.path().join("sessions/recreated/session.json");
    let mut replacement: Value =
        serde_json::from_str(&std::fs::read_to_string(&record_path).unwrap()).unwrap();
    replacement["runtime"]["launch_id"] = json!("launch-recreated-v2");
    std::fs::write(
        &record_path,
        serde_json::to_vec_pretty(&replacement).unwrap(),
    )
    .unwrap();
    let st = state(tmp.path(), Some(TOKEN), tmux);

    let (status, body) = call(
        router(st),
        patch_json(
            "/sessions/recreated",
            Some(TOKEN),
            json!({
                "title": "Stale incarnation title",
                "expected_title_revision": 0,
                "expected_session_created_at": "2000-01-01T00:00:00Z",
                "expected_session_incarnation": "launch-recreated",
                "expected_session_title": null
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "body={body}");
    assert_eq!(body["error"]["code"], "session-incarnation-conflict");
    assert_eq!(
        body["error"]["details"]["actual_session_incarnation"],
        "launch-recreated-v2"
    );
    let persisted: Value =
        serde_json::from_str(&std::fs::read_to_string(&record_path).unwrap()).unwrap();
    assert_eq!(persisted["title"], Value::Null);
    assert_eq!(persisted["title_revision"], Value::Null);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn update_session_title_maps_a_runtime_replaced_while_waiting_to_conflict() {
    let tmp = tempfile::TempDir::new().unwrap();
    let tmux = minimal_tmux(tmp.path());
    seed_session_with_runtime(
        tmp.path(),
        "waiting-replace",
        "codex",
        "hs-codex-waiting-replace",
    );
    let context = crate::CliContext {
        state_dir: tmp.path().to_path_buf(),
        host: None,
    };
    let lock = crate::acquire_session_record_lock(&context, "waiting-replace").unwrap();
    let st = state(tmp.path(), Some(TOKEN), tmux);
    let request = tokio::spawn(async move {
        call(
            router(st),
            patch_json(
                "/sessions/waiting-replace",
                Some(TOKEN),
                json!({
                    "title": "Must not cross runtime replacement",
                    "expected_title_revision": 0,
                    "expected_session_incarnation": "launch-waiting-replace",
                    "expected_session_title": null
                }),
            ),
        )
        .await
    });

    sync_waits::wait_for_record_lock_waiter(tmp.path(), "waiting-replace").await;
    let record_path = tmp.path().join("sessions/waiting-replace/session.json");
    let mut replacement: Value =
        serde_json::from_str(&std::fs::read_to_string(&record_path).unwrap()).unwrap();
    replacement["runtime"]["launch_id"] = json!("launch-waiting-replace-v2");
    std::fs::write(
        &record_path,
        serde_json::to_vec_pretty(&replacement).unwrap(),
    )
    .unwrap();
    drop(lock);

    let (status, body) = request.await.unwrap();
    assert_eq!(status, StatusCode::CONFLICT, "body={body}");
    assert_eq!(body["error"]["code"], "session-incarnation-conflict");
    assert_eq!(
        body["error"]["details"]["actual_session_incarnation"],
        "launch-waiting-replace-v2"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn update_session_title_maps_a_record_recreated_while_waiting_to_conflict() {
    let tmp = tempfile::TempDir::new().unwrap();
    let tmux = minimal_tmux(tmp.path());
    seed_session_with_runtime(
        tmp.path(),
        "waiting-recreate",
        "codex",
        "hs-codex-waiting-recreate",
    );
    let context = crate::CliContext {
        state_dir: tmp.path().to_path_buf(),
        host: None,
    };
    let lock = crate::acquire_session_record_lock(&context, "waiting-recreate").unwrap();
    let st = state(tmp.path(), Some(TOKEN), tmux);
    let request = tokio::spawn(async move {
        call(
            router(st),
            patch_json(
                "/sessions/waiting-recreate",
                Some(TOKEN),
                json!({
                    "title": "Must not cross record recreation",
                    "expected_title_revision": 0,
                    "expected_session_created_at": "2000-01-01T00:00:00Z",
                    "expected_session_title": null
                }),
            ),
        )
        .await
    });

    sync_waits::wait_for_record_lock_waiter(tmp.path(), "waiting-recreate").await;
    let record_path = tmp.path().join("sessions/waiting-recreate/session.json");
    let mut replacement: Value =
        serde_json::from_str(&std::fs::read_to_string(&record_path).unwrap()).unwrap();
    replacement["created_at"] = json!("2001-01-01T00:00:00Z");
    replacement["runtime"]["launch_id"] = json!("launch-waiting-recreate-v2");
    std::fs::write(
        &record_path,
        serde_json::to_vec_pretty(&replacement).unwrap(),
    )
    .unwrap();
    drop(lock);

    let (status, body) = request.await.unwrap();
    assert_eq!(status, StatusCode::CONFLICT, "body={body}");
    assert_eq!(body["error"]["code"], "session-incarnation-conflict");
    assert_eq!(
        body["error"]["details"]["actual_session_created_at"],
        "2001-01-01T00:00:00Z"
    );
}

#[tokio::test]
async fn expected_title_detects_repeated_legacy_writer_changes() {
    let tmp = tempfile::TempDir::new().unwrap();
    let tmux = minimal_tmux(tmp.path());
    seed_session_with_runtime(tmp.path(), "rollback", "codex", "hs-codex-rollback");
    let st = state(tmp.path(), Some(TOKEN), tmux);
    let record_path = tmp.path().join("sessions/rollback/session.json");

    let (status, body) = call(router(st.clone()), get("/sessions")).await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["data"]["sessions"][0]["title"], Value::Null);
    assert_eq!(body["data"]["sessions"][0]["title_revision"], 0);
    assert_eq!(
        body["data"]["sessions"][0]["session_incarnation"],
        "launch-rollback"
    );

    // Simulate an origin/main binary updating an untouched pre-revision record:
    // it mutates only title and preserves unknown revision fields.
    let mut rolled_back: Value =
        serde_json::from_str(&std::fs::read_to_string(&record_path).unwrap()).unwrap();
    rolled_back["title"] = json!("First old daemon title");
    std::fs::write(
        &record_path,
        serde_json::to_vec_pretty(&rolled_back).unwrap(),
    )
    .unwrap();

    let (status, body) = call(
        router(st.clone()),
        patch_json(
            "/sessions/rollback",
            Some(TOKEN),
            json!({
                "title": "Stale upgraded title",
                "expected_title_revision": 0,
                "expected_session_created_at": "2000-01-01T00:00:00Z",
                "expected_session_title": null
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "body={body}");
    assert_eq!(body["error"]["code"], "title-state-conflict");
    assert_eq!(
        body["error"]["details"]["actual_session_title"],
        "First old daemon title"
    );

    // A second old-daemon mutation must also invalidate a client that read
    // the first old-daemon title while the numeric revision stayed at zero.
    let mut rolled_back: Value =
        serde_json::from_str(&std::fs::read_to_string(&record_path).unwrap()).unwrap();
    rolled_back["title"] = json!("Second old daemon title");
    std::fs::write(
        &record_path,
        serde_json::to_vec_pretty(&rolled_back).unwrap(),
    )
    .unwrap();

    let (status, body) = call(
        router(st.clone()),
        patch_json(
            "/sessions/rollback",
            Some(TOKEN),
            json!({
                "title": "Still stale upgraded title",
                "expected_title_revision": 0,
                "expected_session_created_at": "2000-01-01T00:00:00Z",
                "expected_session_title": "First old daemon title"
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "body={body}");
    assert_eq!(body["error"]["code"], "title-state-conflict");

    let (status, body) = call(
        router(st),
        patch_json(
            "/sessions/rollback",
            Some(TOKEN),
            json!({
                "title": "Recovered upgraded title",
                "expected_title_revision": 0,
                "expected_session_created_at": "2000-01-01T00:00:00Z",
                "expected_session_title": "Second old daemon title"
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["data"]["session"]["title"], "Recovered upgraded title");
    assert_eq!(body["data"]["session"]["title_revision"], 1);
}

#[tokio::test]
async fn unique_session_prefix_supports_title_update_and_resume() {
    let tmp = tempfile::TempDir::new().unwrap();
    let state_dir = tmp.path().join("state");
    let cwd = tmp.path().join("repo");
    std::fs::create_dir_all(&cwd).unwrap();
    seed_resumable_session(
        &state_dir,
        "unique-prefix-session-long",
        "codex",
        "hs-unique-prefix-session-long",
        &cwd,
        &["resume", "resume-session-id"],
    );
    let st = state(&state_dir, Some(TOKEN), minimal_tmux(tmp.path()));

    let (status, body) = call(
        router(st.clone()),
        patch_json(
            "/sessions/unique-prefix",
            Some(TOKEN),
            json!({ "title": "Prefix title" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["data"]["session"]["id"], "unique-prefix-session-long");
    assert_eq!(body["data"]["session"]["title"], "Prefix title");

    let (status, body) = call(
        router(st),
        post_json("/sessions/unique-prefix/resume", Some(TOKEN), json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["data"]["session"]["id"], "unique-prefix-session-long");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn resume_transition_serializes_concurrent_title_mutation() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let state_dir = tmp.path().join("state");
    let cwd = tmp.path().join("repo");
    std::fs::create_dir_all(&cwd).expect("cwd");
    let cwd_arg = cwd.to_string_lossy().to_string();
    seed_resumable_session(
        &state_dir,
        "resume-title-race",
        "codex",
        "hs-resume-title-race",
        &cwd,
        &[
            "resume",
            "resume-session-id",
            "--cd",
            &cwd_arg,
            "--no-alt-screen",
        ],
    );
    let started = tmp.path().join("resume-started");
    let release = tmp.path().join("resume-release");
    let running = tmp.path().join("resume-running");
    let pane_stub = live_pane_stub_prelude(tmp.path());
    let tmux = executable(
        &tmp.path().join("tmux-resume-lock"),
        &format!(
            "#!/usr/bin/env sh\n{pane_stub}\ncase \"$1\" in\n  has-session) [ -f {running} ] && exit 0; printf '%s\\n' \"can't find session: fixture\" >&2; exit 1 ;;\n  new-session) : > {started}; while [ ! -f {release} ]; do sleep 0.01; done; : > {running}; heartbeat=''; slot=0; for arg in \"$@\"; do if [ \"$slot\" = 2 ]; then incarnation=\"$arg\"; break; elif [ \"$slot\" = 1 ]; then slot=2; else case \"$arg\" in */coordination/heartbeat) heartbeat=\"$arg\"; slot=1 ;; esac; fi; done; if [ -n \"$heartbeat\" ] && [ -n \"$incarnation\" ]; then mkdir -p \"$(dirname \"$heartbeat\")\"; printf '%s:%s\\n' \"$incarnation\" \"$(date +%s)\" > \"$heartbeat\"; chmod 600 \"$heartbeat\"; fi; start_live_pane || exit 1; printf '%s\\t%s\\t%s\\n' '$77' '%77' \"$NILS_TEST_PANE_PID\"; exit 0 ;;\n  display-message) set -- $(cat \"$NILS_TEST_PANE_PARENT/.pane-identity\"); printf '%s\\t%s\\t%s\\n' '$77' '%77' \"$1\"; exit 0 ;;\n  *) exit 0 ;;\nesac\n",
            started = shell_words::quote(&started.to_string_lossy()),
            release = shell_words::quote(&release.to_string_lossy()),
            running = shell_words::quote(&running.to_string_lossy()),
        ),
    );
    let context = CliContext {
        state_dir: state_dir.clone(),
        host: None,
    };
    let resume_task = {
        let context = context.clone();
        let tmux = tmux.clone();
        tokio::task::spawn_blocking(move || {
            resume_session_by_id(&context, "resume-title-race", &tmux)
        })
    };
    for _ in 0..200 {
        if started.is_file() {
            break;
        }
        if resume_task.is_finished() {
            let result = resume_task.await;
            panic!("resume exited before entering the tmux launch: {result:?}");
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(started.is_file(), "resume did not enter the tmux launch");
    let mut title_task = {
        let context = context.clone();
        let tmux = tmux.clone();
        tokio::task::spawn_blocking(move || {
            crate::update_session_title(
                &context,
                "resume-title-race",
                Some("Title after resume".to_string()),
                &tmux,
            )
        })
    };
    assert!(
        tokio::time::timeout(Duration::from_millis(100), &mut title_task)
            .await
            .is_err(),
        "title mutation must wait for the complete resume transaction"
    );
    let mut second_resume_task = {
        let context = context.clone();
        let tmux = tmux.clone();
        tokio::task::spawn_blocking(move || {
            resume_session_by_id(&context, "resume-title-race", &tmux)
        })
    };
    assert!(
        tokio::time::timeout(Duration::from_millis(100), &mut second_resume_task)
            .await
            .is_err(),
        "a second resume must wait for the first resume transaction"
    );
    std::fs::write(&release, b"").expect("release resume");
    resume_task
        .await
        .expect("resume task")
        .expect("resume transition");
    title_task
        .await
        .expect("title task")
        .expect("title mutation");
    second_resume_task
        .await
        .expect("second resume task")
        .expect("second resume result");

    let record = load_session_record(&context, "resume-title-race").expect("session record");
    assert_eq!(record.title.as_deref(), Some("Title after resume"));
    assert_eq!(
        record.runtime.as_ref().expect("runtime").generation,
        2,
        "title update must preserve the installed runtime generation"
    );
    assert_eq!(
        record
            .provider_resume
            .as_ref()
            .expect("provider resume")
            .session_id,
        "resume-session-id"
    );
    assert!(
        state_dir
            .join("sessions/resume-title-race/resume.json")
            .is_file()
    );
}

#[tokio::test]
async fn update_session_title_renames_live_claude_session() {
    let tmp = tempfile::TempDir::new().unwrap();
    let calls_log = tmp.path().join("tmux-calls.log");
    let pasted_log = tmp.path().join("tmux-pasted.log");
    let tmux = rename_probe_tmux(tmp.path(), &calls_log, &pasted_log, true);
    seed_session(tmp.path(), "steer", "claude", "hs-claude-steer");
    let st = state(tmp.path(), Some(TOKEN), tmux);

    let (status, body) = call(
        router(st.clone()),
        patch_json(
            "/sessions/steer",
            Some(TOKEN),
            json!({ "title": "Cleaned up title" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["data"]["session"]["title"], "Cleaned up title");

    let calls = std::fs::read_to_string(&calls_log).unwrap();
    assert!(
        calls.contains("paste-buffer -S -b steer-send"),
        "a live Claude rename must paste into the pane: {calls:?}"
    );
    assert!(
        calls.contains("send-keys -t hs-claude-steer:0.0 Enter"),
        "a live Claude rename must be submitted with Enter: {calls:?}"
    );
    let pasted = std::fs::read_to_string(&pasted_log).unwrap();
    assert_eq!(
        pasted.trim_end(),
        "/rename Cleaned up title",
        "the pasted rename command must carry the new title: {pasted:?}"
    );
}

#[tokio::test]
async fn live_claude_rename_skips_a_stale_title_revision() {
    let tmp = tempfile::TempDir::new().unwrap();
    let calls_log = tmp.path().join("tmux-calls.log");
    let pasted_log = tmp.path().join("tmux-pasted.log");
    let tmux = rename_probe_tmux(tmp.path(), &calls_log, &pasted_log, true);
    seed_session(
        tmp.path(),
        "stale-rename",
        "claude",
        "hs-claude-stale-rename",
    );
    let st = state(tmp.path(), Some(TOKEN), tmux.clone());
    let context = st.context.clone();
    let mut stale = crate::load_session_record(&context, "stale-rename").unwrap();
    stale.title = Some("Older title".to_string());
    stale.title_revision = 1;
    let current = crate::mutate_session_record(&context, "stale-rename", |record| {
        record.title = Some("Newer title".to_string());
        record.title_revision = 2;
        Ok(record.clone())
    })
    .unwrap();

    crate::rename_live_claude_session(&context, &stale, "Older title", &tmux).unwrap();
    assert!(
        !pasted_log.exists(),
        "a stale title revision must not reach the live Claude pane"
    );

    crate::rename_live_claude_session(&context, &current, "Newer title", &tmux).unwrap();
    let pasted = std::fs::read_to_string(&pasted_log).unwrap();
    assert_eq!(pasted.trim_end(), "/rename Newer title");
}

#[tokio::test]
async fn update_session_title_does_not_rename_non_claude_session() {
    let tmp = tempfile::TempDir::new().unwrap();
    let calls_log = tmp.path().join("tmux-calls.log");
    let pasted_log = tmp.path().join("tmux-pasted.log");
    let tmux = rename_probe_tmux(tmp.path(), &calls_log, &pasted_log, true);
    seed_session(tmp.path(), "steer", "codex", "hs-codex-steer");
    let st = state(tmp.path(), Some(TOKEN), tmux);

    let (status, body) = call(
        router(st.clone()),
        patch_json(
            "/sessions/steer",
            Some(TOKEN),
            json!({ "title": "Cleaned up title" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["data"]["session"]["title"], "Cleaned up title");

    let calls = std::fs::read_to_string(&calls_log).unwrap_or_default();
    assert!(
        !calls.contains("paste-buffer"),
        "Codex has no prompt-bar display name; it must not receive /rename: {calls:?}"
    );
    assert!(
        !pasted_log.exists(),
        "no rename text should be pasted for a non-Claude session"
    );
}

#[tokio::test]
async fn update_session_title_skips_rename_for_stopped_claude_session() {
    let tmp = tempfile::TempDir::new().unwrap();
    let calls_log = tmp.path().join("tmux-calls.log");
    let pasted_log = tmp.path().join("tmux-pasted.log");
    let tmux = rename_probe_tmux(tmp.path(), &calls_log, &pasted_log, false);
    seed_session(tmp.path(), "steer", "claude", "hs-claude-steer");
    let st = state(tmp.path(), Some(TOKEN), tmux);

    let (status, body) = call(
        router(st.clone()),
        patch_json(
            "/sessions/steer",
            Some(TOKEN),
            json!({ "title": "New title" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["data"]["session"]["title"], "New title");
    assert_eq!(body["data"]["session"]["status"], "stopped");

    let calls = std::fs::read_to_string(&calls_log).unwrap_or_default();
    assert!(
        !calls.contains("paste-buffer"),
        "a stopped session has no live pane to rename: {calls:?}"
    );
    assert!(
        !pasted_log.exists(),
        "no rename text should be pasted for a stopped session"
    );
}

#[tokio::test]
async fn update_session_title_clearing_to_null_does_not_rename() {
    let tmp = tempfile::TempDir::new().unwrap();
    let calls_log = tmp.path().join("tmux-calls.log");
    let pasted_log = tmp.path().join("tmux-pasted.log");
    let tmux = rename_probe_tmux(tmp.path(), &calls_log, &pasted_log, true);
    seed_session(tmp.path(), "steer", "claude", "hs-claude-steer");
    let st = state(tmp.path(), Some(TOKEN), tmux);

    let (status, _body) = call(
        router(st.clone()),
        patch_json("/sessions/steer", Some(TOKEN), json!({ "title": "First" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Clearing the title must not fire `/rename` with no argument (which would
    // make Claude auto-generate an unrelated name).
    let (status, body) = call(
        router(st.clone()),
        patch_json("/sessions/steer", Some(TOKEN), json!({ "title": null })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["data"]["session"]["title"], Value::Null);

    let pasted = std::fs::read_to_string(&pasted_log).unwrap();
    assert_eq!(
        pasted.matches("/rename").count(),
        1,
        "only the non-empty title should have been pushed as a rename: {pasted:?}"
    );
    assert!(
        pasted.contains("/rename First"),
        "the one rename must be the non-empty title: {pasted:?}"
    );
}

#[tokio::test]
async fn new_write_routes_require_token() {
    let tmp = tempfile::TempDir::new().unwrap();
    let tmux = minimal_tmux(tmp.path());
    seed_session(tmp.path(), "steer", "codex", "hs-codex-steer");
    let st = state(tmp.path(), Some(TOKEN), tmux);

    let (status, body) = call(
        router(st.clone()),
        patch_json("/sessions/steer", None, json!({ "title": "Nope" })),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"]["code"], "unauthorized");

    let (status, body) = call(
        router(st.clone()),
        put_json(
            "/sessions/steer/account",
            None,
            json!({ "account": "acct1" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"]["code"], "unauthorized");

    let (status, body) = call(
        router(st.clone()),
        post_bytes(
            "/sessions/steer/attachments?filename=secret.png",
            None,
            b"bytes",
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"]["code"], "unauthorized");
}

#[tokio::test]
async fn codex_accounts_route_is_authenticated_and_projects_no_credentials() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().unwrap();
    // Readiness resolves the private runtime dir; never use the host's.
    let runtime_dir = tempfile::Builder::new()
        .prefix("cx-")
        .tempdir_in("/tmp")
        .unwrap();
    fs::set_permissions(runtime_dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let _runtime_dir = EnvGuard::set(
        &lock,
        "XDG_RUNTIME_DIR",
        runtime_dir.path().to_str().unwrap(),
    );
    let codex = tmp.path().join("codex");
    fs::write(
            &codex,
            "#!/bin/sh\nif [ \"$1\" = --version ]; then printf '%s\\n' 'codex-cli 0.145.0'; exit 0; fi\nif [ \"$1\" = app-server ] && [ \"$2\" = --help ]; then printf '%s\\n' '  --listen <URL>  Supported values: stdio://, unix://PATH'; exit 0; fi\nexit 1\n",
        )
        .unwrap();
    fs::set_permissions(&codex, fs::Permissions::from_mode(0o700)).unwrap();
    let broker = tmp.path().join("broker");
    fs::write(
            &broker,
            r#"#!/bin/sh
printf '%s\n' '{"schema_version":"agent-session.codex-auth-broker.v1","accounts":[{"account":"acct1","label":"Acct1","plan":"team"}],"selection_strategies":["current_default","next_with_capacity","future_strategy"]}'
"#,
        )
        .unwrap();
    fs::set_permissions(&broker, fs::Permissions::from_mode(0o700)).unwrap();
    let argv = serde_json::to_string(&vec![broker.to_string_lossy().into_owned()]).unwrap();
    let _broker = EnvGuard::set(&lock, "AGENT_SESSION_CODEX_ACCOUNT_BROKER", &argv);
    let _codex_bin = EnvGuard::set(&lock, "AGENT_SESSION_CODEX_BIN", codex.to_str().unwrap());
    let st = state(tmp.path(), Some(TOKEN), minimal_tmux(tmp.path()));

    let (status, body) = call(router(st.clone()), get_auth("/codex/accounts", None)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"]["code"], "unauthorized");

    let (status, body) = call(router(st), get_auth("/codex/accounts", Some(TOKEN))).await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["data"]["machine"], MACHINE);
    assert_eq!(body["data"]["provider"], "codex");
    assert_eq!(body["data"]["accounts"][0]["account"], "acct1");
    assert_eq!(body["data"]["accounts"][0]["label"], "Acct1");
    assert_eq!(
        body["data"]["selection_strategies"],
        json!(["current_default", "next_with_capacity"])
    );
    assert_eq!(
        body["data"]["readiness"],
        json!({
            "schema_version": "agent-session.codex-account-readiness.v1",
            "supported": true,
            "provider_version": "0.145.0",
        })
    );
    let encoded = body.to_string();
    assert!(!encoded.contains("access_token"));
    assert!(!encoded.contains("chatgpt_account_id"));
}

struct ClaudeBrokerFixture {
    _broker: EnvGuard,
    log: PathBuf,
    accounts_root: PathBuf,
}

impl ClaudeBrokerFixture {
    fn calls(&self) -> String {
        fs::read_to_string(&self.log).unwrap_or_default()
    }

    fn config_dir(&self, account: &str) -> PathBuf {
        self.accounts_root.join(account)
    }
}

fn claude_account_dir(root: &Path, account: &str) -> PathBuf {
    let dir = root.join(account);
    fs::create_dir_all(&dir).unwrap();
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();
    let credentials = dir.join(".credentials.json");
    fs::write(&credentials, "{}").unwrap();
    fs::set_permissions(&credentials, fs::Permissions::from_mode(0o600)).unwrap();
    dir
}

/// A fake Claude account broker speaking `agent-session.account-broker.v2`.
/// It never touches a real Claude login: `materialize` returns a fixture
/// directory holding a placeholder `.credentials.json`. The listed
/// `gamma` account always fails to materialize.
fn claude_broker_fixture(lock: &GlobalStateLock, root: &Path) -> ClaudeBrokerFixture {
    let accounts_root = root.join("claude-accounts");
    for account in ["alpha", "beta"] {
        claude_account_dir(&accounts_root, account);
    }
    let log = root.join("claude-broker.log");
    let broker = executable(
        &root.join("claude-broker"),
        &format!(
            r#"#!/usr/bin/env sh
printf '%s\n' "$*" >> {log}
[ "$2" = --provider ] && [ "$3" = claude ] || exit 64
case "$1" in
  list) printf '%s\n' '{{"schema_version":"agent-session.account-broker.v2","provider":"claude","accounts":[{{"account":"alpha","label":"Alpha","plan":"max"}},{{"account":"beta"}},{{"account":"gamma"}}],"selection_strategies":["current_default"]}}' ;;
  select) printf '%s\n' '{{"schema_version":"agent-session.account-broker.v2","provider":"claude","account":"alpha"}}' ;;
  materialize) [ "$5" = gamma ] && exit 3; printf '{{"schema_version":"agent-session.account-broker.v2","provider":"claude","account":"%s","config_dir":"%s/%s"}}\n' "$5" {root} "$5" ;;
  *) exit 1 ;;
esac
"#,
            log = shell_words::quote(&log.to_string_lossy()),
            root = shell_words::quote(&accounts_root.to_string_lossy()),
        ),
    );
    let argv = serde_json::to_string(&vec![broker.to_string_lossy().into_owned()]).unwrap();
    ClaudeBrokerFixture {
        _broker: EnvGuard::set(lock, "AGENT_SESSION_CLAUDE_ACCOUNT_BROKER", &argv),
        log,
        accounts_root,
    }
}

fn seed_bound_claude_session(state_dir: &Path, id: &str, cwd: &Path, account: &str) {
    seed_resumable_session(
        state_dir,
        id,
        "claude",
        &format!("hs-claude-{id}"),
        cwd,
        &["--resume", "resume-session-id"],
    );
    let record_path = state_dir.join("sessions").join(id).join("session.json");
    let mut record: Value = serde_json::from_slice(&fs::read(&record_path).unwrap()).unwrap();
    record["claude_account_binding"] = json!({
        "schema_version": "agent-session.claude-account-binding.v1",
        "selected_account": account,
        "selection_source": "explicit",
        "revision": 1,
        "state": "bound",
        "applied_runtime_id": "never-launched-fixture",
        "updated_at": "2000-01-01T00:00:00Z",
    });
    fs::write(&record_path, serde_json::to_vec_pretty(&record).unwrap()).unwrap();
}

/// A tmux fake for a live Claude session whose pane is the pid written to
/// `pane_pid`; `running` exists while the fake session is up.
fn claude_live_tmux(dir: &Path, log: &Path, running: &Path, pane_pid: &Path) -> PathBuf {
    executable(
        &dir.join("tmux"),
        &format!(
            r#"#!/usr/bin/env sh
printf '%s\n' "$*" >> {log}
case "$1" in
  has-session)
    test -f {running} && exit 0
    printf '%s\n' "can't find session: fixture" >&2
    exit 1
    ;;
  new-session)
    : > {running}
    heartbeat=''
    slot=0
    for arg in "$@"; do
      if [ "$slot" = 2 ]; then incarnation="$arg"; break
      elif [ "$slot" = 1 ]; then slot=2
      else case "$arg" in */coordination/heartbeat) heartbeat="$arg"; slot=1 ;; esac
      fi
    done
    if [ -n "$heartbeat" ] && [ -n "$incarnation" ]; then
      mkdir -p "$(dirname "$heartbeat")"
      printf '%s:%s\n' "$incarnation" "$(date +%s)" > "$heartbeat"
      chmod 600 "$heartbeat"
    fi
    printf '%s\t%s\t%s\n' '$77' '%77' "$(cat {pane_pid})"
    ;;
  display-message)
    printf '%s\t%s\t%s\n' '$77' '%77' "$(cat {pane_pid})"
    ;;
  if-shell|kill-session)
    kill -KILL "$(cat {pane_pid})" 2>/dev/null || true
    rm -f {running}
    ;;
  *) exit 0 ;;
esac
"#,
            log = shell_words::quote(&log.to_string_lossy()),
            running = shell_words::quote(&running.to_string_lossy()),
            pane_pid = shell_words::quote(&pane_pid.to_string_lossy()),
        ),
    )
}

fn session_record_json(state_dir: &Path, id: &str) -> Value {
    serde_json::from_slice(
        &fs::read(state_dir.join("sessions").join(id).join("session.json")).unwrap(),
    )
    .unwrap()
}

#[tokio::test]
async fn create_claude_session_binds_the_explicit_account_and_pins_its_config_dir() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().unwrap();
    let cwd = tmp.path().join("repo");
    fs::create_dir_all(&cwd).unwrap();
    let broker = claude_broker_fixture(&lock, tmp.path());
    let log = tmp.path().join("tmux.log");
    let st = state(tmp.path(), Some(TOKEN), logging_tmux(tmp.path(), &log));

    let (status, body) = call(
        router(st.clone()),
        post_json(
            "/sessions",
            Some(TOKEN),
            json!({"agent":"claude","id":"claude-bound","cwd":cwd,"claude_account":"beta"}),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body={body}");
    let account = &body["data"]["session"]["claude_account"];
    assert_eq!(account["schema_version"], "agent-session.claude-account.v1");
    assert_eq!(account["supported"], true);
    assert_eq!(account["state"], "bound");
    assert_eq!(account["selected_account"], "beta");
    assert_eq!(account["selection_source"], "explicit");
    assert_eq!(account["revision"], 1);
    assert!(
        !body
            .to_string()
            .contains(broker.accounts_root.to_string_lossy().as_ref()),
        "the account directory path must never enter HTTP projections: {body}"
    );
    let calls = fs::read_to_string(&log).unwrap();
    assert!(
        calls.contains(&format!(
            "CLAUDE_CONFIG_DIR={}",
            broker.config_dir("beta").display()
        )),
        "the Claude process must run in the bound account directory: {calls:?}"
    );
    assert!(
        broker
            .calls()
            .contains("materialize --provider claude --account beta --format json"),
        "broker calls: {}",
        broker.calls()
    );
    let record = session_record_json(tmp.path(), "claude-bound");
    let binding = &record["claude_account_binding"];
    assert_eq!(
        binding["schema_version"],
        "agent-session.claude-account-binding.v1"
    );
    assert_eq!(binding["selected_account"], "beta");
    assert_eq!(binding["state"], "bound");
    assert_eq!(
        binding["applied_runtime_id"],
        record["runtime"]["launch_id"]
    );
}

#[tokio::test]
async fn create_claude_session_binds_the_current_default_at_launch() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().unwrap();
    let cwd = tmp.path().join("repo");
    fs::create_dir_all(&cwd).unwrap();
    let broker = claude_broker_fixture(&lock, tmp.path());
    let log = tmp.path().join("tmux.log");
    let st = state(tmp.path(), Some(TOKEN), logging_tmux(tmp.path(), &log));

    let (status, body) = call(
        router(st.clone()),
        post_json(
            "/sessions",
            Some(TOKEN),
            json!({"agent":"claude","id":"claude-default","cwd":cwd}),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body={body}");
    let account = &body["data"]["session"]["claude_account"];
    assert_eq!(account["selected_account"], "alpha");
    assert_eq!(account["selection_source"], "default_at_launch");
    let calls = fs::read_to_string(&log).unwrap();
    assert!(
        calls.contains(&format!(
            "CLAUDE_CONFIG_DIR={}",
            broker.config_dir("alpha").display()
        )),
        "{calls:?}"
    );
    assert!(
        broker
            .calls()
            .contains("select --provider claude --strategy current_default")
    );
}

#[tokio::test]
async fn create_claude_session_without_a_broker_keeps_the_host_login() {
    let lock = GlobalStateLock::new();
    let _no_broker = EnvGuard::remove(&lock, "AGENT_SESSION_CLAUDE_ACCOUNT_BROKER");
    let tmp = tempfile::TempDir::new().unwrap();
    let cwd = tmp.path().join("repo");
    fs::create_dir_all(&cwd).unwrap();
    let log = tmp.path().join("tmux.log");
    let st = state(tmp.path(), Some(TOKEN), logging_tmux(tmp.path(), &log));

    let (status, body) = call(
        router(st.clone()),
        post_json(
            "/sessions",
            Some(TOKEN),
            json!({"agent":"claude","id":"claude-host","cwd":cwd}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert!(
        body["data"]["session"].get("claude_account").is_none(),
        "{body}"
    );
    let calls = fs::read_to_string(&log).unwrap();
    assert!(!calls.contains("CLAUDE_CONFIG_DIR="), "{calls:?}");
    let record = session_record_json(tmp.path(), "claude-host");
    assert!(record.get("claude_account_binding").is_none());

    let (status, body) = call(
        router(st),
        post_json(
            "/sessions",
            Some(TOKEN),
            json!({"agent":"claude","cwd":cwd,"claude_account":"beta"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "body={body}");
    assert_eq!(body["error"]["code"], "claude-account-unsupported");
}

#[tokio::test]
async fn create_rejects_claude_account_for_other_agents_and_unsafe_account_dirs() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().unwrap();
    let cwd = tmp.path().join("repo");
    fs::create_dir_all(&cwd).unwrap();
    let broker = claude_broker_fixture(&lock, tmp.path());
    let log = tmp.path().join("tmux.log");
    let st = state(tmp.path(), Some(TOKEN), logging_tmux(tmp.path(), &log));

    let (status, body) = call(
        router(st.clone()),
        post_json(
            "/sessions",
            Some(TOKEN),
            json!({"agent":"codex","cwd":cwd,"claude_account":"beta"}),
        ),
    )
    .await;
    assert_ne!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["error"]["code"], "claude-account-agent-conflict");

    // A symlinked credentials file is refused: Claude Code opens it with
    // O_NOFOLLOW, and a link could redirect it to another login.
    let beta = broker.config_dir("beta");
    fs::remove_file(beta.join(".credentials.json")).unwrap();
    let elsewhere = tmp.path().join("elsewhere.json");
    fs::write(&elsewhere, "{}").unwrap();
    symlink(&elsewhere, beta.join(".credentials.json")).unwrap();
    let (status, body) = call(
        router(st),
        post_json(
            "/sessions",
            Some(TOKEN),
            json!({"agent":"claude","id":"claude-unsafe","cwd":cwd,"claude_account":"beta"}),
        ),
    )
    .await;
    assert_ne!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["error"]["code"], "claude-account-dir-unsafe");
    assert!(
        !log.exists() || !fs::read_to_string(&log).unwrap().contains("new-session"),
        "an unsafe account directory must never launch"
    );
}

#[tokio::test]
async fn resume_rematerializes_the_bound_claude_account() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().unwrap();
    let cwd = tmp.path().join("repo");
    fs::create_dir_all(&cwd).unwrap();
    let broker = claude_broker_fixture(&lock, tmp.path());
    seed_bound_claude_session(tmp.path(), "claude-resume", &cwd, "beta");
    let log = tmp.path().join("tmux.log");
    let st = state(tmp.path(), Some(TOKEN), resume_tmux(tmp.path(), &log));

    let (status, body) = call(
        router(st),
        post_json("/sessions/claude-resume/resume", Some(TOKEN), json!({})),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(
        body["data"]["session"]["claude_account"]["selected_account"],
        "beta"
    );
    let calls = fs::read_to_string(&log).unwrap();
    assert!(
        calls.contains(&format!(
            "CLAUDE_CONFIG_DIR={}",
            broker.config_dir("beta").display()
        )),
        "resume must keep the bound account: {calls:?}"
    );
    assert!(calls.contains("--resume resume-session-id"), "{calls:?}");
    assert!(
        broker
            .calls()
            .contains("materialize --provider claude --account beta")
    );
    let record = session_record_json(tmp.path(), "claude-resume");
    assert_eq!(
        record["claude_account_binding"]["applied_runtime_id"],
        record["runtime"]["launch_id"]
    );
    assert_eq!(record["claude_account_binding"]["revision"], 1);
}

#[tokio::test]
async fn resume_of_a_bound_claude_session_fails_closed_without_a_broker() {
    let lock = GlobalStateLock::new();
    let _no_broker = EnvGuard::remove(&lock, "AGENT_SESSION_CLAUDE_ACCOUNT_BROKER");
    let tmp = tempfile::TempDir::new().unwrap();
    let cwd = tmp.path().join("repo");
    fs::create_dir_all(&cwd).unwrap();
    seed_bound_claude_session(tmp.path(), "claude-orphan", &cwd, "beta");
    let log = tmp.path().join("tmux.log");
    let st = state(tmp.path(), Some(TOKEN), resume_tmux(tmp.path(), &log));

    let (status, body) = call(
        router(st),
        post_json("/sessions/claude-orphan/resume", Some(TOKEN), json!({})),
    )
    .await;

    assert_eq!(status, StatusCode::CONFLICT, "body={body}");
    assert_eq!(body["error"]["code"], "claude-account-unsupported");
    assert!(
        !fs::read_to_string(&log)
            .unwrap_or_default()
            .contains("new-session"),
        "a bound session must never fall back to the host login"
    );
}

#[tokio::test]
async fn account_list_routes_without_a_broker_are_409_unsupported() {
    let lock = GlobalStateLock::new();
    let _no_claude = EnvGuard::remove(&lock, "AGENT_SESSION_CLAUDE_ACCOUNT_BROKER");
    let _no_codex = EnvGuard::remove(&lock, "AGENT_SESSION_CODEX_ACCOUNT_BROKER");
    let tmp = tempfile::TempDir::new().unwrap();
    let st = state(tmp.path(), Some(TOKEN), minimal_tmux(tmp.path()));
    for (route, code) in [
        ("/codex/accounts", "codex-account-unsupported"),
        ("/claude/accounts", "claude-account-unsupported"),
    ] {
        let (status, body) = call(router(st.clone()), get_auth(route, Some(TOKEN))).await;
        assert_eq!(status, StatusCode::CONFLICT, "{route}: body={body}");
        assert_eq!(body["error"]["code"], code);
    }
}

#[tokio::test]
async fn claude_accounts_route_is_authenticated_and_projects_no_paths() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().unwrap();
    let broker = claude_broker_fixture(&lock, tmp.path());
    let st = state(tmp.path(), Some(TOKEN), minimal_tmux(tmp.path()));

    let (status, body) = call(router(st.clone()), get_auth("/claude/accounts", None)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"]["code"], "unauthorized");

    let (status, body) = call(router(st), get_auth("/claude/accounts", Some(TOKEN))).await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["data"]["machine"], MACHINE);
    assert_eq!(body["data"]["provider"], "claude");
    assert_eq!(
        body["data"]["accounts"],
        json!([
            {"account":"alpha","label":"Alpha","plan":"max"},
            {"account":"beta"},
            {"account":"gamma"}
        ])
    );
    assert_eq!(
        body["data"]["selection_strategies"],
        json!(["current_default"])
    );
    let encoded = body.to_string();
    assert!(!encoded.contains("config_dir"));
    assert!(!encoded.contains(broker.accounts_root.to_string_lossy().as_ref()));
}

#[tokio::test]
async fn sessions_capabilities_advertise_account_switch_only_with_a_broker() {
    let lock = GlobalStateLock::new();
    let _no_claude = EnvGuard::remove(&lock, "AGENT_SESSION_CLAUDE_ACCOUNT_BROKER");
    let _no_codex = EnvGuard::remove(&lock, "AGENT_SESSION_CODEX_ACCOUNT_BROKER");
    let tmp = tempfile::TempDir::new().unwrap();
    let st = state(tmp.path(), Some(TOKEN), minimal_tmux(tmp.path()));
    let (status, body) = call(router(st.clone()), get("/sessions")).await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["data"]["capabilities"]["claude_account_switch"], false);
    assert_eq!(body["data"]["capabilities"]["codex_account_switch"], false);

    let _broker = claude_broker_fixture(&lock, tmp.path());
    let (status, body) = call(router(st.clone()), get("/sessions")).await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["data"]["capabilities"]["claude_account_switch"], true);
    assert_eq!(body["data"]["capabilities"]["codex_account_switch"], false);

    let _codex_broker = codex_listing_broker(&lock, tmp.path(), &["acct1"]);
    let (status, body) = call(router(st), get("/sessions")).await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["data"]["capabilities"]["codex_account_switch"], true);
}

#[tokio::test]
async fn claude_account_switch_on_a_stopped_session_applies_at_the_next_resume() {
    let lock = GlobalStateLock::new();
    let _policy = EnvGuard::remove(&lock, crate::launch_env::ALLOWLIST_ENV);
    let tmp = tempfile::TempDir::new().unwrap();
    let cwd = tmp.path().join("repo");
    fs::create_dir_all(&cwd).unwrap();
    let broker = claude_broker_fixture(&lock, tmp.path());
    seed_bound_claude_session(tmp.path(), "claude-switch", &cwd, "alpha");
    let context = CliContext {
        state_dir: tmp.path().to_path_buf(),
        host: None,
    };
    let mut record = load_session_record(&context, "claude-switch").unwrap();
    record.extra.insert(
        "launch_env".into(),
        json!({"AGENT_RUNTIME_SUPPRESS_MEMORY":"1"}),
    );
    crate::write_session_record(&context, &record).unwrap();
    let log = tmp.path().join("tmux.log");
    let st = state(tmp.path(), Some(TOKEN), resume_tmux(tmp.path(), &log));

    let (status, body) = call(
        router(st.clone()),
        put_json(
            "/sessions/claude-switch/account",
            Some(TOKEN),
            json!({"account":"beta","expected_session_incarnation":"never-launched-fixture"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let account = &body["data"]["claude_account"];
    assert_eq!(account["selected_account"], "alpha");
    assert_eq!(account["next"]["account"], "beta");
    assert_eq!(account["next"]["state"], "queued");
    assert!(
        !fs::read_to_string(&log)
            .unwrap_or_default()
            .contains("new-session"),
        "queueing on a stopped session must not launch it"
    );

    let (status, body) = call(
        router(st),
        post_json("/sessions/claude-switch/resume", Some(TOKEN), json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(
        body["data"]["session"]["launch_env"],
        json!({"AGENT_RUNTIME_SUPPRESS_MEMORY":"1"})
    );
    let account = &body["data"]["session"]["claude_account"];
    assert_eq!(account["selected_account"], "beta");
    assert_eq!(account["selection_source"], "explicit");
    assert_eq!(account["revision"], 2);
    assert!(account.get("next").is_none(), "{account}");
    let calls = fs::read_to_string(&log).unwrap();
    assert!(calls.contains("-e AGENT_RUNTIME_SUPPRESS_MEMORY=1"));
    assert!(
        calls.contains(&format!(
            "CLAUDE_CONFIG_DIR={}",
            broker.config_dir("beta").display()
        )),
        "{calls:?}"
    );
    assert_serve_lifecycle(
        tmp.path(),
        "claude-switch",
        &[("account-switch", 1), ("resume", 1)],
    );
}

#[tokio::test]
async fn account_switch_statuses_match_across_providers() {
    let lock = GlobalStateLock::new();
    let no_claude = EnvGuard::remove(&lock, "AGENT_SESSION_CLAUDE_ACCOUNT_BROKER");
    let no_codex = EnvGuard::remove(&lock, "AGENT_SESSION_CODEX_ACCOUNT_BROKER");
    let tmp = tempfile::TempDir::new().unwrap();
    let cwd = tmp.path().join("repo");
    fs::create_dir_all(&cwd).unwrap();
    seed_bound_claude_session(tmp.path(), "claude-status", &cwd, "alpha");
    let codex_launch = seed_codex_app_server_session(tmp.path(), "codex-status");
    let log = tmp.path().join("tmux.log");
    let st = state(tmp.path(), Some(TOKEN), resume_tmux(tmp.path(), &log));
    let switch = |id: &str, incarnation: &str| {
        put_json(
            &format!("/sessions/{id}/account"),
            Some(TOKEN),
            json!({"account":"beta","expected_session_incarnation":incarnation}),
        )
    };

    // Without a broker, both providers report `*-account-unsupported` as 409.
    for (id, incarnation, code) in [
        (
            "claude-status",
            "never-launched-fixture",
            "claude-account-unsupported",
        ),
        (
            "codex-status",
            codex_launch.as_str(),
            "codex-account-unsupported",
        ),
    ] {
        let (status, body) = call(router(st.clone()), switch(id, incarnation)).await;
        assert_eq!(status, StatusCode::CONFLICT, "body={body}");
        assert_eq!(body["error"]["code"], code);
    }

    // A stale incarnation is a 409 conflict for both providers.
    drop((no_claude, no_codex));
    let _claude = claude_broker_fixture(&lock, tmp.path());
    let _codex = codex_listing_broker(&lock, tmp.path(), &["beta"]);
    for (id, code) in [
        (
            "claude-status",
            "claude-account-session-incarnation-conflict",
        ),
        ("codex-status", "codex-account-session-incarnation-conflict"),
    ] {
        let (status, body) = call(router(st.clone()), switch(id, "stale-incarnation")).await;
        assert_eq!(status, StatusCode::CONFLICT, "body={body}");
        assert_eq!(body["error"]["code"], code);
    }
}

/// `agent-session account switch` runs the same Claude switch as the serve
/// route: busy queues, an unknown or unpreparable account is refused before
/// anything stops, and an idle switch preflights and then stops the runtime
/// for the resume. An omitted incarnation fences on the current runtime.
#[tokio::test]
async fn account_cli_claude_switch_queues_busy_refuses_and_restarts_idle() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().unwrap();
    let cwd = tmp.path().join("repo");
    fs::create_dir_all(&cwd).unwrap();
    let broker = claude_broker_fixture(&lock, tmp.path());
    let log = tmp.path().join("tmux.log");
    let running = tmp.path().join("running");
    let pane_pid = tmp.path().join("pane.pid");
    let tmux = claude_live_tmux(tmp.path(), &log, &running, &pane_pid);
    let st = state(tmp.path(), Some(TOKEN), tmux.clone());
    let first_pane = TestProcessGroup::spawn();
    fs::write(&pane_pid, first_pane.pid().to_string()).unwrap();
    let _first_pane = first_pane.into_background_reaper();

    let (status, body) = call(
        router(st.clone()),
        post_json(
            "/sessions",
            Some(TOKEN),
            json!({"agent":"claude","id":"claude-cli","cwd":cwd,"claude_account":"alpha"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let record = load_session_record(&st.context, "claude-cli").unwrap();
    let first_launch = record.runtime.as_ref().unwrap().launch_id.clone();
    let provider_session_id = record.provider_resume.as_ref().unwrap().session_id.clone();
    let turn = |event_id: &str, kind: &str| {
        serde_json::from_value(json!({
            "schema_version": crate::activity::TURN_EVENT_VERSION,
            "event_id": event_id,
            "runtime_id": first_launch,
            "provider": "claude",
            "provider_session_id": provider_session_id,
            "provider_turn_id": "turn-claude-cli-switch",
            "kind": kind,
            "confidence": "authoritative"
        }))
        .unwrap()
    };
    let launches = || {
        fs::read_to_string(&log)
            .unwrap()
            .lines()
            .filter(|line| line.starts_with("new-session"))
            .count()
    };
    let switch = |account: &str, expected: Option<&str>| {
        crate::session_account::switch(&st.context, "claude-cli", account, expected, &tmux)
            .map(|result| serde_json::to_value(result).unwrap())
    };

    let shown =
        serde_json::to_value(crate::session_account::show(&st.context, "claude-cli").unwrap())
            .unwrap();
    assert_eq!(shown["id"], "claude-cli");
    assert_eq!(shown["agent"], "claude");
    assert_eq!(shown["session_incarnation"], first_launch.as_str());
    assert_eq!(shown["claude_account"]["selected_account"], "alpha");
    assert!(shown.get("codex_account").is_none(), "{shown}");

    // Busy: the switch stays queued and nothing restarts.
    crate::activity::ingest_event(&st.context, &record.id, turn("turn-start", "turn_started"))
        .unwrap();
    let queued = switch("beta", None).unwrap();
    assert_eq!(queued["session_incarnation"], first_launch.as_str());
    assert_eq!(queued["claude_account"]["selected_account"], "alpha");
    assert_eq!(queued["claude_account"]["next"]["account"], "beta");
    assert_eq!(queued["claude_account"]["next"]["state"], "queued");
    assert_eq!(launches(), 1, "a busy turn must not restart");
    let shown =
        serde_json::to_value(crate::session_account::show(&st.context, "claude-cli").unwrap())
            .unwrap();
    assert_eq!(shown["claude_account"]["next"]["account"], "beta");

    // Typed errors match the serve route.
    let stale = switch("beta", Some("stale-incarnation")).unwrap_err();
    assert_eq!(stale.code(), "claude-account-session-incarnation-conflict");
    let unknown = switch("delta", Some(&first_launch)).unwrap_err();
    assert_eq!(unknown.code(), "claude-account-unknown");

    // Idle refusal: the preflight fails before anything is stopped.
    crate::activity::ingest_event(&st.context, &record.id, turn("turn-done", "turn_completed"))
        .unwrap();
    let before_refusal = session_record_json(tmp.path(), "claude-cli");
    let refused = switch("gamma", None).unwrap_err();
    assert_eq!(refused.code(), "claude-account-switch-refused");
    assert_eq!(
        refused.details().unwrap()["cause"],
        "claude-account-broker-rejected"
    );
    let calls = fs::read_to_string(&log).unwrap();
    assert!(
        !calls.contains("kill-session") && !calls.contains("if-shell"),
        "a refused preflight must never stop the runtime: {calls:?}"
    );
    assert!(running.exists(), "the session must still be running");
    let after_refusal = session_record_json(tmp.path(), "claude-cli");
    assert_eq!(
        after_refusal["claude_account_binding"],
        before_refusal["claude_account_binding"]
    );
    assert_eq!(
        after_refusal["claude_account_next"],
        before_refusal["claude_account_next"]
    );

    // Run from inside the session's own tmux session, an idle switch must
    // not stop the runtime that hosts the caller: it only queues.
    {
        let _inside = EnvGuard::set(&lock, "AGENT_SESSION_ID", "claude-cli");
        let queued = switch("beta", None).unwrap();
        assert_eq!(queued["claude_account"]["selected_account"], "alpha");
        assert_eq!(queued["claude_account"]["next"]["account"], "beta");
        let calls = fs::read_to_string(&log).unwrap();
        assert!(
            !calls.contains("kill-session") && !calls.contains("if-shell"),
            "a switch from inside the session must never stop it: {calls:?}"
        );
        assert_eq!(launches(), 1);
    }

    // Idle: preflight once, then the same verified stop as the serve route.
    // As there, this fixture pane has no dedicated control group, so the
    // stop refuses and the intent stays queued for the next resume.
    let stop = switch("beta", None).unwrap_err();
    assert_eq!(stop.code(), "session-termination-failed");
    assert_eq!(launches(), 1, "a refused stop must not relaunch");
    assert_eq!(
        broker
            .calls()
            .matches("materialize --provider claude --account beta")
            .count(),
        1,
        "the idle switch preflights the next account once, before the stop"
    );
    let persisted = load_session_record(&st.context, "claude-cli").unwrap();
    assert!(crate::claude_account::has_queued_next(&persisted));
}

/// Once a switch has stopped the runtime, a failed resume must say the
/// session is stopped with the account queued and how to recover, instead
/// of surfacing the bare resume error (sympoies/nils-cli#2040).
#[test]
fn claude_switch_resume_failure_reports_the_stopped_session_and_queued_account() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().unwrap();
    let cwd = tmp.path().join("repo");
    fs::create_dir_all(&cwd).unwrap();
    let _broker = claude_broker_fixture(&lock, tmp.path());
    seed_bound_claude_session(tmp.path(), "claude-stranded", &cwd, "alpha");
    let tmux = executable(
        &tmp.path().join("tmux-launch-fails"),
        "#!/bin/sh\ncase \"$1\" in\n  has-session) printf '%s\\n' \"can't find session: fixture\" >&2; exit 1 ;;\n  new-session) printf '%s\\n' 'launch failed' >&2; exit 1 ;;\n  *) exit 0 ;;\nesac\n",
    );
    let context = CliContext {
        state_dir: tmp.path().to_path_buf(),
        host: None,
    };
    let mut record = load_session_record(&context, "claude-stranded").unwrap();
    crate::claude_account::queue_next(&mut record, "beta").unwrap();
    crate::write_session_record(&context, &record).unwrap();

    let error =
        crate::session_account::resume_after_switch_stop(&context, "claude-stranded", &tmux)
            .unwrap_err();

    assert_eq!(error.code(), "claude-account-switch-resume-failed");
    let details = error.details().unwrap();
    assert_eq!(details["id"], "claude-stranded");
    assert_eq!(details["session_state"], "stopped");
    assert_eq!(details["next_account"], "beta");
    assert!(
        details["cause"]
            .as_str()
            .is_some_and(|cause| !cause.is_empty())
    );
    assert_eq!(
        details["recovery"], "agent-session resume claude-stranded",
        "{details}"
    );
    let persisted = load_session_record(&context, "claude-stranded").unwrap();
    let view =
        serde_json::to_value(crate::session_view(&context, &persisted, None, Some(&tmux))).unwrap();
    assert_eq!(view["claude_account"]["next"]["state"], "failed");
    assert_eq!(
        view["claude_account"]["next"]["failure_reason"],
        details["cause"]
    );
    assert_eq!(view["claude_account"]["state"], "failed");
    assert_eq!(view["startup"]["state"], "failed");
    assert_eq!(
        view["startup"]["failure_code"],
        "claude-account-switch-resume-failed"
    );
    assert!(!crate::claude_account::has_queued_next(&persisted));
    let retry_tmux = resume_tmux(tmp.path(), &tmp.path().join("retry.log"));
    let retried = crate::resume_session_locked(&context, persisted, &retry_tmux).unwrap();
    let retried = serde_json::to_value(retried.session).unwrap();
    assert_eq!(retried["claude_account"]["selected_account"], "beta");
    assert!(retried["claude_account"].get("next").is_none());
    assert_ne!(retried["startup"]["state"], "failed");
}

#[path = "tests/account_switch.rs"]
mod account_switch;
#[path = "tests/sync_waits.rs"]
mod sync_waits;

#[test]
fn legacy_broker_resume_uses_prior_persisted_identity() {
    use crate::coordination::broker::test_support;
    let lock = GlobalStateLock::new();
    for agent in ["claude", "codex"] {
        let tmp = tempfile::TempDir::new().unwrap();
        let cwd = tmp.path().join("repo");
        fs::create_dir_all(&cwd).unwrap();
        let _broker = claude_broker_fixture(&lock, tmp.path());
        let context = CliContext {
            state_dir: tmp.path().to_path_buf(),
            host: None,
        };
        if agent == "claude" {
            seed_bound_claude_session(tmp.path(), "prior-resume", &cwd, "alpha");
        } else {
            seed_resumable_session(
                tmp.path(),
                "prior-resume",
                "codex",
                "fixture-runtime",
                &cwd,
                &[
                    "resume",
                    "resume-session-id",
                    "--cd",
                    cwd.to_str().unwrap(),
                    "--no-alt-screen",
                ],
            );
        }
        let mut record = load_session_record(&context, "prior-resume").unwrap();
        let old = record.runtime.as_ref().unwrap().launch_id.clone();
        let mut identity =
            test_support::process_group_identity(test_support::exited_process_group());
        identity["launch_id"] = json!(old);
        record.extra.remove("tmux_runtime_never_launched");
        record
            .extra
            .insert("delete_tmux_identity".to_string(), identity.clone());
        crate::write_session_record(&context, &record).unwrap();
        test_support::seed_live_broker(&context, &record.id, &old, identity);
        test_support::make_legacy_broker(&context, &record.id, false);
        let tmux = resume_tmux(tmp.path(), &tmp.path().join("tmux.log"));
        let result = crate::resume_session_locked(&context, record, &tmux)
            .expect("a stopped older broker must permit exact provider resume");
        assert_ne!(result.session_incarnation.as_deref(), Some(old.as_str()));
        assert_eq!(result.session.status, "running");
    }
}

#[cfg(target_os = "linux")]
#[test]
fn generation_two_legacy_broker_resume_ignores_recycled_numeric_selector() {
    use crate::coordination::broker::test_support;
    let _lock = GlobalStateLock::new();
    for prior_boot in [true, false] {
        let tmp = tempfile::TempDir::new().unwrap();
        let cwd = tmp.path().join("repo");
        fs::create_dir_all(&cwd).unwrap();
        let context = CliContext {
            state_dir: tmp.path().to_path_buf(),
            host: None,
        };
        seed_resumable_session(
            tmp.path(),
            "broker-reboot",
            "codex",
            "managed-runtime",
            &cwd,
            &[
                "resume",
                "resume-session-id",
                "--cd",
                cwd.to_str().unwrap(),
                "--no-alt-screen",
            ],
        );
        let mut record = load_session_record(&context, "broker-reboot").unwrap();
        let old = record.runtime.as_ref().unwrap().launch_id.clone();
        record.runtime.as_mut().unwrap().generation = 2;
        let mut identity =
            test_support::process_group_identity(test_support::exited_process_group());
        identity["launch_id"] = json!(old);
        identity["session_id"] = json!("$7");
        if prior_boot {
            let boot = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
            assert_ne!(crate::linux_boot_id().unwrap(), boot);
            identity["pid_namespace"]["boot_id"] = json!(boot);
        }
        record.extra.remove("tmux_runtime_never_launched");
        record
            .extra
            .insert("delete_tmux_identity".to_string(), identity.clone());
        crate::write_session_record(&context, &record).unwrap();
        test_support::seed_live_broker(&context, &record.id, &old, identity);
        test_support::make_legacy_broker(&context, &record.id, false);
        let registry_path = tmp.path().join("coordination/registry.json");
        let mut registry: Value =
            serde_json::from_slice(&fs::read(&registry_path).unwrap()).unwrap();
        registry["brokers"][&record.id]["generation"] = json!(2);
        assert!(registry["brokers"][&record.id]["runtime_identity"].is_null());
        fs::write(
            &registry_path,
            serde_json::to_vec_pretty(&registry).unwrap(),
        )
        .unwrap();

        let log = tmp.path().join("tmux.log");
        let tmux = resume_tmux(tmp.path(), &log);
        let script = fs::read_to_string(&tmux).unwrap().replace(
            "has-session)",
            "has-session) if [ \"$3\" = '$7' ]; then exit 0; fi;",
        );
        fs::write(&tmux, script).unwrap();
        assert_eq!(
            crate::verified_tmux_status_with_timeout(&tmux, "$7", Duration::from_secs(1)),
            "running"
        );
        assert_eq!(
            crate::verified_tmux_status_with_timeout(
                &tmux,
                "=managed-runtime",
                Duration::from_secs(1)
            ),
            "stopped"
        );
        let provider_resume = serde_json::to_value(&record.provider_resume).unwrap();
        let result = crate::resume_session_locked(&context, record, &tmux)
            .unwrap_or_else(|error| panic!("prior_boot={prior_boot}: {error:?}"));
        assert_eq!(result.session.status, "running");
        let resumed = load_session_record(&context, "broker-reboot").unwrap();
        assert_eq!(resumed.runtime.as_ref().unwrap().generation, 3);
        assert_ne!(resumed.runtime.as_ref().unwrap().launch_id, old);
        assert_eq!(
            serde_json::to_value(&resumed.provider_resume).unwrap(),
            provider_resume
        );
        assert_eq!(
            crate::verified_tmux_status_with_timeout(&tmux, "$7", Duration::from_secs(1)),
            "running",
            "the unrelated numeric target must remain live"
        );
        assert!(
            !fs::read_to_string(&log)
                .unwrap()
                .lines()
                .any(|call| call.starts_with("kill-session"))
        );
    }
}

#[test]
fn legacy_resume_allows_tmux_to_reuse_the_stopped_internal_session_id() {
    use crate::coordination::broker::test_support;
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().unwrap();
    let cwd = tmp.path().join("repo");
    fs::create_dir_all(&cwd).unwrap();
    let _broker = claude_broker_fixture(&lock, tmp.path());
    let context = CliContext {
        state_dir: tmp.path().to_path_buf(),
        host: None,
    };
    seed_bound_claude_session(tmp.path(), "reused-runtime", &cwd, "alpha");
    let mut record = load_session_record(&context, "reused-runtime").unwrap();
    let old = record.runtime.as_ref().unwrap().launch_id.clone();
    let mut identity = test_support::process_group_identity(test_support::exited_process_group());
    identity["launch_id"] = json!(old);
    identity["session_id"] = json!("$77");
    identity["pane_id"] = json!("%77");
    record.extra.remove("tmux_runtime_never_launched");
    record
        .extra
        .insert("delete_tmux_identity".to_string(), identity.clone());
    crate::write_session_record(&context, &record).unwrap();
    test_support::seed_live_broker(&context, &record.id, &old, identity);
    test_support::make_legacy_broker(&context, &record.id, false);
    let tmux = resume_tmux(tmp.path(), &tmp.path().join("tmux.log"));
    let script = fs::read_to_string(&tmux)
        .unwrap()
        .replace(
            "has-session)",
            "has-session) if [ -f \"$NILS_TEST_PANE_PARENT/.replacement-live\" ]; then exit 0; fi;",
        )
        .replace(
            "new-session)",
            "new-session) touch \"$NILS_TEST_PANE_PARENT/.replacement-live\";",
        );
    fs::write(&tmux, script).unwrap();
    let result = crate::resume_session_locked(&context, record, &tmux)
        .expect("the new runtime may reuse the stopped internal tmux ID");
    assert_eq!(result.session.status, "running");
}

#[test]
fn claude_stopped_broker_switch_and_failed_switch_retry_preserve_the_conversation() {
    use crate::coordination::broker::test_support;
    for failed_switch_retry in [false, true] {
        let lock = GlobalStateLock::new();
        let tmp = tempfile::TempDir::new().unwrap();
        let cwd = tmp.path().join("repo");
        fs::create_dir_all(&cwd).unwrap();
        let broker = claude_broker_fixture(&lock, tmp.path());
        let context = CliContext {
            state_dir: tmp.path().to_path_buf(),
            host: None,
        };
        seed_bound_claude_session(tmp.path(), "broker-switch", &cwd, "alpha");
        let mut record = load_session_record(&context, "broker-switch").unwrap();
        let old = record.runtime.as_ref().unwrap().launch_id.clone();
        let provider = serde_json::to_value(record.provider_resume.as_ref().unwrap()).unwrap();
        let mut identity =
            test_support::process_group_identity(test_support::verified_absent_process_group());
        identity["launch_id"] = json!(old);
        #[cfg(target_os = "macos")]
        {
            identity["macos_boot_id"] = json!(crate::capture_macos_boot_id().unwrap());
        }
        record.extra.remove("tmux_runtime_never_launched");
        record
            .extra
            .insert("delete_tmux_identity".to_string(), identity.clone());
        crate::claude_account::queue_next(&mut record, "beta").unwrap();
        if failed_switch_retry {
            let stopped = record.clone();
            crate::claude_account::mark_next_resume_failed(
                &mut record,
                &stopped,
                "session-incarnation-conflict",
            )
            .unwrap();
        }
        crate::write_session_record(&context, &record).unwrap();
        test_support::seed_live_broker(&context, &record.id, &old, identity);
        if failed_switch_retry {
            crate::coordination::revoke(&context, &record).unwrap();
            fs::remove_file(crate::coordination::heartbeat_path(
                &context.state_dir,
                &record.id,
            ))
            .unwrap();
        }
        let tmux = resume_tmux(tmp.path(), &tmp.path().join("tmux.log"));
        let script = fs::read_to_string(&tmux).unwrap().replace(
                r#"if [ -n "$heartbeat" ] && [ -n "$incarnation" ]; then"#,
                r#"if [ -n "$heartbeat" ] && [ -n "$incarnation" ]; then gate="$(dirname "$heartbeat")/broker-provisioned"; (i=0; while [ ! -f "$gate" ] && [ "$i" -lt 1000 ]; do i=$((i + 1)); sleep 0.01; done; printf '%s:%s\n' "$incarnation" "$(date +%s)" > "$heartbeat"; chmod 600 "$heartbeat") >/dev/null 2>&1 & fi; if false; then"#,
            );
        fs::write(&tmux, script).unwrap();
        if failed_switch_retry {
            crate::resume_session_locked(&context, record, &tmux)
                .expect("documented retry must replace the stopped broker");
        } else {
            crate::session_account::resume_after_switch_stop(&context, &record.id, &tmux)
                .expect("verified account switch stop must retire its broker");
        }
        let resumed = load_session_record(&context, "broker-switch").unwrap();
        assert_eq!(
            serde_json::to_value(resumed.provider_resume.as_ref().unwrap()).unwrap(),
            provider
        );
        assert_ne!(resumed.runtime.as_ref().unwrap().launch_id, old);
        let view = serde_json::to_value(crate::claude_account::view_for_record(&resumed)).unwrap();
        assert_eq!(view["selected_account"], "beta");
        assert!(view.get("next").is_none());
        assert!(
            fs::read_to_string(tmp.path().join("tmux.log"))
                .unwrap()
                .contains(&format!(
                    "CLAUDE_CONFIG_DIR={}",
                    broker.config_dir("beta").display()
                ))
        );
    }
}

/// The killed runtime's broker still has a fresh heartbeat when the switch
/// resumes. Without retiring that incarnation first, the resume is refused
/// as "the prior coordination incarnation is still live" and the session is
/// left stopped (sympoies/nils-cli#2040).
#[cfg(target_os = "linux")]
#[test]
fn claude_switch_resume_retires_the_stopped_incarnation_before_resuming() {
    use crate::coordination::broker::test_support;
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().unwrap();
    let cwd = tmp.path().join("repo");
    fs::create_dir_all(&cwd).unwrap();
    let broker = claude_broker_fixture(&lock, tmp.path());
    seed_bound_claude_session(tmp.path(), "claude-retire", &cwd, "alpha");
    let log = tmp.path().join("tmux.log");
    // Like the real held launch, start the new heartbeat only once the
    // broker gate opens, so the old incarnation's beat is still the one on
    // disk when the resume provisions coordination.
    let tmux = resume_tmux(tmp.path(), &log);
    let script = fs::read_to_string(&tmux).unwrap().replace(
            r#"if [ -n "$heartbeat" ] && [ -n "$incarnation" ]; then"#,
            r#"if [ -n "$heartbeat" ] && [ -n "$incarnation" ]; then gate="$(dirname "$heartbeat")/broker-provisioned"; (i=0; while [ ! -f "$gate" ] && [ "$i" -lt 1000 ]; do i=$((i + 1)); sleep 0.01; done; printf '%s:%s\n' "$incarnation" "$(date +%s)" > "$heartbeat"; chmod 600 "$heartbeat") >/dev/null 2>&1 & fi; if false; then"#,
        );
    assert!(script.contains("broker-provisioned"));
    fs::write(&tmux, script).unwrap();
    let context = CliContext {
        state_dir: tmp.path().to_path_buf(),
        host: None,
    };
    let mut record = load_session_record(&context, "claude-retire").unwrap();
    crate::claude_account::queue_next(&mut record, "beta").unwrap();
    crate::write_session_record(&context, &record).unwrap();
    test_support::seed_live_broker(
        &context,
        "claude-retire",
        "never-launched-fixture",
        test_support::process_group_identity(test_support::exited_process_group()),
    );

    let (session_incarnation, view) =
        crate::session_account::resume_after_switch_stop(&context, "claude-retire", &tmux)
            .expect("the switch resumes under the queued account");

    assert!(
        session_incarnation
            .as_deref()
            .is_some_and(|incarnation| incarnation != "never-launched-fixture"),
        "{session_incarnation:?}"
    );
    let view = view.unwrap();
    assert_eq!(view.selected_account.as_deref(), Some("beta"));
    let calls = fs::read_to_string(&log).unwrap();
    assert!(
        calls.contains(&format!(
            "CLAUDE_CONFIG_DIR={}",
            broker.config_dir("beta").display()
        )),
        "{calls:?}"
    );
}

/// `agent-session account switch` on a Codex session has no in-process
/// control connection, so it takes the serve route's durable queue path:
/// the account is bound for the next prompt, applied by the daemon's control
/// connection at the idle boundary, and the prompt is fenced until then.
#[tokio::test]
async fn account_cli_codex_switch_binds_for_the_next_prompt() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().unwrap();
    let broker = codex_listing_broker(&lock, tmp.path(), &["acct1", "omega"]);
    let launch_id = seed_codex_app_server_session(tmp.path(), "codex-cli");
    let tmux = minimal_tmux(tmp.path());
    let context = CliContext {
        state_dir: tmp.path().to_path_buf(),
        host: None,
    };
    let mut record = load_session_record(&context, "codex-cli").unwrap();
    crate::codex_account::set_initial_binding(&mut record, Some("acct1")).unwrap();
    crate::write_session_record(&context, &record).unwrap();
    crate::codex_account::finish_binding(&context, &record.id, &launch_id, "acct1", 1, Ok(()))
        .unwrap();
    let switch = |account: &str, expected: Option<&str>| {
        crate::session_account::switch(&context, "codex-cli", account, expected, &tmux)
            .map(|result| serde_json::to_value(result).unwrap())
    };

    let shown =
        serde_json::to_value(crate::session_account::show(&context, "codex-cli").unwrap()).unwrap();
    assert_eq!(shown["agent"], "codex");
    assert_eq!(shown["session_incarnation"], launch_id.as_str());
    assert_eq!(shown["codex_account"]["selected_account"], "acct1");
    assert!(shown.get("claude_account").is_none(), "{shown}");

    let stale = switch("omega", Some("stale-incarnation")).unwrap_err();
    assert_eq!(stale.code(), "codex-account-session-incarnation-conflict");
    let unlisted = switch("delta", None).unwrap_err();
    assert_eq!(unlisted.code(), "codex-account-unknown");

    let bound = switch("omega", None).unwrap();
    assert_eq!(bound["session_incarnation"], launch_id.as_str());
    assert_eq!(bound["codex_account"]["selected_account"], "acct1");
    assert_eq!(bound["codex_account"]["next"]["account"], "omega");
    assert_eq!(bound["codex_account"]["next"]["state"], "queued");
    let persisted = load_session_record(&context, "codex-cli").unwrap();
    assert_eq!(
        crate::codex_account::pending_next_apply(&persisted)
            .unwrap()
            .map(|(account, _)| account),
        Some("omega".to_string()),
        "the daemon's control connection applies it at the idle boundary"
    );
    assert!(
        crate::codex_account::ensure_input_allowed(&persisted).is_err(),
        "the next prompt is fenced until the queued account is applied"
    );

    // Re-selecting the bound account cancels the queued switch, as in serve.
    let cancelled = switch("acct1", Some(&launch_id)).unwrap();
    assert!(
        cancelled["codex_account"].get("next").is_none(),
        "{cancelled}"
    );

    drop(broker);
    let _no_broker = EnvGuard::remove(&lock, "AGENT_SESSION_CODEX_ACCOUNT_BROKER");
    let unsupported = switch("omega", None).unwrap_err();
    assert_eq!(unsupported.code(), "codex-account-unsupported");
}

#[tokio::test]
async fn upload_attachment_streams_before_request_completion() {
    let tmp = tempfile::TempDir::new().unwrap();
    let tmux = minimal_tmux(tmp.path());
    seed_session(tmp.path(), "stream", "codex", "hs-codex-stream");
    let st = state(tmp.path(), Some(TOKEN), tmux);
    let (chunk_tx, chunk_rx) = mpsc::channel::<Result<Bytes, io::Error>>(2);
    let request_stream = futures_util::stream::unfold(chunk_rx, |mut receiver| async move {
        receiver.recv().await.map(|chunk| (chunk, receiver))
    });
    let request = Request::builder()
        .method("POST")
        .uri("/sessions/stream/attachments?filename=recording.mp4")
        .header(AUTHORIZATION, format!("Bearer {TOKEN}"))
        .header(CONTENT_TYPE, "video/mp4")
        .body(Body::from_stream(request_stream))
        .expect("streaming attachment request");
    let upload = tokio::spawn(call(router(st), request));

    chunk_tx
        .send(Ok(Bytes::from_static(b"first chunk")))
        .await
        .expect("send first chunk");
    let attachments = tmp.path().join("sessions/stream/attachments");
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if let Ok(entries) = std::fs::read_dir(&attachments) {
                let entries = entries.collect::<Result<Vec<_>, _>>().unwrap();
                if entries.len() == 1
                    && entries[0].file_name().to_string_lossy().ends_with(".part")
                    && std::fs::read(entries[0].path()).ok().as_deref() == Some(b"first chunk")
                {
                    break;
                }
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the first request chunk must reach a temporary file before completion");

    chunk_tx
        .send(Ok(Bytes::from_static(b" and second chunk")))
        .await
        .expect("send second chunk");
    drop(chunk_tx);
    let (status, body) = upload.await.expect("upload task");
    assert_eq!(status, StatusCode::OK, "body={body}");
    let path = PathBuf::from(body["data"]["attachment"]["path"].as_str().unwrap());
    assert_eq!(
        std::fs::read(path).unwrap(),
        b"first chunk and second chunk"
    );
    assert!(std::fs::read_dir(attachments).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".part")
    }));
}

#[tokio::test]
async fn upload_attachment_cleans_up_after_request_body_error() {
    let tmp = tempfile::TempDir::new().unwrap();
    let tmux = minimal_tmux(tmp.path());
    seed_session(tmp.path(), "broken", "codex", "hs-codex-broken");
    let st = state(tmp.path(), Some(TOKEN), tmux);
    let request_stream = futures_util::stream::iter([
        Ok(Bytes::from_static(b"partial recording")),
        Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "client disconnected",
        )),
    ]);
    let request = Request::builder()
        .method("POST")
        .uri("/sessions/broken/attachments?filename=broken.mp4")
        .header(AUTHORIZATION, format!("Bearer {TOKEN}"))
        .header(CONTENT_TYPE, "video/mp4")
        .body(Body::from_stream(request_stream))
        .unwrap();

    let (status, body) = call(router(st), request).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body={body}");
    assert_eq!(body["error"]["code"], "attachment-read-failed");
    let attachments = tmp.path().join("sessions/broken/attachments");
    assert_eq!(std::fs::read_dir(attachments).unwrap().count(), 0);
}

#[tokio::test]
async fn upload_attachment_cleans_up_when_handler_is_cancelled() {
    let tmp = tempfile::TempDir::new().unwrap();
    let tmux = minimal_tmux(tmp.path());
    seed_session(tmp.path(), "cancelled", "codex", "hs-codex-cancelled");
    let st = state(tmp.path(), Some(TOKEN), tmux);
    let (chunk_tx, chunk_rx) = mpsc::channel::<Result<Bytes, io::Error>>(1);
    let request_stream = futures_util::stream::unfold(chunk_rx, |mut receiver| async move {
        receiver.recv().await.map(|chunk| (chunk, receiver))
    });
    let request = Request::builder()
        .method("POST")
        .uri("/sessions/cancelled/attachments?filename=cancelled.mp4")
        .header(AUTHORIZATION, format!("Bearer {TOKEN}"))
        .body(Body::from_stream(request_stream))
        .unwrap();
    let upload = tokio::spawn(call(router(st), request));

    chunk_tx
        .send(Ok(Bytes::from_static(b"partial recording")))
        .await
        .unwrap();
    let attachments = tmp.path().join("sessions/cancelled/attachments");
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if std::fs::read_dir(&attachments)
                .ok()
                .is_some_and(|mut entries| entries.next().is_some())
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the temporary attachment must exist before cancellation");

    upload.abort();
    let _ = upload.await;
    assert_eq!(std::fs::read_dir(attachments).unwrap().count(), 0);
}

#[tokio::test]
async fn upload_attachment_authenticates_and_resolves_session_before_polling_body() {
    let tmp = tempfile::TempDir::new().unwrap();
    let tmux = minimal_tmux(tmp.path());
    seed_session(tmp.path(), "ordered", "codex", "hs-codex-ordered");
    let st = state(tmp.path(), Some(TOKEN), tmux);

    for (uri, token, expected) in [
        (
            "/sessions/ordered/attachments?filename=unauthorized.bin",
            None,
            StatusCode::UNAUTHORIZED,
        ),
        (
            "/sessions/missing/attachments?filename=missing.bin",
            Some(TOKEN),
            StatusCode::NOT_FOUND,
        ),
    ] {
        let request_stream = futures_util::stream::once(async {
            panic!("the body must not be polled before request admission");
            #[allow(unreachable_code)]
            Ok::<Bytes, io::Error>(Bytes::new())
        });
        let mut request = Request::builder().method("POST").uri(uri);
        if let Some(token) = token {
            request = request.header(AUTHORIZATION, format!("Bearer {token}"));
        }
        let (status, _) = call(
            router(st.clone()),
            request.body(Body::from_stream(request_stream)).unwrap(),
        )
        .await;
        assert_eq!(status, expected);
    }
    assert!(!tmp.path().join("sessions/ordered/attachments").exists());
}

#[tokio::test]
async fn upload_attachment_rejects_declared_oversize_before_polling_body() {
    let tmp = tempfile::TempDir::new().unwrap();
    let tmux = minimal_tmux(tmp.path());
    seed_session(tmp.path(), "declared", "codex", "hs-codex-declared");
    let mut st = state(tmp.path(), Some(TOKEN), tmux);
    Arc::get_mut(&mut st).unwrap().max_attachment_bytes = 64;
    let request_stream = futures_util::stream::once(async {
        panic!("an oversized declared body must not be polled");
        #[allow(unreachable_code)]
        Ok::<Bytes, io::Error>(Bytes::new())
    });
    let request = Request::builder()
        .method("POST")
        .uri("/sessions/declared/attachments?filename=oversize.mp4")
        .header(AUTHORIZATION, format!("Bearer {TOKEN}"))
        .header(CONTENT_TYPE, "video/mp4")
        .header(CONTENT_LENGTH, "65")
        .body(Body::from_stream(request_stream))
        .unwrap();

    let (status, body) = call(router(st), request).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE, "body={body}");
    assert_eq!(body["error"]["code"], "attachment-too-large");
    assert!(!tmp.path().join("sessions/declared/attachments").exists());
}

#[tokio::test]
async fn upload_attachment_accepts_observed_body_at_exact_limit() {
    let tmp = tempfile::TempDir::new().unwrap();
    let tmux = minimal_tmux(tmp.path());
    seed_session(tmp.path(), "exact", "codex", "hs-codex-exact");
    let mut st = state(tmp.path(), Some(TOKEN), tmux);
    Arc::get_mut(&mut st).unwrap().max_attachment_bytes = 64;
    let payload = Bytes::from(vec![7_u8; 64]);
    let request_stream = futures_util::stream::once({
        let payload = payload.clone();
        async move { Ok::<Bytes, io::Error>(payload) }
    });
    let request = Request::builder()
        .method("POST")
        .uri("/sessions/exact/attachments?filename=exact.bin")
        .header(AUTHORIZATION, format!("Bearer {TOKEN}"))
        .body(Body::from_stream(request_stream))
        .unwrap();

    let (status, body) = call(router(st), request).await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["data"]["attachment"]["bytes"], 64);
    let path = PathBuf::from(body["data"]["attachment"]["path"].as_str().unwrap());
    assert_eq!(std::fs::read(path).unwrap(), payload);
    assert!(
        std::fs::read_dir(tmp.path().join("sessions/exact/attachments"))
            .unwrap()
            .all(|entry| !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".part"))
    );
}

#[tokio::test]
async fn upload_attachment_writes_private_session_file_with_safe_name() {
    let tmp = tempfile::TempDir::new().unwrap();
    let tmux = minimal_tmux(tmp.path());
    seed_session(tmp.path(), "steer", "codex", "hs-codex-steer");
    let mut st = state(tmp.path(), Some(TOKEN), tmux);
    Arc::get_mut(&mut st).unwrap().max_attachment_bytes = 64;
    let payload = b"not actually a png";

    let (status, body) = call(
        router(st.clone()),
        post_bytes(
            "/sessions/steer/attachments?filename=..%2FScreen%20Shot.png",
            Some(TOKEN),
            payload,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["schema_version"], "cli.agent-session.serve.v1");
    let attachment = &body["data"]["attachment"];
    assert_eq!(attachment["bytes"], payload.len());
    assert_eq!(attachment["filename"], "Screen_Shot.png");
    let path = PathBuf::from(attachment["path"].as_str().expect("attachment path"));
    assert_eq!(std::fs::read(&path).unwrap(), payload);
    assert!(path.starts_with(tmp.path().join("sessions/steer/attachments")));
    let mode = std::fs::metadata(&path).unwrap().permissions().mode();
    assert_eq!(mode & 0o077, 0, "attachment file must be private");

    let (status, second) = call(
        router(st.clone()),
        post_bytes(
            "/sessions/steer/attachments?filename=Screen%20Shot.png",
            Some(TOKEN),
            b"second payload",
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={second}");
    let second_path = PathBuf::from(second["data"]["attachment"]["path"].as_str().unwrap());
    assert_ne!(second_path, path);
    assert_eq!(std::fs::read(&path).unwrap(), payload);
    assert_eq!(std::fs::read(&second_path).unwrap(), b"second payload");

    let (status, body) = call(
        router(st.clone()),
        post_bytes(
            "/sessions/ghost/attachments?filename=ghost.png",
            Some(TOKEN),
            b"ghost",
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["code"], "session-not-found");

    let before_oversize = std::fs::read_dir(tmp.path().join("sessions/steer/attachments"))
        .unwrap()
        .count();
    let oversize = vec![0u8; 65];
    let (status, body) = call(
        router(st.clone()),
        post_bytes(
            "/sessions/steer/attachments?filename=oversize.bin",
            Some(TOKEN),
            &oversize,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(body["error"]["code"], "attachment-too-large");
    let after_oversize = std::fs::read_dir(tmp.path().join("sessions/steer/attachments"))
        .unwrap()
        .count();
    assert_eq!(after_oversize, before_oversize);
}

#[tokio::test]
async fn workdir_search_filters_default_project_and_config_roots() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().unwrap();
    let home = tmp.path().join("home");
    let project_match = home.join("Project/sympoies/agent-console");
    let second_match = home.join("Project/agent-tools");
    let too_deep_match = home.join("Project/a/b/c/d/e/agent-deep");
    let config_match = home.join(".config/zsh");
    let outside = home.join("Downloads/agent-console-copy");
    std::fs::create_dir_all(project_match.join(".git")).unwrap();
    std::fs::create_dir_all(&second_match).unwrap();
    std::fs::create_dir_all(&too_deep_match).unwrap();
    std::fs::create_dir_all(&config_match).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    let _home = EnvGuard::set(&lock, "HOME", home.to_str().unwrap());

    let st = state(tmp.path(), Some(TOKEN), PathBuf::from("tmux"));
    let (status, body) = call(router(st.clone()), get("/workdirs?q=agent&limit=20")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"]["code"], "unauthorized");

    let (status, body) = call(
        router(st.clone()),
        get_auth("/workdirs?q=agent&limit=20", Some(TOKEN)),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["schema_version"], "cli.agent-session.serve.v1");
    let workdirs = body["data"]["workdirs"].as_array().expect("workdirs array");
    assert!(
        workdirs
            .iter()
            .any(|item| item["path"] == project_match.to_string_lossy().as_ref()),
        "Project match missing: {workdirs:?}"
    );
    assert!(
        !workdirs
            .iter()
            .any(|item| item["path"] == outside.to_string_lossy().as_ref()),
        "outside root must not be returned: {workdirs:?}"
    );
    assert!(
        !workdirs
            .iter()
            .any(|item| item["path"] == too_deep_match.to_string_lossy().as_ref()),
        "matches beyond max depth must not be returned: {workdirs:?}"
    );

    let (status, limited) = call(
        router(st.clone()),
        get_auth("/workdirs?q=agent&limit=1", Some(TOKEN)),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={limited}");
    assert_eq!(
        limited["data"]["workdirs"]
            .as_array()
            .expect("workdirs")
            .len(),
        1
    );

    let (status, body) = call(
        router(st.clone()),
        get_auth("/workdirs?q=zsh&limit=20", Some(TOKEN)),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let workdirs = body["data"]["workdirs"].as_array().expect("workdirs array");
    assert!(
        workdirs
            .iter()
            .any(|item| item["path"] == config_match.to_string_lossy().as_ref()),
        ".config match missing: {workdirs:?}"
    );
}

#[tokio::test]
async fn repo_remote_url_resolves_github_and_gitlab_remotes() {
    let tmp = tempfile::TempDir::new().unwrap();
    let github_repo = tmp.path().join("home/Project/sympoies/agent-console");
    let gitlab_repo = tmp.path().join("home/Project/acme/backend");
    init_git_remote(&github_repo, "git@github.com:sympoies/agent-console.git");
    init_git_remote(
        &gitlab_repo,
        "ssh://git@gitlab.example.com:2222/group/sub/project.git",
    );

    let st = state(tmp.path(), Some(TOKEN), PathBuf::from("tmux"));
    let unauthorized = format!("/repos/remote-url?cwd={}", github_repo.to_string_lossy());
    let (status, body) = call(router(st.clone()), get(&unauthorized)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"]["code"], "unauthorized");

    let github_path = format!("/repos/remote-url?cwd={}", github_repo.to_string_lossy());
    let (status, body) = call(router(st.clone()), get_auth(&github_path, Some(TOKEN))).await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["data"]["machine"], "test-machine");
    assert_eq!(
        body["data"]["url"],
        "https://github.com/sympoies/agent-console"
    );

    let gitlab_path = format!("/repos/remote-url?cwd={}", gitlab_repo.to_string_lossy());
    let (status, body) = call(router(st.clone()), get_auth(&gitlab_path, Some(TOKEN))).await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(
        body["data"]["url"],
        "https://gitlab.example.com/group/sub/project"
    );

    let missing = tmp.path().join("home/Project/no-remote");
    std::fs::create_dir_all(&missing).unwrap();
    let missing_path = format!("/repos/remote-url?cwd={}", missing.to_string_lossy());
    let (status, body) = call(router(st), get_auth(&missing_path, Some(TOKEN))).await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert!(body["data"]["url"].is_null());
}

#[tokio::test]
async fn workdir_search_can_return_git_repos_without_linked_worktrees() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().unwrap();
    let home = tmp.path().join("home");
    let project_repo = home.join("Project/sympoies/agent-console");
    let linked_worktree = home.join("Project/sympoies/wt-agent-console-21");
    let non_git_dir = home.join("Project/sympoies/notes");
    let config_repo = home.join(".config/zsh");
    std::fs::create_dir_all(project_repo.join(".git")).unwrap();
    std::fs::create_dir_all(&linked_worktree).unwrap();
    std::fs::write(
        linked_worktree.join(".git"),
        "gitdir: ../agent-console/.git/worktrees/wt-agent-console-21\n",
    )
    .unwrap();
    std::fs::create_dir_all(&non_git_dir).unwrap();
    std::fs::create_dir_all(config_repo.join(".git")).unwrap();
    let _home = EnvGuard::set(&lock, "HOME", home.to_str().unwrap());

    let st = state(tmp.path(), Some(TOKEN), PathBuf::from("tmux"));
    let (status, body) = call(
        router(st.clone()),
        get_auth(
            "/workdirs?q=&limit=20&git_only=true&exclude_worktrees=true",
            Some(TOKEN),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let workdirs = body["data"]["workdirs"].as_array().expect("workdirs array");
    let paths: Vec<&str> = workdirs
        .iter()
        .filter_map(|item| item["path"].as_str())
        .collect();
    assert!(
        paths.contains(&project_repo.to_string_lossy().as_ref()),
        "primary project repo missing: {workdirs:?}"
    );
    assert!(
        paths.contains(&config_repo.to_string_lossy().as_ref()),
        "config repo missing: {workdirs:?}"
    );
    assert!(
        !paths.contains(&non_git_dir.to_string_lossy().as_ref()),
        "non-git dir must not be returned: {workdirs:?}"
    );
    assert!(
        !paths.contains(&linked_worktree.to_string_lossy().as_ref()),
        "linked worktree must not be returned: {workdirs:?}"
    );
    assert!(
        workdirs.iter().all(|item| item["is_git_repo"] == true),
        "all returned rows must be git repos: {workdirs:?}"
    );
    assert!(
        workdirs.iter().all(|item| item.get("last_used").is_some()),
        "last_used field must be present for every row: {workdirs:?}"
    );
}

#[tokio::test]
async fn create_records_workdir_usage_for_recent_first_search() {
    let lock = GlobalStateLock::new();
    let _unpinned = crate::agent_bin::unpinned_provider_env(&lock);
    let tmp = tempfile::TempDir::new().unwrap();
    let home = tmp.path().join("home");
    let alpha_repo = home.join("Project/sympoies/alpha");
    let beta_repo = home.join("Project/sympoies/beta");
    for repo in [&alpha_repo, &beta_repo] {
        std::fs::create_dir_all(repo).unwrap();
        assert!(
            std::process::Command::new("/usr/bin/git")
                .args(["init", "--quiet"])
                .current_dir(repo)
                .status()
                .unwrap()
                .success()
        );
    }
    let _home = EnvGuard::set(&lock, "HOME", home.to_str().unwrap());
    let tmux = minimal_tmux(tmp.path());
    let st = state(tmp.path(), Some(TOKEN), tmux);

    let (status, body) = call(
        router(st.clone()),
        post_json(
            "/sessions",
            Some(TOKEN),
            json!({
                "agent": "codex",
                "id": "uses-beta",
                "cwd": beta_repo.to_string_lossy(),
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");

    let (status, body) = call(
        router(st.clone()),
        get_auth(
            "/workdirs?q=&limit=20&git_only=true&exclude_worktrees=true",
            Some(TOKEN),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let workdirs = body["data"]["workdirs"].as_array().expect("workdirs array");
    assert_eq!(
        workdirs
            .first()
            .and_then(|item| item["path"].as_str())
            .unwrap_or_default(),
        beta_repo.to_string_lossy().as_ref(),
        "created cwd must rank first: {workdirs:?}"
    );
    assert!(
        workdirs
            .first()
            .and_then(|item| item["last_used"].as_str())
            .is_some_and(|value| !value.is_empty()),
        "created cwd must expose last_used: {workdirs:?}"
    );
    let alpha = workdirs
        .iter()
        .find(|item| item["path"] == alpha_repo.to_string_lossy().as_ref())
        .expect("unused repo row");
    assert_eq!(alpha["last_used"], Value::Null);
}

#[tokio::test]
async fn workdir_search_rejects_symlink_roots() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().unwrap();
    let home = tmp.path().join("home");
    let outside = tmp.path().join("outside");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(outside.join("agent-secret")).unwrap();
    symlink(&outside, home.join("Project")).unwrap();
    let _home = EnvGuard::set(&lock, "HOME", home.to_str().unwrap());

    let st = state(tmp.path(), Some(TOKEN), PathBuf::from("tmux"));
    let (status, body) = call(
        router(st.clone()),
        get_auth("/workdirs?q=agent&limit=20", Some(TOKEN)),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let workdirs = body["data"]["workdirs"].as_array().expect("workdirs array");
    assert!(
        workdirs.is_empty(),
        "symlink roots must not expose outside directories: {workdirs:?}"
    );
}

#[test]
fn serve_default_bind_is_loopback() {
    use crate::cli::{Cli, Command};
    use clap::Parser;
    let cli = Cli::try_parse_from(["agent-session", "serve"]).unwrap();
    let Command::Serve(args) = cli.command else {
        panic!("expected serve command");
    };
    let addr: SocketAddr = args.bind.parse().unwrap();
    assert!(addr.ip().is_loopback(), "default bind must be loopback");
}

#[test]
fn serve_token_stdin_conflicts_with_token_without_leaking_token() {
    use crate::cli::Cli;
    use clap::Parser;

    let err = Cli::try_parse_from([
        "agent-session",
        "serve",
        "--token",
        "secret-from-argv",
        "--token-stdin",
    ])
    .unwrap_err();
    let message = err.to_string();

    assert!(message.contains("--token"));
    assert!(message.contains("--token-stdin"));
    assert!(
        !message.contains("secret-from-argv"),
        "parse error leaked token material: {message}"
    );
}

#[test]
fn token_stdin_reads_single_trimmed_token() {
    assert_eq!(
        read_token_from_stdin("  stdin-token\n".as_bytes()).unwrap(),
        "stdin-token"
    );
}

#[test]
fn token_stdin_rejects_empty_input_without_leaking_material() {
    let err = read_token_from_stdin("   \n".as_bytes()).unwrap_err();
    let message = err.to_string();

    assert!(message.contains("empty"));
    assert!(!message.contains("\\n"));
}

#[test]
fn token_stdin_rejects_multiple_lines_without_leaking_material() {
    let err = read_token_from_stdin("first-token\nsecond-token\n".as_bytes()).unwrap_err();
    let message = err.to_string();

    assert!(message.contains("exactly one token"));
    assert!(!message.contains("first-token"));
    assert!(!message.contains("second-token"));
}

#[test]
fn token_stdin_rejects_oversized_input() {
    let oversized = "a".repeat((MAX_STDIN_TOKEN_BYTES + 1) as usize);
    let err = read_token_from_stdin(oversized.as_bytes()).unwrap_err();
    assert!(err.to_string().contains("8192 bytes"));
}

#[tokio::test]
async fn create_rejects_unknown_agent() {
    let tmp = tempfile::TempDir::new().unwrap();
    let st = state(tmp.path(), Some(TOKEN), PathBuf::from("tmux"));
    let (status, body) = call(
        router(st.clone()),
        post_json("/sessions", Some(TOKEN), json!({"agent": "banana"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "invalid-agent");
}

#[tokio::test]
async fn glance_reads_open_with_machine() {
    let tmp = tempfile::TempDir::new().unwrap();
    let tmux = minimal_tmux(tmp.path());
    seed_session(tmp.path(), "look", "codex", "hs-codex-look");
    let st = state(tmp.path(), Some(TOKEN), tmux);
    // No auth header: glance is a read, open on loopback.
    let (status, body) = call(router(st.clone()), get("/sessions/look/glance")).await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["schema_version"], "cli.agent-session.serve.v1");
    assert_eq!(body["data"]["machine"], MACHINE);
    assert!(body["data"]["glance"].is_object());
    assert!(body["data"]["glance"].get("startup").is_none());
}

#[tokio::test]
async fn buffer_returns_tmux_show_buffer_text_open_on_reads() {
    let tmp = tempfile::TempDir::new().unwrap();
    let tmux = minimal_tmux(tmp.path());
    seed_session(tmp.path(), "look", "claude", "hs-claude-look");
    let st = state(tmp.path(), Some(TOKEN), tmux);
    // No auth header: buffer is a read, open on loopback like glance.
    let (status, body) = call(router(st.clone()), get("/sessions/look/buffer")).await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["schema_version"], "cli.agent-session.serve.v1");
    assert_eq!(body["data"]["machine"], MACHINE);
    assert_eq!(body["data"]["text"], "buffered selection\n");
}

#[tokio::test]
async fn buffer_unknown_session_is_not_found() {
    let tmp = tempfile::TempDir::new().unwrap();
    let tmux = minimal_tmux(tmp.path());
    let st = state(tmp.path(), Some(TOKEN), tmux);
    let (status, body) = call(router(st.clone()), get("/sessions/ghost/buffer")).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "body={body}");
    assert_eq!(body["error"]["code"], "session-not-found");
}

#[tokio::test]
async fn buffer_reports_empty_when_no_buffer_set() {
    // A fresh tmux server has no paste buffer yet: `show-buffer` exits non-zero
    // ("no buffers"), which must surface as an empty selection, not an error.
    let tmp = tempfile::TempDir::new().unwrap();
    let bin = tmp.path().join("tmux");
    std::fs::write(
            &bin,
            "#!/usr/bin/env sh\ncase \"$1\" in\n  show-buffer) echo 'no buffers' >&2; exit 1 ;;\n  *) exit 0 ;;\nesac\n",
        )
        .unwrap();
    let mut perms = std::fs::metadata(&bin).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&bin, perms).unwrap();
    seed_session(tmp.path(), "look", "claude", "hs-claude-look");
    let st = state(tmp.path(), Some(TOKEN), bin);
    let (status, body) = call(router(st.clone()), get("/sessions/look/buffer")).await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["data"]["text"], "");
}

#[tokio::test]
async fn glance_preserves_provider_resume_metadata() {
    let tmp = tempfile::TempDir::new().unwrap();
    let cwd = tmp.path().join("repo");
    std::fs::create_dir_all(&cwd).unwrap();
    seed_resumable_session(
        tmp.path(),
        "recover",
        "claude",
        "hs-claude-recover",
        &cwd,
        &["--resume", "resume-session-id"],
    );
    add_provider_resume_extra(tmp.path(), "recover");
    let st = state(tmp.path(), Some(TOKEN), minimal_tmux(tmp.path()));

    let (status, body) = call(router(st.clone()), get("/sessions/recover/glance")).await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let glance = &body["data"]["glance"];
    assert_eq!(glance["provider_resume"]["provider"], "claude");
    assert_eq!(
        glance["provider_resume"]["resume_args"],
        json!(["--resume", "resume-session-id"])
    );
    assert!(glance["provider_resume"].get("storage_only").is_none());
}

#[tokio::test]
async fn delete_missing_session_maps_to_404() {
    let tmp = tempfile::TempDir::new().unwrap();
    let st = state(tmp.path(), Some(TOKEN), PathBuf::from("tmux"));
    let del = Request::builder()
        .method("DELETE")
        .uri("/sessions/ghost")
        .header("authorization", format!("Bearer {TOKEN}"))
        .body(Body::empty())
        .unwrap();
    let (status, body) = call(router(st.clone()), del).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["code"], "session-not-found");
}

#[tokio::test]
async fn delete_kill_failure_returns_structured_error_and_retains_state() {
    let tmp = tempfile::TempDir::new().unwrap();

    for agent in ["codex", "claude"] {
        let id = format!("failed-delete-{agent}");
        let tmux_session = format!("hs-{agent}-failed-delete");
        seed_session_with_runtime(tmp.path(), &id, agent, &tmux_session);
        #[cfg(not(target_os = "linux"))]
        let pane = TestProcessGroup::spawn();
        let calls = tmp.path().join(format!("failed-delete-{agent}.calls"));
        #[cfg(not(target_os = "linux"))]
        let tmux = failing_delete_tmux(tmp.path(), tmp.path(), &id, pane.pid(), &calls);
        // The fake tmux is a shell script whose launch cannot guarantee
        // fitting the termination probe's bounded startup budget under
        // load, so pin the probe outcome it models (the pane is live)
        // and let the real kill step fail as the test intends
        // (sympoies/nils-cli#2131).
        #[cfg(not(target_os = "linux"))]
        let _probe = crate::tmux_probe_fixture::install(
            &tmux,
            crate::tmux_probe_fixture::running_identity(
                &tmux_session,
                "%88",
                pane.pid() as libc::pid_t,
                Some(pane.process_group_id),
                None,
            ),
        );
        #[cfg(target_os = "linux")]
        let tmux = tmp.path().join(format!("unavailable-probe-{agent}"));
        #[cfg(target_os = "linux")]
        let _probe = crate::tmux_probe_fixture::install(
            &tmux,
            crate::tmux_probe_fixture::Probe::IdentityUnavailable,
        );
        let session_dir = tmp.path().join("sessions").join(&id);
        let runtime_metadata = session_dir.join("provider-runtime.json");
        std::fs::write(&runtime_metadata, format!("runtime metadata for {agent}")).unwrap();
        let st = state(tmp.path(), Some(TOKEN), tmux.clone());
        let request = Request::builder()
            .method("DELETE")
            .uri(format!("/sessions/{id}"))
            .header("authorization", format!("Bearer {TOKEN}"))
            .body(Body::empty())
            .unwrap();

        let (status, body) = call(router(st), request).await;

        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body={body}");
        assert_eq!(body["schema_version"], "cli.agent-session.serve.v1");
        assert_eq!(body["ok"], false);
        assert_eq!(body["error"]["code"], "session-termination-failed");
        assert_eq!(body["error"]["details"]["id"], id);
        assert_eq!(body["error"]["details"]["tmux_session"], tmux_session);
        #[cfg(target_os = "linux")]
        {
            assert_eq!(
                body["error"]["details"]["reason"],
                "runtime-identity-unavailable"
            );
            assert_eq!(
                body["error"]["details"]["action"],
                "manual-runtime-verification-required"
            );
            assert!(
                !std::fs::read_to_string(&calls)
                    .unwrap_or_default()
                    .lines()
                    .any(|line| line.starts_with("if-shell")),
                "Linux deletion must not mutate tmux before pinning a dedicated cgroup"
            );
        }
        #[cfg(not(target_os = "linux"))]
        {
            assert_eq!(body["error"]["details"]["reason"], "kill-failed");
            assert_eq!(body["error"]["details"]["action"], "retry-delete");
        }
        assert!(session_dir.exists(), "{agent} state must remain retryable");
        assert!(
            runtime_metadata.exists(),
            "{agent} runtime metadata must remain"
        );
    }
}

#[tokio::test]
async fn delete_stopped_pre_upgrade_session_returns_non_retryable_manual_action() {
    let tmp = tempfile::TempDir::new().unwrap();
    let id = "pre-upgrade-stopped";
    seed_session(tmp.path(), id, "codex", "hs-codex-pre-upgrade-stopped");
    let tmux = executable(
        &tmp.path().join("pre-upgrade-stopped-tmux"),
        "#!/usr/bin/env sh\ncase \"$1\" in\n  display-message|has-session) printf \"%s\\n\" \"can't find session: pre-upgrade\" >&2; exit 1 ;;\n  *) exit 42 ;;\nesac\n",
    );
    // The fake tmux is a shell script whose launch cannot guarantee fitting
    // the termination probe's bounded startup budget under load, so pin the
    // probe outcome it models: the session is gone (sympoies/nils-cli#2131).
    let _probe =
        crate::tmux_probe_fixture::install(&tmux, crate::tmux_probe_fixture::Probe::Stopped);
    let session_dir = tmp.path().join("sessions").join(id);
    let st = state(tmp.path(), Some(TOKEN), tmux);
    let request = Request::builder()
        .method("DELETE")
        .uri(format!("/sessions/{id}"))
        .header("authorization", format!("Bearer {TOKEN}"))
        .body(Body::empty())
        .unwrap();

    let (status, body) = call(router(st), request).await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body={body}");
    assert_eq!(body["error"]["code"], "session-termination-failed");
    assert_eq!(
        body["error"]["details"]["reason"],
        "runtime-identity-unavailable"
    );
    assert_eq!(body["error"]["details"]["retryable"], false);
    assert_eq!(
        body["error"]["details"]["action"],
        "manual-runtime-verification-required"
    );
    assert!(
        session_dir.exists(),
        "pre-upgrade metadata must remain fail-closed"
    );
}

#[tokio::test]
async fn delete_accepts_current_generation_never_launched_proof() {
    let tmp = tempfile::TempDir::new().unwrap();
    let calls = tmp.path().join("never-launched-delete-calls");
    let tmux = executable(
        &tmp.path().join("never-launched-delete-tmux"),
        &format!(
            "#!/usr/bin/env sh\nprintf '%s\\n' \"$*\" >> {}\ncase \"$1\" in\n  display-message|has-session) printf \"%s\\n\" \"can't find session: never-launched\" >&2; exit 1 ;;\n  *) exit 42 ;;\nesac\n",
            shell_words::quote(&calls.to_string_lossy())
        ),
    );

    let _probe =
        crate::tmux_probe_fixture::install(&tmux, crate::tmux_probe_fixture::Probe::Stopped);

    for agent in ["codex", "claude"] {
        let id = format!("never-launched-delete-{agent}");
        let tmux_session = format!("hs-{agent}-never-launched-delete");
        seed_session_with_runtime(tmp.path(), &id, agent, &tmux_session);
        let session_dir = tmp.path().join("sessions").join(&id);
        let record_path = session_dir.join("session.json");
        let mut record: Value =
            serde_json::from_slice(&std::fs::read(&record_path).unwrap()).unwrap();
        record["tmux_runtime_never_launched"] = record["runtime"]["launch_id"].clone();
        std::fs::write(&record_path, serde_json::to_vec_pretty(&record).unwrap()).unwrap();
        let runtime_metadata = session_dir.join("provider-runtime.json");
        std::fs::write(&runtime_metadata, format!("runtime metadata for {agent}")).unwrap();

        let st = state(tmp.path(), Some(TOKEN), tmux.clone());
        let request = Request::builder()
            .method("DELETE")
            .uri(format!("/sessions/{id}"))
            .header("authorization", format!("Bearer {TOKEN}"))
            .body(Body::empty())
            .unwrap();
        let (status, body) = call(router(st), request).await;

        assert_eq!(status, StatusCode::OK, "body={body}");
        assert_eq!(body["ok"], true);
        assert_eq!(body["data"]["deleted"]["deleted"], true);
        assert_serve_lifecycle(tmp.path(), &id, &[("delete", 1)]);
        assert!(!session_dir.exists(), "{agent} metadata must be removed");
        assert!(
            !runtime_metadata.exists(),
            "{agent} runtime metadata must be removed"
        );
    }

    let calls = std::fs::read_to_string(calls).unwrap_or_default();
    assert!(
        !calls.lines().any(|line| line.starts_with("kill-session")),
        "a definitively unlaunched runtime must not invoke kill-session: {calls}"
    );
}

#[tokio::test]
async fn delete_and_archive_refuse_a_session_with_children_unless_orphaned() {
    let tmp = tempfile::TempDir::new().unwrap();
    let tmux = executable(
        &tmp.path().join("children-guard-tmux"),
        "#!/usr/bin/env sh\ncase \"$1\" in\n  display-message|has-session) printf \"%s\\n\" \"can't find session: gone\" >&2; exit 1 ;;\n  *) exit 42 ;;\nesac\n",
    );
    // The fake tmux is a shell script whose launch cannot guarantee fitting
    // the termination probe's bounded startup budget under load, so pin the
    // probe outcome it models: the session is gone (sympoies/nils-cli#2154).
    let _probe =
        crate::tmux_probe_fixture::install(&tmux, crate::tmux_probe_fixture::Probe::Stopped);
    // guard-stale names a parent with the same id but an earlier creation
    // time, so it is not a child of the current guard-parent.
    for id in ["guard-parent", "guard-child", "guard-stale"] {
        seed_session_with_runtime(tmp.path(), id, "codex", &format!("hs-codex-{id}"));
        let record_path = tmp.path().join("sessions").join(id).join("session.json");
        let mut record: Value =
            serde_json::from_slice(&std::fs::read(&record_path).unwrap()).unwrap();
        record["tmux_runtime_never_launched"] = record["runtime"]["launch_id"].clone();
        if id != "guard-parent" {
            let created_at = if id == "guard-child" {
                "2000-01-01T00:00:00Z"
            } else {
                "1999-01-01T00:00:00Z"
            };
            let parent = json!({
                "machine": "elsewhere",
                "session_id": "guard-parent",
                "session_created_at": created_at,
            });
            record["lineage"] = json!({
                "schema_version": "agent-session.session-lineage.v1",
                "parent": parent,
                "root": parent,
                "depth": 1,
                "starter": {"kind": "session", "via": "cli"},
                "budget": null,
            });
        }
        std::fs::write(&record_path, serde_json::to_vec_pretty(&record).unwrap()).unwrap();
    }
    let st = state(tmp.path(), Some(TOKEN), tmux);
    let delete = |uri: &str| {
        Request::builder()
            .method("DELETE")
            .uri(uri)
            .header("authorization", format!("Bearer {TOKEN}"))
            .body(Body::empty())
            .unwrap()
    };
    let child = json!({
        "machine": MACHINE,
        "session_id": "guard-child",
        "session_created_at": "2000-01-01T00:00:00Z",
    });

    let (status, body) = call(router(st.clone()), delete("/sessions/guard-parent")).await;
    assert_eq!(status, StatusCode::CONFLICT, "body={body}");
    assert_eq!(body["error"]["code"], "session-has-live-children");
    assert_eq!(
        body["error"]["details"],
        json!({"children": [child], "scope": "local"})
    );
    let (status, body) = call(
        router(st.clone()),
        post_json(
            "/sessions/guard-parent/archive",
            Some(TOKEN),
            json!({"expected_session_incarnation": "launch-guard-parent"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "body={body}");
    assert_eq!(body["error"]["code"], "session-has-live-children");
    assert!(tmp.path().join("sessions/guard-parent").exists());

    let (status, body) = call(
        router(st.clone()),
        delete("/sessions/guard-parent?orphan_children=yes"),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body={body}");

    let (status, body) = call(
        router(st.clone()),
        delete("/sessions/guard-parent?orphan_children=true"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(
        body["data"]["deleted"]["children"],
        json!({"scope": "local", "orphaned": [child]})
    );
    assert!(!tmp.path().join("sessions/guard-parent").exists());

    let (status, body) = call(router(st.clone()), delete("/sessions/guard-child")).await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(
        body["data"]["deleted"]["children"],
        json!({"scope": "local", "orphaned": []})
    );

    // Archive takes the same override in its body and reports it.
    seed_resumable_session(
        tmp.path(),
        "guard-archive",
        "codex",
        "hs-codex-guard-archive",
        tmp.path(),
        &["resume", "guard-archive-provider-id"],
    );
    seed_session_with_runtime(
        tmp.path(),
        "guard-archive-child",
        "codex",
        "hs-codex-guard-archive-child",
    );
    let child_path = tmp.path().join("sessions/guard-archive-child/session.json");
    let mut record: Value = serde_json::from_slice(&std::fs::read(&child_path).unwrap()).unwrap();
    let parent = json!({
        "machine": MACHINE,
        "session_id": "guard-archive",
        "session_created_at": "2000-01-01T00:00:00Z",
    });
    record["lineage"] = json!({
        "schema_version": "agent-session.session-lineage.v1",
        "parent": parent,
        "root": parent,
        "depth": 1,
        "starter": {"kind": "session", "via": "cli"},
        "budget": null,
    });
    std::fs::write(&child_path, serde_json::to_vec_pretty(&record).unwrap()).unwrap();
    let (status, body) = call(
        router(st),
        post_json(
            "/sessions/guard-archive/archive",
            Some(TOKEN),
            json!({
                "expected_session_incarnation": "never-launched-fixture",
                "orphan_children": true,
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(
        body["data"]["deleted"]["children"],
        json!({"scope": "local", "orphaned": [{
            "machine": MACHINE,
            "session_id": "guard-archive-child",
            "session_created_at": "2000-01-01T00:00:00Z",
        }]})
    );
    assert!(!tmp.path().join("sessions/guard-archive").exists());
}

#[tokio::test]
async fn delete_operational_tmux_probe_error_returns_structured_failure() {
    let tmp = tempfile::TempDir::new().unwrap();
    let tmux = tmp.path().join("operational-error-tmux");
    std::fs::write(
            &tmux,
            "#!/usr/bin/env sh\nprintf '%s\\n' 'error connecting to /tmp/tmux-test/default (Permission denied)' >&2\nexit 1\n",
        )
        .unwrap();
    let mut permissions = std::fs::metadata(&tmux).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&tmux, permissions).unwrap();
    let id = "probe-error-codex";
    seed_session_with_runtime(tmp.path(), id, "codex", "hs-codex-probe-error");
    let session_dir = tmp.path().join("sessions").join(id);
    let st = state(tmp.path(), Some(TOKEN), tmux);
    let request = Request::builder()
        .method("DELETE")
        .uri(format!("/sessions/{id}"))
        .header("authorization", format!("Bearer {TOKEN}"))
        .body(Body::empty())
        .unwrap();

    let (status, body) = call(router(st), request).await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body={body}");
    assert_eq!(body["ok"], false);
    assert_eq!(body["error"]["code"], "session-termination-failed");
    assert_eq!(body["error"]["details"]["reason"], "verification-failed");
    assert!(session_dir.exists());
}

#[tokio::test]
async fn handle_input_resizes_and_ignores_malformed_frames() {
    let tmp = tempfile::TempDir::new().unwrap();
    let log = tmp.path().join("calls.log");
    let tmux = logging_tmux(tmp.path(), &log);
    let ctx = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let record = test_record("look", "hs-codex-look");
    let target = "hs-codex-look:0.0";
    let resize_lock = tokio::sync::Mutex::new(());

    // Malformed JSON and an empty object must not touch tmux.
    let mut pending = false;
    handle_input(
        &ctx,
        &tmux,
        &record,
        target,
        "{ not json",
        &mut pending,
        &resize_lock,
    )
    .await
    .unwrap();
    handle_input(
        &ctx,
        &tmux,
        &record,
        target,
        "{}",
        &mut pending,
        &resize_lock,
    )
    .await
    .unwrap();
    let after_noops = std::fs::read_to_string(&log).unwrap_or_default();
    assert!(
        after_noops.trim().is_empty(),
        "malformed/empty frames must not call tmux: {after_noops:?}"
    );

    // A non-initial resize frame maps to a single `tmux resize-window
    // -x <cols> -y <rows>` (no repaint nudge once the flag is spent).
    handle_input(
        &ctx,
        &tmux,
        &record,
        target,
        r#"{"resize":{"cols":123,"rows":45}}"#,
        &mut pending,
        &resize_lock,
    )
    .await
    .unwrap();
    let calls = std::fs::read_to_string(&log).unwrap_or_default();
    let resizes: Vec<&str> = calls
        .lines()
        .filter(|line| line.contains("resize-window"))
        .collect();
    assert_eq!(
        resizes.len(),
        1,
        "a non-initial resize must issue exactly one resize-window: {calls:?}"
    );
    assert!(
        resizes[0].contains("123") && resizes[0].contains("45"),
        "resize frame must invoke resize-window with the requested size: {calls:?}"
    );
}

#[tokio::test]
async fn incompatible_proxy_input_is_reported_before_auto_resume_mutation() {
    let tmp = tempfile::TempDir::new().unwrap();
    let log = tmp.path().join("calls.log");
    let tmux = logging_tmux(tmp.path(), &log);
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let mut record = test_record("old-proxy", "hs-codex-old-proxy");
    let (_socket_dir, socket) = nils_test_support::short_socket_path("old-proxy.sock");
    record.runtime = Some(crate::RuntimeInfo {
        kind: codex_app_server::RUNTIME_KIND.to_string(),
        tmux_session: record.tmux_session.clone(),
        generation: 1,
        started_at: "2030-01-01T00:00:00Z".to_string(),
        launch_id: "old-proxy-launch".to_string(),
        extra: std::collections::BTreeMap::from([
            (
                codex_app_server::PROTOCOL_KEY.to_string(),
                json!(codex_app_server::PROTOCOL_VERSION),
            ),
            (
                codex_app_server::SOCKET_KEY.to_string(),
                json!(crate::display_path(&socket)),
            ),
            (
                codex_app_server::PROXY_KEY.to_string(),
                json!(crate::display_path(&socket.with_extension("proxy"))),
            ),
            (
                codex_app_server::THREAD_HANDOFF_KEY.to_string(),
                json!(crate::display_path(&socket.with_extension("thread"))),
            ),
            (
                codex_app_server::THREAD_ATTACHED_KEY.to_string(),
                json!(crate::display_path(&socket.with_extension("attached"))),
            ),
        ]),
    });
    crate::write_session_record(&context, &record).unwrap();
    crate::activity::activate_runtime(&context, &record).unwrap();
    auto_resume::set_enabled(&context, &record.id, true, "2030-01-01T00:00:00Z").unwrap();

    let mut repaint = false;
    let error = handle_input(
        &context,
        &tmux,
        &record,
        "hs-codex-old-proxy:0.0",
        r#"{"text":"\r"}"#,
        &mut repaint,
        &tokio::sync::Mutex::new(()),
    )
    .await
    .unwrap_err();
    assert_eq!(error.code(), "codex-input-section-unavailable");
    assert!(auto_resume::view_for_record(&context, &record).enabled);
}

#[tokio::test]
async fn raw_terminal_carriage_return_is_delivered_as_enter_not_paste() {
    let tmp = tempfile::TempDir::new().unwrap();
    let log = tmp.path().join("calls.log");
    let tmux = logging_tmux(tmp.path(), &log);
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let record = test_record("raw-enter", "hs-codex-raw-enter");
    crate::write_session_record(&context, &record).unwrap();
    let mut repaint = false;

    handle_input(
        &context,
        &tmux,
        &record,
        "hs-codex-raw-enter:0.0",
        r#"{"text":"\r"}"#,
        &mut repaint,
        &tokio::sync::Mutex::new(()),
    )
    .await
    .unwrap();
    let calls = std::fs::read_to_string(&log).unwrap();
    assert!(calls.contains("send-keys -t hs-codex-raw-enter:0.0 Enter"));
    assert!(!calls.contains("load-buffer"));
    assert!(!calls.contains("paste-buffer"));
}

#[tokio::test]
async fn attach_backspace_is_delivered_as_named_tmux_key() {
    let tmp = tempfile::TempDir::new().unwrap();
    let log = tmp.path().join("calls.log");
    let tmux = logging_tmux(tmp.path(), &log);
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let record = test_record("backspace", "hs-codex-backspace");
    crate::write_session_record(&context, &record).unwrap();
    let mut repaint = false;

    handle_input(
        &context,
        &tmux,
        &record,
        "hs-codex-backspace:0.0",
        r#"{"keys":["backspace"]}"#,
        &mut repaint,
        &tokio::sync::Mutex::new(()),
    )
    .await
    .unwrap();

    let calls = std::fs::read_to_string(&log).unwrap_or_default();
    assert!(
        calls.contains("send-keys -t hs-codex-backspace:0.0 BSpace"),
        "Backspace must use a named tmux key: {calls:?}"
    );
    assert!(!calls.contains("load-buffer"));
    assert!(!calls.contains("paste-buffer"));
}

#[tokio::test]
async fn attach_shift_left_sequence_is_a_key_but_embedded_sequences_are_text() {
    for (frame, expected_key) in [
        (json!({"text": "\u{1b}[1;2D"}), true),
        (json!({"key": "shift-left"}), true),
        (json!({"keys": ["shift-left"]}), true),
        (json!({"text": "before\u{1b}[1;2Dafter"}), false),
        (json!({"text": "\u{1b}[200~\u{1b}[1;2D\u{1b}[201~"}), false),
    ] {
        let tmp = tempfile::TempDir::new().unwrap();
        let log = tmp.path().join("calls.log");
        let tmux = logging_tmux(tmp.path(), &log);
        let context = CliContext {
            state_dir: tmp.path().join("state"),
            host: None,
        };
        let record = test_record("shift-left", "hs-codex-shift-left");
        crate::write_session_record(&context, &record).unwrap();
        handle_input(
            &context,
            &tmux,
            &record,
            "hs-codex-shift-left:0.0",
            &frame.to_string(),
            &mut false,
            &tokio::sync::Mutex::new(()),
        )
        .await
        .unwrap();
        let calls = std::fs::read_to_string(&log).unwrap();
        assert_eq!(
            calls.contains("send-keys -t hs-codex-shift-left:0.0 S-Left"),
            expected_key
        );
        assert_eq!(calls.contains("paste-buffer"), !expected_key);
        assert_eq!(calls.contains("load-buffer"), !expected_key);
    }
}

#[tokio::test]
async fn first_resize_forces_a_repaint_nudge_then_plain_resizes() {
    let tmp = tempfile::TempDir::new().unwrap();
    let log = tmp.path().join("calls.log");
    let tmux = logging_tmux(tmp.path(), &log);
    let ctx = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let record = test_record("look", "hs-codex-look");
    let target = "hs-codex-look:0.0";
    let resize_lock = tokio::sync::Mutex::new(());

    // First resize after attach: nudge to rows-1, then the real rows, so the
    // agent is guaranteed a SIGWINCH and repaints its whole frame into the
    // freshly rebuilt client grid (fixes stale layout on re-entry).
    let mut pending = true;
    handle_input(
        &ctx,
        &tmux,
        &record,
        target,
        r#"{"resize":{"cols":80,"rows":24}}"#,
        &mut pending,
        &resize_lock,
    )
    .await
    .unwrap();
    let calls = std::fs::read_to_string(&log).unwrap_or_default();
    let resizes: Vec<&str> = calls
        .lines()
        .filter(|line| line.contains("resize-window"))
        .collect();
    assert_eq!(
        resizes.len(),
        2,
        "the first resize must nudge then set the real size: {calls:?}"
    );
    assert!(
        resizes[0].contains("-y 23"),
        "nudge step must use rows-1: {:?}",
        resizes[0]
    );
    assert!(
        resizes[1].contains("-y 24"),
        "final step must use the requested rows: {:?}",
        resizes[1]
    );
    assert!(
        !pending,
        "the initial-repaint flag must be consumed after the first resize"
    );

    // Subsequent resizes are a single plain resize-window (no flicker).
    std::fs::write(&log, "").unwrap();
    handle_input(
        &ctx,
        &tmux,
        &record,
        target,
        r#"{"resize":{"cols":100,"rows":40}}"#,
        &mut pending,
        &resize_lock,
    )
    .await
    .unwrap();
    let calls = std::fs::read_to_string(&log).unwrap_or_default();
    let resizes: Vec<&str> = calls
        .lines()
        .filter(|line| line.contains("resize-window"))
        .collect();
    assert_eq!(resizes.len(), 1, "later resizes must not nudge: {calls:?}");
    assert!(
        resizes[0].contains("-y 40"),
        "later resize must use the requested rows: {:?}",
        resizes[0]
    );
}

#[tokio::test]
async fn concurrent_first_resize_sequences_do_not_interleave() {
    let tmp = tempfile::TempDir::new().unwrap();
    let log = tmp.path().join("calls.log");
    let tmux = logging_tmux(tmp.path(), &log);
    let ctx = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let record = test_record("look", "hs-codex-look");
    let target = "hs-codex-look:0.0";
    let resize_lock = tokio::sync::Mutex::new(());
    let mut first_pending = true;
    let mut second_pending = true;

    let (first, second) = tokio::join!(
        handle_input(
            &ctx,
            &tmux,
            &record,
            target,
            r#"{"resize":{"cols":80,"rows":24}}"#,
            &mut first_pending,
            &resize_lock,
        ),
        handle_input(
            &ctx,
            &tmux,
            &record,
            target,
            r#"{"resize":{"cols":100,"rows":40}}"#,
            &mut second_pending,
            &resize_lock,
        ),
    );
    first.unwrap();
    second.unwrap();

    let calls = std::fs::read_to_string(&log).unwrap();
    let rows: Vec<&str> = calls
        .lines()
        .filter(|line| line.contains("resize-window"))
        .map(|line| line.rsplit_once("-y ").unwrap().1)
        .collect();
    assert!(
        rows == ["23", "24", "39", "40"] || rows == ["39", "40", "23", "24"],
        "each client's nudge/final resize pair must stay contiguous: {calls:?}"
    );
}

#[tokio::test]
async fn timed_out_resize_releases_the_shared_lock() {
    // The fake command blocks for much longer than the second resize may
    // wait for the lock, so the lock arrives in time only if the timed-out
    // resize released it while its command would still be running.
    const COMMAND_BLOCKS_FOR_SECS: u64 = 10;
    const LOCK_BOUND: Duration = Duration::from_secs(2);
    const FIRST_RESIZE_TIMEOUT: Duration = Duration::from_millis(20);
    // Bounds that only turn a stuck host into a failure. They are not part
    // of what the test proves, so they are generous.
    const BLOCKED_COMMAND_DEADLINE: Duration = Duration::from_secs(30);
    const SECOND_RESIZE_TIMEOUT: Duration = Duration::from_secs(30);

    let tmp = tempfile::TempDir::new().unwrap();
    let log = tmp.path().join("calls.log");
    let first_started = tmp.path().join("first-started");
    // `exec` makes the sleep the process the timeout kills, so no orphan
    // outlives the test holding its output.
    let tmux = executable(
        &tmp.path().join("tmux"),
        &format!(
            r#"#!/usr/bin/env sh
printf '%s\n' "$*" >> {}
if [ "$1" = "resize-window" ] && [ ! -e {} ]; then
  : > {}
  exec sleep {COMMAND_BLOCKS_FOR_SECS}
fi
exit 0
"#,
            shell_words::quote(&log.to_string_lossy()),
            shell_words::quote(&first_started.to_string_lossy()),
            shell_words::quote(&first_started.to_string_lossy()),
        ),
    );
    let resize_lock = tokio::sync::Mutex::new(());

    // The first client owns the lock before the second one asks for it.
    let first_guard = resize_lock.lock().await;
    let first = async {
        let _guard = first_guard;
        // The resize ignores its command's result, and on a loaded host the
        // timeout can kill the command before it writes its marker, or the
        // spawn can fail. Retry under the held lock until one attempt has
        // really started the blocking command; that attempt is the last.
        let deadline = Instant::now() + BLOCKED_COMMAND_DEADLINE;
        loop {
            let attempt_started = Instant::now();
            resize_pane_with_timeout(
                &tmux,
                "hs-codex-look:0.0",
                80,
                24,
                false,
                FIRST_RESIZE_TIMEOUT,
            )
            .await;
            if first_started.exists() {
                break attempt_started;
            }
            assert!(
                Instant::now() < deadline,
                "no resize attempt reached the blocking command within \
                     {BLOCKED_COMMAND_DEADLINE:?}"
            );
        }
    };
    let second = async {
        let _guard = resize_lock.lock().await;
        let lock_acquired = Instant::now();
        resize_pane_with_timeout(
            &tmux,
            "hs-codex-look:0.0",
            100,
            40,
            false,
            SECOND_RESIZE_TIMEOUT,
        )
        .await;
        lock_acquired
    };
    let (blocking_attempt_started, lock_acquired) = tokio::join!(first, second);

    // The blocking command started no earlier than its attempt did, so a
    // lock acquired within `LOCK_BOUND` of that attempt was acquired while
    // the command would still be running.
    let lock_wait = lock_acquired.duration_since(blocking_attempt_started);
    assert!(
        lock_wait < LOCK_BOUND,
        "a timed-out resize kept the shared lock: the second resize got it \
             {lock_wait:?} after a command that blocks for {COMMAND_BLOCKS_FOR_SECS} s started"
    );
    let calls = std::fs::read_to_string(&log).unwrap();
    let resizes: Vec<&str> = calls
        .lines()
        .filter(|line| line.contains("resize-window"))
        .collect();
    let second_resizes = resizes
        .iter()
        .filter(|line| line.ends_with("-x 100 -y 40"))
        .count();
    assert!(
        second_resizes == 1
            && resizes
                .last()
                .is_some_and(|line| line.ends_with("-x 100 -y 40")),
        "the second resize must run once, after the first command times out: {calls:?}"
    );
}

#[test]
fn from_name_round_trips_agent_and_key() {
    use clap::ValueEnum;
    for agent in AgentKind::value_variants() {
        assert_eq!(AgentKind::from_name(agent.as_str()), Some(*agent));
    }
    assert_eq!(AgentKind::from_name("banana"), None);
    for key in SpecialKey::value_variants() {
        assert_eq!(SpecialKey::from_name(key.as_str()), Some(*key));
    }
    assert_eq!(SpecialKey::from_name("f13"), None);
}

#[test]
fn backspace_key_round_trips_cli_and_tmux() {
    use clap::Parser;

    let cli = crate::cli::Cli::try_parse_from([
        "agent-session",
        "send",
        "session-id",
        "--key",
        "backspace",
    ])
    .expect("Backspace must be accepted by --key");
    let crate::cli::Command::Send(args) = cli.command else {
        panic!("expected send command");
    };
    assert_eq!(args.keys.len(), 1);
    assert_eq!(args.keys[0].as_str(), "backspace");
    assert_eq!(args.keys[0].tmux_key(), "BSpace");
}

#[test]
fn lagged_live_output_disconnects_the_slow_client() {
    assert!(
        attach_event_bytes(Err(tokio::sync::broadcast::error::RecvError::Lagged(1))).is_none(),
        "a lagged subscriber must disconnect instead of blocking the broker"
    );
    assert!(
        attach_event_bytes(Ok(AttachEvent::Closed)).is_none(),
        "a closed broker must disconnect its subscribers"
    );
}

#[tokio::test]
async fn stalled_websocket_send_is_bounded() {
    struct PendingSink;

    impl futures_util::Sink<Message> for PendingSink {
        type Error = io::Error;

        fn poll_ready(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Result<(), Self::Error>> {
            std::task::Poll::Pending
        }

        fn start_send(self: std::pin::Pin<&mut Self>, _item: Message) -> Result<(), Self::Error> {
            Ok(())
        }

        fn poll_flush(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Result<(), Self::Error>> {
            std::task::Poll::Pending
        }

        fn poll_close(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Result<(), Self::Error>> {
            std::task::Poll::Ready(Ok(()))
        }
    }

    assert!(
        !send_attach_message(
            &mut PendingSink,
            Message::Binary(Bytes::from_static(b"blocked")),
            Duration::from_millis(20),
        )
        .await,
        "a stalled WebSocket sink must time out"
    );
}

#[tokio::test]
async fn outbound_writer_reports_a_stalled_sink() {
    struct PendingSink;

    impl futures_util::Sink<Message> for PendingSink {
        type Error = io::Error;

        fn poll_ready(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Result<(), Self::Error>> {
            std::task::Poll::Pending
        }

        fn start_send(self: std::pin::Pin<&mut Self>, _item: Message) -> Result<(), Self::Error> {
            Ok(())
        }

        fn poll_flush(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Result<(), Self::Error>> {
            std::task::Poll::Pending
        }

        fn poll_close(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Result<(), Self::Error>> {
            std::task::Poll::Ready(Ok(()))
        }
    }

    let (terminal_tx, terminal_rx) = mpsc::channel(1);
    let (control_tx, control_rx) = mpsc::channel(1);
    terminal_tx
        .send(Message::Binary(Bytes::from_static(b"blocked")))
        .await
        .unwrap();
    drop(terminal_tx);
    drop(control_tx);

    assert_eq!(
        outbound_writer(
            PendingSink,
            terminal_rx,
            control_rx,
            Duration::from_millis(20),
        )
        .await,
        AttachWriterExit::SendFailed
    );
}

#[tokio::test]
async fn normal_broker_close_drains_frames_already_accepted_by_the_writer() {
    #[derive(Clone)]
    struct RecordingSink(Arc<std::sync::Mutex<Vec<Message>>>);

    impl futures_util::Sink<Message> for RecordingSink {
        type Error = io::Error;

        fn poll_ready(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Result<(), Self::Error>> {
            std::task::Poll::Ready(Ok(()))
        }

        fn start_send(self: std::pin::Pin<&mut Self>, item: Message) -> Result<(), Self::Error> {
            self.0.lock().unwrap().push(item);
            Ok(())
        }

        fn poll_flush(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Result<(), Self::Error>> {
            std::task::Poll::Ready(Ok(()))
        }

        fn poll_close(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Result<(), Self::Error>> {
            std::task::Poll::Ready(Ok(()))
        }
    }

    let written = Arc::new(std::sync::Mutex::new(Vec::new()));
    let (terminal_tx, terminal_rx) = mpsc::channel(2);
    let (control_tx, control_rx) = mpsc::channel(1);
    terminal_tx
        .send(Message::Binary(Bytes::from_static(b"first")))
        .await
        .unwrap();
    terminal_tx
        .send(Message::Binary(Bytes::from_static(b"second")))
        .await
        .unwrap();
    drop(terminal_tx);
    drop(control_tx);

    let task = AbortOnDropTask::new(tokio::spawn(outbound_writer(
        RecordingSink(written.clone()),
        terminal_rx,
        control_rx,
        Duration::from_secs(1),
    )));
    finish_outbound_writer(task, true).await;

    let written = written.lock().unwrap();
    assert_eq!(written.len(), 2);
    assert_eq!(written[0], Message::Binary(Bytes::from_static(b"first")));
    assert_eq!(written[1], Message::Binary(Bytes::from_static(b"second")));
}

#[tokio::test]
async fn attach_pump_drains_beyond_the_broadcast_ring_capacity() {
    let (broker_tx, broker_rx) = tokio::sync::broadcast::channel(ATTACH_BROADCAST_CAPACITY);
    let (terminal_tx, mut terminal_rx) = mpsc::channel(ATTACH_BROADCAST_CAPACITY * 2);
    let task = tokio::spawn(pump_attach_events(broker_rx, terminal_tx));

    for index in 0..=ATTACH_BROADCAST_CAPACITY {
        broker_tx
            .send(AttachEvent::Output(Bytes::from(vec![index as u8])))
            .unwrap();
        tokio::task::yield_now().await;
    }
    broker_tx.send(AttachEvent::Closed).unwrap();

    assert_eq!(task.await.unwrap(), AttachPumpExit::BrokerClosed);
    let mut received = 0;
    while terminal_rx.recv().await.is_some() {
        received += 1;
    }
    assert_eq!(received, ATTACH_BROADCAST_CAPACITY + 1);
}

#[tokio::test]
async fn dropping_an_attach_child_task_aborts_it() {
    struct DropSignal(Option<tokio::sync::oneshot::Sender<()>>);

    impl Drop for DropSignal {
        fn drop(&mut self) {
            if let Some(sender) = self.0.take() {
                let _ = sender.send(());
            }
        }
    }

    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (dropped_tx, dropped_rx) = tokio::sync::oneshot::channel();
    let task = AbortOnDropTask::new(tokio::spawn(async move {
        let _signal = DropSignal(Some(dropped_tx));
        let _ = started_tx.send(());
        std::future::pending::<()>().await;
    }));
    started_rx.await.unwrap();
    drop(task);

    tokio::time::timeout(Duration::from_millis(250), dropped_rx)
        .await
        .expect("aborted task must drop its owned state")
        .expect("drop signal");
}

#[test]
fn attach_replay_normalizes_pane_lines_for_a_raw_pty_stream() {
    let replay = pty_ready_attach_replay("first\nsecond\n").expect("replay");

    assert_eq!(
        String::from_utf8(replay).expect("utf8"),
        "\x1b]8;;\x1b\\\x1b[0m\x0f\x1b[H\x1b[2Jfirst\r\nsecond\r\n\x1b]8;;\x1b\\\x1b[0m\x0f",
    );
}

#[test]
fn attach_replay_does_not_double_existing_carriage_returns() {
    let replay = pty_ready_attach_replay("first\r\nsecond\r\n").expect("replay");

    assert_eq!(
        String::from_utf8(replay).expect("utf8"),
        "\x1b]8;;\x1b\\\x1b[0m\x0f\x1b[H\x1b[2Jfirst\r\nsecond\r\n\x1b]8;;\x1b\\\x1b[0m\x0f",
    );
}

#[test]
fn attach_replay_trims_trailing_blank_pane_rows() {
    let replay = pty_ready_attach_replay("first\n\n   \n\n").expect("replay");

    assert_eq!(
        String::from_utf8(replay).expect("utf8"),
        "\x1b]8;;\x1b\\\x1b[0m\x0f\x1b[H\x1b[2Jfirst\r\n\x1b]8;;\x1b\\\x1b[0m\x0f",
    );
}

#[test]
fn attach_replay_keeps_interior_blank_rows() {
    let replay = pty_ready_attach_replay("first\n\nthird\n").expect("replay");

    assert_eq!(
        String::from_utf8(replay).expect("utf8"),
        "\x1b]8;;\x1b\\\x1b[0m\x0f\x1b[H\x1b[2Jfirst\r\n\r\nthird\r\n\x1b]8;;\x1b\\\x1b[0m\x0f",
    );
}

#[test]
fn attach_replay_drops_a_blank_capture() {
    assert!(pty_ready_attach_replay("").is_none());
    assert!(pty_ready_attach_replay("\n\n").is_none());
    assert!(pty_ready_attach_replay("   \n").is_none());
}

#[test]
fn attach_replay_preserves_styling_and_wide_characters() {
    let replay = pty_ready_attach_replay("\x1b[32m✔ 測試通過\x1b[0m\n").expect("replay");

    assert_eq!(
        String::from_utf8(replay).expect("utf8"),
        "\x1b]8;;\x1b\\\x1b[0m\x0f\x1b[H\x1b[2J\x1b[32m✔ 測試通過\x1b[0m\r\n\x1b]8;;\x1b\\\x1b[0m\x0f",
    );
}

#[test]
fn attach_replay_keeps_an_osc8_hyperlink_intact() {
    // The whole point of capturing with -e: a hyperlink whose visible label
    // is not a URL survives re-attach. Without it the replayed scrollback
    // is plain text and the link is gone until the provider repaints.
    let replay = pty_ready_attach_replay(
        "see \x1b]8;;https://example.test/issues/398\x1b\\#398\x1b]8;;\x1b\\ now\n",
    )
    .expect("replay");

    let text = String::from_utf8(replay).expect("utf8");
    assert!(
        text.contains("\x1b]8;;https://example.test/issues/398\x1b\\#398\x1b]8;;\x1b\\"),
        "replay must carry the hyperlink verbatim: {text:?}",
    );
}

#[test]
fn attach_replay_trims_trailing_rows_that_only_carry_escapes() {
    // tmux pads the capture to the pane height. With -e those padding rows
    // can carry attribute or hyperlink resets, which are not whitespace, so
    // a naive emptiness test would keep them and push the provider's own
    // repaint down the screen.
    let replay =
        pty_ready_attach_replay("first\n\x1b[0m\n\x1b]8;;\x1b\\\n   \x1b[m\n").expect("replay");

    assert_eq!(
        String::from_utf8(replay).expect("utf8"),
        "\x1b]8;;\x1b\\\x1b[0m\x0f\x1b[H\x1b[2Jfirst\r\n\x1b]8;;\x1b\\\x1b[0m\x0f",
    );
}

#[test]
fn attach_replay_closes_an_acs_run_left_open_by_the_captured_region() {
    // tmux encodes ACS line drawing as SO/SI, and the -J capture emits the
    // SO without its SI (verified against tmux 3.4). SGR 0 does not reset
    // the charset glevel, so without an explicit SI a later `enacs` from any
    // ncurses repaint would render ordinary text as box-drawing glyphs.
    let replay = pty_ready_attach_replay("\x0elqqk\n").expect("replay");

    let text = String::from_utf8(replay).expect("utf8");
    assert!(
        text.ends_with('\x0f'),
        "replay must shift back in after a captured ACS run: {text:?}",
    );
}

#[test]
fn attach_replay_resets_before_erasing_the_screen() {
    // Erase-display paints with the current background on a back-colour-erase
    // terminal, and the client reuses one Terminal across reconnects, so the
    // reset has to precede the erase or a re-attach repaints the screen in
    // the previous stream's stale colour.
    let replay = pty_ready_attach_replay("first\n").expect("replay");

    let text = String::from_utf8(replay).expect("utf8");
    let reset = text.find("\x1b[0m").expect("reset present");
    let erase = text.find("\x1b[2J").expect("erase present");
    assert!(reset < erase, "reset must precede the erase: {text:?}");
}

#[test]
fn attach_replay_closes_state_left_open_by_the_captured_region() {
    // A hyperlink that continues past the captured region leaves tmux's -e
    // output mid-link. Everything the provider paints underneath would join
    // that link and open the wrong target on a click, so the replay closes
    // the hyperlink and clears attributes at both ends.
    let replay = pty_ready_attach_replay("\x1b]8;;https://example.test/open\x1b\\dangling\n")
        .expect("replay");

    let text = String::from_utf8(replay).expect("utf8");
    assert!(
        text.ends_with("\x1b]8;;\x1b\\\x1b[0m\x0f"),
        "replay must close hyperlink and attribute state: {text:?}",
    );
    assert!(
        text.starts_with("\x1b]8;;\x1b\\\x1b[0m\x0f\x1b[H\x1b[2J"),
        "replay must not inherit state from the preceding stream: {text:?}",
    );
}

#[tokio::test]
async fn websocket_attach_replays_a_pty_ready_snapshot_for_every_provider() {
    // The staircased attach screen was provider-visible but not
    // provider-specific: the same tmux capture path serves Codex, Claude
    // Code, and DSH, so every provider must receive CRLF-terminated
    // rows from a deterministic origin.
    for (id, agent, tmux_session) in [
        ("ws-replay-codex", "codex", "hs-codex-ws-replay"),
        ("ws-replay-claude", "claude", "hs-claude-ws-replay"),
        ("ws-replay-dsh", "dsh", "hs-dsh-ws-replay"),
    ] {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let state_dir = tmp.path().join("state");
        seed_session(&state_dir, id, agent, tmux_session);
        let tmux = minimal_tmux(tmp.path());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener");
        let addr = listener.local_addr().expect("address");
        let server = tokio::spawn(async move {
            let _ = axum::serve(listener, router(state(&state_dir, Some(TOKEN), tmux))).await;
        });

        let mut request = format!("ws://{addr}/sessions/{id}/attach")
            .into_client_request()
            .expect("request");
        request.headers_mut().insert(
            AUTHORIZATION,
            format!("Bearer {TOKEN}").parse().expect("authorization"),
        );
        let (mut socket, _) = connect_async(request).await.expect("connect");
        let snapshot = tokio::time::timeout(Duration::from_secs(2), socket.next())
            .await
            .expect("snapshot timeout")
            .expect("snapshot frame")
            .expect("snapshot message");
        let ClientMessage::Binary(bytes) = snapshot else {
            panic!("{agent} attach snapshot must be a binary frame");
        };
        assert_eq!(
            bytes.as_ref(),
            pty_ready_attach_replay("pane\n")
                .expect("replay")
                .as_slice(),
            "{agent} attach snapshot must replay CRLF rows from a known origin",
        );

        let _ = socket.close(None).await;
        server.abort();
    }
}

#[tokio::test]
async fn snapshot_handoff_recaptures_after_internal_buffer_overflow() {
    let tmp = tempfile::TempDir::new().unwrap();
    let state_dir = tmp.path().join("state");
    let calls = tmp.path().join("capture-calls");
    let first_started = tmp.path().join("first-started");
    let first_release = tmp.path().join("first-release");
    let second_started = tmp.path().join("second-started");
    let second_release = tmp.path().join("second-release");
    seed_session(&state_dir, "recapture", "codex", "hs-codex-recapture");
    let tmux = executable(
        &tmp.path().join("tmux"),
        &format!(
            r#"#!/usr/bin/env sh
if [ "$1" = "capture-pane" ]; then
  printf 'capture\n' >> {calls}
  count=$(wc -l < {calls})
  if [ "$count" -eq 1 ]; then
    : > {first_started}
    while [ ! -f {first_release} ]; do sleep 0.01; done
    printf 'stale\n'
  else
    : > {second_started}
    while [ ! -f {second_release} ]; do sleep 0.01; done
    printf 'fresh\n'
  fi
fi
exit 0
"#,
            calls = shell_words::quote(&calls.to_string_lossy()),
            first_started = shell_words::quote(&first_started.to_string_lossy()),
            first_release = shell_words::quote(&first_release.to_string_lossy()),
            second_started = shell_words::quote(&second_started.to_string_lossy()),
            second_release = shell_words::quote(&second_release.to_string_lossy()),
        ),
    );
    let context = CliContext {
        state_dir,
        host: None,
    };
    let record = load_session_record(&context, "recapture").unwrap();
    let (broker_tx, mut broker_rx) = tokio::sync::broadcast::channel(ATTACH_BROADCAST_CAPACITY);
    let task =
        tokio::spawn(async move { capture_attach_handoff(&tmux, &record, &mut broker_rx).await });

    wait_for_file(&first_started).await;
    for index in 0..=ATTACH_HANDOFF_BUFFER_CAPACITY {
        broker_tx
            .send(AttachEvent::Output(Bytes::from(vec![index as u8])))
            .unwrap();
        tokio::task::yield_now().await;
    }
    std::fs::write(&first_release, b"").unwrap();
    wait_for_file(&second_started).await;
    broker_tx
        .send(AttachEvent::Output(Bytes::from_static(b"after-recapture")))
        .unwrap();
    tokio::task::yield_now().await;
    std::fs::write(&second_release, b"").unwrap();

    let handoff = task.await.unwrap().unwrap();
    assert_eq!(handoff.snapshot.as_deref(), Some("fresh\n"));
    assert_eq!(handoff.live.len(), 1);
    assert_eq!(handoff.live[0], Bytes::from_static(b"after-recapture"));
    assert_eq!(std::fs::read_to_string(calls).unwrap().lines().count(), 2);
}

#[tokio::test]
async fn attach_snapshot_capture_joins_wrapped_rows() {
    let tmp = tempfile::TempDir::new().unwrap();
    let state_dir = tmp.path().join("state");
    let args_log = tmp.path().join("capture-args");
    seed_session(&state_dir, "joined", "codex", "hs-codex-joined");
    let tmux = executable(
        &tmp.path().join("tmux"),
        &format!(
            "#!/usr/bin/env sh\nprintf '%s\\n' \"$@\" > {args}\nprintf 'pane\\n'\nexit 0\n",
            args = shell_words::quote(&args_log.to_string_lossy()),
        ),
    );
    let context = CliContext {
        state_dir,
        host: None,
    };
    let record = load_session_record(&context, "joined").unwrap();

    let capture = capture_attach_snapshot(&tmux, &record).await.unwrap();

    assert_eq!(capture.as_deref(), Some("pane\n"));
    // -J joins tmux-wrapped rows back into logical lines, so the attaching
    // client's terminal re-wraps them at its own width and marks the
    // continuation rows as wrapped. Without it, a wrapped URL or command in
    // the replayed scrollback arrives as unrelated hard rows.
    //
    // -e keeps the escape sequences. `capture-pane -p` alone prints plain
    // text, so every re-attach repainted the scrollback with no attributes
    // and — the reason this is a bug, not a cosmetic loss — no OSC 8
    // hyperlinks. A link whose visible label is not a URL simply stopped
    // existing until the provider repainted that region.
    let args: Vec<String> = std::fs::read_to_string(&args_log)
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect();
    assert_eq!(
        args,
        vec![
            "capture-pane",
            "-p",
            "-e",
            "-J",
            "-t",
            "hs-codex-joined",
            "-S",
            "-200"
        ],
    );
}

#[tokio::test]
async fn attach_broker_fans_out_and_only_last_subscriber_tears_down_pipe() {
    let tmp = tempfile::TempDir::new().unwrap();
    let state_dir = tmp.path().join("state");
    let calls = tmp.path().join("calls.log");
    let source = tmp.path().join("pane-output.fifo");
    let writer_guard = FixtureWriterGuard::new(tmp.path(), &source);
    create_private_fifo(&source).unwrap();
    seed_session(&state_dir, "fanout", "codex", "hs-codex-fanout");

    let tmux = fanout_tmux(tmp.path(), &calls, &source, writer_guard.identity_dir());
    let context = CliContext {
        state_dir: state_dir.clone(),
        host: None,
    };
    let record = load_session_record(&context, "fanout").unwrap();
    let registry = AttachBrokerRegistry::default();
    let live_fifo = session_dir(&context, "fanout").join(ATTACH_LIVE_FIFO_NAME);
    crate::write_private_file(&live_fifo, b"stale daemon bytes").unwrap();

    let mut first = registry.subscribe(&context, &tmux, &record).await.unwrap();
    let mut second = registry.subscribe(&context, &tmux, &record).await.unwrap();
    assert!(
        std::fs::symlink_metadata(&live_fifo)
            .unwrap()
            .file_type()
            .is_fifo(),
        "the live broker transport must be an ephemeral FIFO"
    );

    let mut source_writer = std::fs::OpenOptions::new()
        .write(true)
        .open(&source)
        .unwrap();
    source_writer.write_all(b"hello").unwrap();
    source_writer.flush().unwrap();

    assert_eq!(recv_attach_bytes(&mut first).await, b"hello");
    assert_eq!(recv_attach_bytes(&mut second).await, b"hello");

    let closes_before_detach = pipe_close_count(&calls);
    first.release().await;
    assert_eq!(
        pipe_close_count(&calls),
        closes_before_detach,
        "disconnecting one subscriber must not close the shared tmux pipe"
    );
    assert!(live_fifo.exists());

    source_writer.write_all(b"again").unwrap();
    source_writer.flush().unwrap();
    assert_eq!(recv_attach_bytes(&mut second).await, b"again");

    second.release().await;
    assert_eq!(
        pipe_close_count(&calls),
        closes_before_detach + 1,
        "the final subscriber must close the shared tmux pipe"
    );
    assert!(
        !live_fifo.exists(),
        "broker teardown must remove the ephemeral FIFO"
    );

    let calls = std::fs::read_to_string(&calls).unwrap();
    let enables: Vec<_> = calls
        .lines()
        .filter(|line| line.contains("cat >"))
        .collect();
    assert_eq!(
        enables.len(),
        1,
        "simultaneous subscribers must share one tmux pipe: {calls:?}"
    );
    assert!(
        !enables[0].split_whitespace().any(|arg| arg == "-o"),
        "a fresh broker must replace a stale tmux pipe instead of preserving it: {calls:?}"
    );
    writer_guard.cleanup().expect("cleanup fixture writers");
}

#[tokio::test]
async fn late_attach_cleanup_does_not_stop_a_same_id_replacement() {
    let tmp = tempfile::TempDir::new().unwrap();
    let state_dir = tmp.path().join("state");
    let calls = tmp.path().join("calls.log");
    let source = tmp.path().join("pane-output.fifo");
    let writer_guard = FixtureWriterGuard::new(tmp.path(), &source);
    create_private_fifo(&source).unwrap();
    seed_session_with_runtime(
        &state_dir,
        "replacement-fence",
        "codex",
        "hs-codex-replacement-fence",
    );
    let tmux = fanout_tmux(tmp.path(), &calls, &source, writer_guard.identity_dir());
    let context = CliContext {
        state_dir,
        host: None,
    };
    let old_record = load_session_record(&context, "replacement-fence").unwrap();
    let old_fence = crate::SessionRegistryFence::from_record(&old_record);
    let registry = AttachBrokerRegistry::default();
    let first = registry
        .subscribe(&context, &tmux, &old_record)
        .await
        .unwrap();

    let mut replacement = old_record.clone();
    let runtime = replacement.runtime.as_mut().unwrap();
    runtime.launch_id = "launch-replacement-fence-next".to_string();
    runtime.generation = 2;
    let mut second = registry
        .subscribe(&context, &tmux, &replacement)
        .await
        .unwrap();
    registry.shutdown_session_if_matches(&old_fence).await;

    assert_eq!(
        registry.subscriber_count("replacement-fence").await,
        1,
        "cleanup for the deleted runtime must retain the same-id replacement broker"
    );
    let mut source_writer = std::fs::OpenOptions::new()
        .write(true)
        .open(&source)
        .unwrap();
    source_writer.write_all(b"replacement-live").unwrap();
    source_writer.flush().unwrap();
    assert_eq!(recv_attach_bytes(&mut second).await, b"replacement-live");
    first.release().await;
    second.release().await;
    wait_for_subscriber_count(&registry, "replacement-fence", 0).await;
    writer_guard.cleanup().expect("cleanup fixture writers");
}

#[tokio::test]
async fn websocket_attach_fans_out_live_output_after_snapshot() {
    let tmp = tempfile::TempDir::new().unwrap();
    let state_dir = tmp.path().join("state");
    let calls = tmp.path().join("calls.log");
    let source = tmp.path().join("pane-output.fifo");
    let writer_guard = FixtureWriterGuard::new(tmp.path(), &source);
    create_private_fifo(&source).unwrap();
    seed_session(&state_dir, "socket", "codex", "hs-codex-socket");

    let tmux = fanout_tmux(tmp.path(), &calls, &source, writer_guard.identity_dir());
    let state = state(&state_dir, Some(TOKEN), tmux);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = router(state.clone());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let connect = || async {
        let mut request = format!("ws://{address}/sessions/socket/attach")
            .into_client_request()
            .unwrap();
        request
            .headers_mut()
            .insert("authorization", format!("Bearer {TOKEN}").parse().unwrap());
        tokio_tungstenite::connect_async(request).await.unwrap().0
    };
    let mut first = connect().await;
    let mut second = connect().await;
    assert_eq!(
        recv_websocket_binary(&mut first).await,
        pty_ready_attach_replay("pane\n")
            .expect("replay")
            .as_slice()
    );
    assert_eq!(
        recv_websocket_binary(&mut second).await,
        pty_ready_attach_replay("pane\n")
            .expect("replay")
            .as_slice()
    );
    wait_for_subscriber_count(&state.attach_brokers, "socket", 2).await;

    let mut source_writer = std::fs::OpenOptions::new()
        .write(true)
        .open(&source)
        .unwrap();
    source_writer.write_all(b"both-clients").unwrap();
    source_writer.flush().unwrap();
    assert_eq!(recv_websocket_binary(&mut first).await, b"both-clients");
    assert_eq!(recv_websocket_binary(&mut second).await, b"both-clients");

    first.close(None).await.unwrap();
    wait_for_subscriber_count(&state.attach_brokers, "socket", 1).await;
    source_writer.write_all(b"remaining-client").unwrap();
    source_writer.flush().unwrap();
    assert_eq!(
        recv_websocket_binary(&mut second).await,
        b"remaining-client"
    );

    second.close(None).await.unwrap();
    wait_for_subscriber_count(&state.attach_brokers, "socket", 0).await;
    server.abort();
    let _ = server.await;
    writer_guard.cleanup().expect("cleanup fixture writers");
}

#[tokio::test]
async fn websocket_attach_buffers_live_output_during_snapshot_capture() {
    let tmp = tempfile::TempDir::new().unwrap();
    let state_dir = tmp.path().join("state");
    let calls = tmp.path().join("calls.log");
    let fifo_destination = tmp.path().join("fifo-destination");
    let writer_pid = tmp.path().join("writer.pid");
    let capture_started = tmp.path().join("capture-started");
    let capture_release = tmp.path().join("capture-release");
    seed_session(&state_dir, "handoff", "codex", "hs-codex-handoff");

    let tmux = snapshot_handoff_tmux(
        tmp.path(),
        &calls,
        &fifo_destination,
        &writer_pid,
        &capture_started,
        &capture_release,
    );
    let state = state(&state_dir, Some(TOKEN), tmux);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = router(state.clone());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let mut request = format!("ws://{address}/sessions/handoff/attach")
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("authorization", format!("Bearer {TOKEN}").parse().unwrap());
    let mut socket = tokio_tungstenite::connect_async(request).await.unwrap().0;
    tokio::time::timeout(Duration::from_secs(1), async {
        while !capture_started.exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("capture-pane never reached its controlled wait");

    let live_fifo = PathBuf::from(std::fs::read_to_string(&fifo_destination).unwrap());
    let mut live_writer = std::fs::OpenOptions::new()
        .write(true)
        .open(live_fifo)
        .unwrap();
    live_writer.write_all(b"during-capture").unwrap();
    live_writer.flush().unwrap();
    std::fs::write(&capture_release, b"").unwrap();

    assert_eq!(
        recv_websocket_binary(&mut socket).await,
        pty_ready_attach_replay("pane\n")
            .expect("replay")
            .as_slice()
    );
    assert_eq!(recv_websocket_binary(&mut socket).await, b"during-capture");
    socket.close(None).await.unwrap();
    wait_for_subscriber_count(&state.attach_brokers, "handoff", 0).await;
    server.abort();
    let _ = server.await;
}

#[tokio::test]
async fn snapshot_timeout_releases_the_final_attach_broker() {
    let tmp = tempfile::TempDir::new().unwrap();
    let state_dir = tmp.path().join("state");
    let calls = tmp.path().join("calls.log");
    let fifo_destination = tmp.path().join("fifo-destination");
    let writer_pid = tmp.path().join("writer.pid");
    let capture_started = tmp.path().join("capture-started");
    let capture_release = tmp.path().join("never-release");
    seed_session(&state_dir, "timeout", "codex", "hs-codex-timeout");

    let tmux = snapshot_handoff_tmux(
        tmp.path(),
        &calls,
        &fifo_destination,
        &writer_pid,
        &capture_started,
        &capture_release,
    );
    let state = state(&state_dir, Some(TOKEN), tmux);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = router(state.clone());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let mut request = format!("ws://{address}/sessions/timeout/attach")
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("authorization", format!("Bearer {TOKEN}").parse().unwrap());
    let mut socket = tokio_tungstenite::connect_async(request).await.unwrap().0;
    tokio::time::timeout(Duration::from_secs(1), async {
        while !capture_started.exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("capture-pane never started");
    assert_eq!(state.attach_brokers.subscriber_count("timeout").await, 1);
    tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            if state.attach_brokers.subscriber_count("timeout").await == 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("snapshot timeout did not release the final broker lease");
    assert!(
        !session_dir(&state.context, "timeout")
            .join(ATTACH_LIVE_FIFO_NAME)
            .exists(),
        "snapshot timeout must remove the broker FIFO"
    );
    let _ = socket.close(None).await;
    server.abort();
    let _ = server.await;
}

#[tokio::test]
async fn dropping_last_attach_subscription_releases_the_broker() {
    let tmp = tempfile::TempDir::new().unwrap();
    let state_dir = tmp.path().join("state");
    let calls = tmp.path().join("calls.log");
    let source = tmp.path().join("pane-output.fifo");
    let writer_guard = FixtureWriterGuard::new(tmp.path(), &source);
    create_private_fifo(&source).unwrap();
    seed_session(&state_dir, "cancel", "codex", "hs-codex-cancel");

    let tmux = fanout_tmux(tmp.path(), &calls, &source, writer_guard.identity_dir());
    let context = CliContext {
        state_dir: state_dir.clone(),
        host: None,
    };
    let record = load_session_record(&context, "cancel").unwrap();
    let registry = AttachBrokerRegistry::default();
    let live_fifo = session_dir(&context, "cancel").join(ATTACH_LIVE_FIFO_NAME);

    let subscription = registry.subscribe(&context, &tmux, &record).await.unwrap();
    assert_eq!(registry.subscriber_count("cancel").await, 1);
    drop(subscription);

    wait_for_subscriber_count(&registry, "cancel", 0).await;
    assert!(
        !live_fifo.exists(),
        "cancelling an attach task must remove its final broker FIFO"
    );
    assert_eq!(pipe_close_count(&calls), 1);
    writer_guard.cleanup().expect("cleanup fixture writers");
}

#[tokio::test]
async fn aborting_release_during_teardown_does_not_strand_the_broker() {
    let tmp = tempfile::TempDir::new().unwrap();
    let state_dir = tmp.path().join("state");
    let calls = tmp.path().join("calls.log");
    let close_started = tmp.path().join("close-started");
    seed_session(&state_dir, "abort", "codex", "hs-codex-abort");
    let tmux = executable(
        &tmp.path().join("tmux"),
        &format!(
            r#"#!/usr/bin/env sh
printf '%s\n' "$*" >> {}
if [ "$1" = "pipe-pane" ]; then
  last=""
  for arg in "$@"; do last="$arg"; done
  case "$last" in
    "cat > "*) ;;
    *)
      if [ ! -e {} ]; then
        : > {}
        sleep 1
      fi
      ;;
  esac
fi
exit 0
"#,
            shell_words::quote(&calls.to_string_lossy()),
            shell_words::quote(&close_started.to_string_lossy()),
            shell_words::quote(&close_started.to_string_lossy()),
        ),
    );
    let context = CliContext {
        state_dir,
        host: None,
    };
    let record = load_session_record(&context, "abort").unwrap();
    let registry = AttachBrokerRegistry::default();
    let live_fifo = session_dir(&context, "abort").join(ATTACH_LIVE_FIFO_NAME);
    let subscription = registry.subscribe(&context, &tmux, &record).await.unwrap();

    let release_task = tokio::spawn(subscription.release());
    tokio::time::timeout(Duration::from_secs(1), async {
        while !close_started.exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("broker teardown never reached the blocking close command");
    release_task.abort();
    let _ = release_task.await;

    tokio::time::timeout(Duration::from_secs(2), async {
        while live_fifo.exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("supervised teardown did not remove the broker FIFO");
    assert_eq!(pipe_close_count(&calls), 1);

    let replacement = registry.subscribe(&context, &tmux, &record).await.unwrap();
    replacement.release().await;
    assert_eq!(pipe_close_count(&calls), 2);
}

#[tokio::test]
async fn slow_broker_start_does_not_block_an_unrelated_session() {
    let tmp = tempfile::TempDir::new().unwrap();
    let state_dir = tmp.path().join("state");
    let slow_started = tmp.path().join("slow-started");
    seed_session(&state_dir, "slow", "codex", "hs-codex-slow");
    seed_session(&state_dir, "fast", "codex", "hs-codex-fast");
    let tmux = executable(
        &tmp.path().join("tmux"),
        &format!(
            "#!/usr/bin/env sh\ncase \"$*\" in\n  *hs-codex-slow*\\ cat\\ \\>* ) : > {}; sleep 1 ;;\nesac\nexit 0\n",
            shell_words::quote(&slow_started.to_string_lossy())
        ),
    );
    let context = CliContext {
        state_dir,
        host: None,
    };
    let slow_record = load_session_record(&context, "slow").unwrap();
    let fast_record = load_session_record(&context, "fast").unwrap();
    let registry = Arc::new(AttachBrokerRegistry::default());

    let slow_task = tokio::spawn({
        let registry = registry.clone();
        let context = context.clone();
        let tmux = tmux.clone();
        async move {
            registry
                .subscribe(&context, &tmux, &slow_record)
                .await
                .unwrap()
        }
    });
    tokio::time::timeout(Duration::from_secs(1), async {
        while !slow_started.exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("slow session never began broker startup");

    let fast = tokio::time::timeout(
        Duration::from_millis(250),
        registry.subscribe(&context, &tmux, &fast_record),
    )
    .await
    .expect("an unrelated session was blocked by slow broker startup")
    .unwrap();
    fast.release().await;

    let slow = slow_task.await.unwrap();
    slow.release().await;
}

#[tokio::test]
async fn late_subscriber_restarts_a_closed_broker_generation() {
    let tmp = tempfile::TempDir::new().unwrap();
    let state_dir = tmp.path().join("state");
    let calls = tmp.path().join("calls.log");
    let source = tmp.path().join("pane-output.fifo");
    let writer_guard = FixtureWriterGuard::new(tmp.path(), &source);
    create_private_fifo(&source).unwrap();
    seed_session(&state_dir, "restart", "codex", "hs-codex-restart");

    let tmux = fanout_tmux(tmp.path(), &calls, &source, writer_guard.identity_dir());
    let context = CliContext {
        state_dir,
        host: None,
    };
    let record = load_session_record(&context, "restart").unwrap();
    let registry = AttachBrokerRegistry::default();
    let mut first = registry.subscribe(&context, &tmux, &record).await.unwrap();

    let mut source_writer = std::fs::OpenOptions::new()
        .write(true)
        .open(&source)
        .unwrap();
    source_writer.write_all(b"before-close").unwrap();
    source_writer.flush().unwrap();
    assert_eq!(recv_attach_bytes(&mut first).await, b"before-close");
    drop(source_writer);
    writer_guard.terminate_latest();

    let closed = tokio::time::timeout(Duration::from_secs(5), first.receiver_mut().recv())
        .await
        .expect("timed out waiting for the old broker to close");
    assert!(attach_event_bytes(closed).is_none());

    let mut second = registry.subscribe(&context, &tmux, &record).await.unwrap();
    let mut source_writer = std::fs::OpenOptions::new()
        .write(true)
        .open(&source)
        .unwrap();
    source_writer.write_all(b"after-restart").unwrap();
    source_writer.flush().unwrap();
    assert_eq!(recv_attach_bytes(&mut second).await, b"after-restart");

    first.release().await;
    second.release().await;
    let enables = std::fs::read_to_string(&calls)
        .unwrap()
        .lines()
        .filter(|line| line.contains("cat >"))
        .count();
    assert_eq!(enables, 2, "a closed generation must be replaced once");
    writer_guard.cleanup().expect("cleanup fixture writers");
}

async fn recv_attach_bytes(subscription: &mut AttachSubscription) -> Vec<u8> {
    let event = tokio::time::timeout(Duration::from_secs(2), subscription.receiver_mut().recv())
        .await
        .expect("timed out waiting for broker output");
    attach_event_bytes(event)
        .expect("broker closed before delivering output")
        .to_vec()
}

async fn recv_websocket_binary<S>(socket: &mut S) -> Vec<u8>
where
    S: futures_util::Stream<
            Item = Result<
                tokio_tungstenite::tungstenite::Message,
                tokio_tungstenite::tungstenite::Error,
            >,
        > + Unpin,
{
    let message = tokio::time::timeout(Duration::from_secs(2), socket.next())
        .await
        .expect("timed out waiting for WebSocket output")
        .expect("WebSocket closed before output")
        .expect("WebSocket output failed");
    match message {
        tokio_tungstenite::tungstenite::Message::Binary(bytes) => bytes.to_vec(),
        other => panic!("expected binary WebSocket output, got {other:?}"),
    }
}

async fn wait_for_subscriber_count(
    registry: &AttachBrokerRegistry,
    session_id: &str,
    expected: usize,
) {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if registry.subscriber_count(session_id).await == expected {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("timed out waiting for subscriber cleanup");
}

async fn wait_for_file(path: &Path) {
    tokio::time::timeout(Duration::from_secs(60), async {
        while !path.exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("timed out waiting for test marker");
}

fn pipe_close_count(calls: &Path) -> usize {
    std::fs::read_to_string(calls)
        .unwrap_or_default()
        .lines()
        .filter(|line| line.starts_with("pipe-pane") && !line.contains("cat >"))
        .count()
}

#[cfg(target_os = "linux")]
#[test]
fn fanout_fixture_close_reaps_only_the_exact_target_writer() {
    let tmp = tempfile::TempDir::new().unwrap();
    let calls = tmp.path().join("calls.log");
    let source = tmp.path().join("source.fifo");
    create_private_fifo(&source).unwrap();
    let guard = FixtureWriterGuard::new(tmp.path(), &source);
    let tmux = fanout_tmux(tmp.path(), &calls, &source, guard.identity_dir());

    for target in ["target-a", "target-b"] {
        let destination = tmp.path().join(format!("{target}.out"));
        let status = std::process::Command::new(&tmux)
            .args([
                "pipe-pane",
                "-t",
                target,
                &format!("cat > {}", destination.display()),
            ])
            .status()
            .unwrap();
        assert!(status.success());
    }
    let target_a = wait_for_fixture_target_identity(guard.identity_dir(), "target-a");
    let target_b = wait_for_fixture_target_identity(guard.identity_dir(), "target-b");
    assert!(fixture_process_matches(&target_a));
    assert!(fixture_process_matches(&target_b));

    let status = std::process::Command::new(&tmux)
        .args(["pipe-pane", "-t", "target-a"])
        .status()
        .unwrap();
    assert!(status.success());
    wait_for_fixture_process_exit(&target_a);

    assert!(
        !fixture_process_matches(&target_a),
        "closing target-a must reap its exact writer",
    );
    assert!(
        fixture_process_matches(&target_b),
        "closing target-a must not stop target-b's writer",
    );
    guard.cleanup().expect("cleanup fixture writers");
}

#[cfg(target_os = "linux")]
#[test]
fn fanout_fixture_close_is_fenced_to_the_captured_target_generation() {
    let tmp = tempfile::TempDir::new().unwrap();
    let calls = tmp.path().join("calls.log");
    let source = tmp.path().join("source.fifo");
    create_private_fifo(&source).unwrap();
    let guard = FixtureWriterGuard::new_with_pidfd_fault(
        tmp.path(),
        &source,
        FixturePidfdFault::ErrnoTimes {
            errno: libc::EIO,
            remaining: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(20)),
        },
    );
    let tmux = fanout_tmux(tmp.path(), &calls, &source, guard.identity_dir());
    let old_destination = tmp.path().join("old.out");
    let replacement_destination = tmp.path().join("replacement.out");
    arm_fixture_writer(&tmux, "same-target", &old_destination);
    let old = wait_for_fixture_target_record(
        guard.identity_dir(),
        "same-target",
        FixtureIdentityState::Ready,
    );

    close_fixture_writer(&tmux, "same-target");
    arm_fixture_writer(&tmux, "same-target", &replacement_destination);
    let replacement =
        wait_for_fixture_target_replacement(guard.identity_dir(), "same-target", &old);
    wait_for_fixture_pid_start_exit(&old.identity);
    assert!(!fixture_pid_start_matches(&old.identity));
    assert!(fixture_process_matches(&replacement.identity));

    let mut source_writer = std::fs::OpenOptions::new()
        .write(true)
        .open(&source)
        .unwrap();
    source_writer.write_all(b"replacement-live").unwrap();
    source_writer.flush().unwrap();
    wait_for_fixture_output(&replacement_destination, b"replacement-live");

    close_fixture_writer(&tmux, "same-target");
    wait_for_fixture_pid_start_exit(&replacement.identity);
    assert!(!fixture_pid_start_matches(&replacement.identity));
    guard.cleanup().expect("cleanup generation-fenced writers");
}

#[cfg(target_os = "linux")]
#[test]
fn fanout_fixture_pidfd_denial_falls_back_without_leaking() {
    for errno in [libc::ENOSYS, libc::EPERM] {
        let tmp = tempfile::TempDir::new().unwrap();
        let calls = tmp.path().join("calls.log");
        let source = tmp.path().join("source.fifo");
        create_private_fifo(&source).unwrap();
        let guard = FixtureWriterGuard::new_with_pidfd_fault(
            tmp.path(),
            &source,
            FixturePidfdFault::Errno(errno),
        );
        let tmux = fanout_tmux(tmp.path(), &calls, &source, guard.identity_dir());
        let destination_a = tmp.path().join("target-a.out");
        let destination_b = tmp.path().join("target-b.out");
        arm_fixture_writer(&tmux, "target-a", &destination_a);
        arm_fixture_writer(&tmux, "target-b", &destination_b);
        let target_a = wait_for_fixture_target_identity(guard.identity_dir(), "target-a");
        let target_b = wait_for_fixture_target_identity(guard.identity_dir(), "target-b");
        let mut source_writer = std::fs::OpenOptions::new()
            .write(true)
            .open(&source)
            .unwrap();

        close_fixture_writer(&tmux, "target-a");
        wait_for_fixture_pid_start_exit(&target_a);
        assert!(!fixture_pid_start_matches(&target_a));
        assert!(fixture_process_matches(&target_b));
        source_writer.write_all(b"fallback-b").unwrap();
        source_writer.flush().unwrap();
        wait_for_fixture_output(&destination_b, b"fallback-b");

        close_fixture_writer(&tmux, "target-b");
        wait_for_fixture_pid_start_exit(&target_b);
        assert!(!fixture_pid_start_matches(&target_b));
        guard.cleanup().expect("pidfd denial fallback cleanup");
    }
}

#[cfg(target_os = "linux")]
fn arm_fixture_writer(tmux: &Path, target: &str, destination: &Path) {
    assert!(
        std::process::Command::new(tmux)
            .args([
                "pipe-pane",
                "-t",
                target,
                &format!("cat > {}", destination.display()),
            ])
            .status()
            .unwrap()
            .success(),
    );
}

#[cfg(target_os = "linux")]
fn close_fixture_writer(tmux: &Path, target: &str) {
    assert!(
        std::process::Command::new(tmux)
            .args(["pipe-pane", "-t", target])
            .status()
            .unwrap()
            .success(),
    );
}

#[cfg(target_os = "linux")]
fn wait_for_fixture_output(path: &Path, expected: &[u8]) {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if std::fs::read(path).unwrap_or_default() == expected {
            return;
        }
        assert!(Instant::now() < deadline, "fixture output did not arrive");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(target_os = "linux")]
#[test]
fn fanout_fixture_cleanup_disarms_late_writer_launches() {
    let tmp = tempfile::TempDir::new().unwrap();
    let calls = tmp.path().join("calls.log");
    let source = tmp.path().join("source.fifo");
    create_private_fifo(&source).unwrap();
    let guard = FixtureWriterGuard::new(tmp.path(), &source);
    let tmux = fanout_tmux(tmp.path(), &calls, &source, guard.identity_dir());

    guard.cleanup().expect("disarm fixture writer launches");
    let destination = tmp.path().join("target.out");
    assert!(
        std::process::Command::new(&tmux)
            .args([
                "pipe-pane",
                "-t",
                "late-target",
                &format!("cat > {}", destination.display()),
            ])
            .status()
            .unwrap()
            .success(),
    );

    assert!(
        fixture_identity_records(guard.identity_dir())
            .unwrap()
            .is_empty()
    );
    assert!(
        fixture_identity_records(&guard.identity_dir().join("started"))
            .unwrap()
            .is_empty()
    );
    assert!(
        !fixture_fifo_has_reader(&source),
        "cleanup must prevent a delayed tmux fixture arm from opening the source FIFO",
    );
}

#[cfg(target_os = "linux")]
#[test]
fn fanout_fixture_cleanup_tracks_a_paused_pre_exec_writer_until_exit() {
    let tmp = tempfile::TempDir::new().unwrap();
    let calls = tmp.path().join("calls.log");
    let source = tmp.path().join("source.fifo");
    let pre_exec_started = tmp.path().join("pre-exec-started");
    let pre_exec_release = tmp.path().join("pre-exec-release");
    create_private_fifo(&source).unwrap();
    let guard = FixtureWriterGuard::new(tmp.path(), &source);
    let tmux = fanout_tmux_with_pre_exec_gate(
        tmp.path(),
        &calls,
        &source,
        guard.identity_dir(),
        Some(&pre_exec_started),
        Some(&pre_exec_release),
    );
    let destination = tmp.path().join("target.out");
    let launch_tmux = tmux.clone();
    let launch = std::thread::spawn(move || {
        std::process::Command::new(&launch_tmux)
            .args([
                "pipe-pane",
                "-t",
                "target",
                &format!("cat > {}", destination.display()),
            ])
            .status()
            .unwrap()
    });
    let deadline = Instant::now() + Duration::from_secs(2);
    while !pre_exec_started.exists() {
        assert!(Instant::now() < deadline, "pre-exec gate did not start");
        std::thread::sleep(Duration::from_millis(10));
    }
    let initializing = wait_for_fixture_target_record(
        guard.identity_dir(),
        "target",
        FixtureIdentityState::Initializing,
    );
    assert!(fixture_pid_start_matches(&initializing.identity));
    assert!(!fixture_process_matches(&initializing.identity));

    assert!(
        std::process::Command::new(&tmux)
            .args(["pipe-pane", "-t", "target"])
            .status()
            .unwrap()
            .success()
    );
    assert!(launch.join().unwrap().success());
    wait_for_fixture_pid_start_exit(&initializing.identity);
    assert!(!fixture_pid_start_matches(&initializing.identity));
    guard
        .wait_for_started_and_reaped(1)
        .expect("paused pre-exec writer must be recorded and reaped");
    guard.cleanup().expect("cleanup paused pre-exec writer");
}

#[cfg(target_os = "linux")]
fn wait_for_fixture_target_identity(
    identity_dir: &Path,
    expected_target: &str,
) -> FixtureProcessIdentity {
    wait_for_fixture_target_record(identity_dir, expected_target, FixtureIdentityState::Ready)
        .identity
}

#[cfg(target_os = "linux")]
fn wait_for_fixture_target_record(
    identity_dir: &Path,
    expected_target: &str,
    expected_state: FixtureIdentityState,
) -> FixtureIdentityRecord {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        for record in fixture_identity_records(identity_dir).unwrap() {
            if record.target == expected_target && record.state == expected_state {
                return record;
            }
        }
        assert!(
            Instant::now() < deadline,
            "fixture writer for {expected_target} did not become ready",
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(target_os = "linux")]
fn wait_for_fixture_target_replacement(
    identity_dir: &Path,
    expected_target: &str,
    previous: &FixtureIdentityRecord,
) -> FixtureIdentityRecord {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        for record in fixture_identity_records(identity_dir).unwrap() {
            if record.target == expected_target
                && record.state == FixtureIdentityState::Ready
                && record.generation != previous.generation
            {
                return record;
            }
        }
        assert!(
            Instant::now() < deadline,
            "replacement fixture writer for {expected_target} did not become ready",
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(target_os = "linux")]
#[test]
fn fixture_signal_rejects_a_reused_pid_identity() {
    let tmp = tempfile::TempDir::new().unwrap();
    let source = tmp.path().join("identity-source.fifo");
    create_private_fifo(&source).unwrap();
    let mut child = FixtureChildGuard::spawn_cat(&source);
    let identity = child.identity().clone();
    let mismatched = FixtureProcessIdentity {
        start_token: format!("{}-reused", identity.start_token),
        ..identity.clone()
    };

    assert!(!signal_fixture_process_if_matches(&mismatched, libc::SIGTERM).unwrap());
    assert!(
        child.is_running(),
        "identity mismatch must not signal the process occupying that PID",
    );

    assert!(signal_fixture_process_if_matches(&identity, libc::SIGTERM).unwrap());
    child.wait();
}

#[cfg(target_os = "linux")]
#[test]
fn fixture_child_guard_reaps_after_injected_signal_failure() {
    for errno in [libc::ENOSYS, libc::EPERM] {
        let tmp = tempfile::TempDir::new().unwrap();
        let source = tmp.path().join("identity-source.fifo");
        create_private_fifo(&source).unwrap();
        let identity = {
            let child = FixtureChildGuard::spawn_cat(&source);
            let identity = child.identity().clone();
            let error = signal_fixture_process_if_matches_with_fault(
                &identity,
                libc::SIGTERM,
                FixturePidfdFault::Errno(errno),
            )
            .unwrap_err();
            assert_eq!(error.raw_os_error(), Some(errno));
            identity
        };
        assert!(!fixture_pid_start_matches(&identity));
    }
}

#[cfg(target_os = "linux")]
#[test]
fn fixture_child_guard_bounds_a_non_exiting_child() {
    let started = Instant::now();
    let identity = {
        let child = FixtureChildGuard::spawn_non_exiting();
        child.identity().clone()
    };
    assert!(started.elapsed() < Duration::from_secs(1));
    assert!(!fixture_pid_start_matches(&identity));
}

#[cfg(target_os = "linux")]
#[test]
fn fixture_cleanup_reports_an_unhandled_signal_failure_without_leaking() {
    let tmp = tempfile::TempDir::new().unwrap();
    let source = tmp.path().join("identity-source.fifo");
    let unrelated_source = tmp.path().join("guard-source.fifo");
    create_private_fifo(&source).unwrap();
    create_private_fifo(&unrelated_source).unwrap();
    let guard = FixtureWriterGuard::new_with_pidfd_fault(
        tmp.path(),
        &unrelated_source,
        FixturePidfdFault::Errno(libc::EIO),
    );
    let child = FixtureChildGuard::spawn_cat(&source);
    let identity = child.identity();
    let identity_path = guard.identity_dir().join(format!(
        "{}.{}.identity",
        identity.pid, identity.start_token
    ));
    let stop_path = guard.identity_dir().join("synthetic.stop");
    std::fs::write(
        identity_path,
        format!(
            "{}\n{}\n{}\n{}\ntarget\nready\n\ngeneration\n{}\n",
            identity.pid,
            identity.start_token,
            identity.command_token,
            identity.source_token.display(),
            stop_path.display(),
        ),
    )
    .unwrap();

    let error = guard.cleanup().unwrap_err();
    assert_eq!(error.raw_os_error(), Some(libc::EIO));
    assert!(fixture_process_matches(identity));
}

#[cfg(target_os = "linux")]
#[test]
fn fanout_fixture_cleanup_surfaces_a_terminal_monitor_failure() {
    let tmp = tempfile::TempDir::new().unwrap();
    let calls = tmp.path().join("calls.log");
    let source = tmp.path().join("source.fifo");
    create_private_fifo(&source).unwrap();
    let guard = FixtureWriterGuard::new_with_pidfd_fault(
        tmp.path(),
        &source,
        FixturePidfdFault::Errno(libc::EIO),
    );
    let tmux = fanout_tmux(tmp.path(), &calls, &source, guard.identity_dir());
    let destination = tmp.path().join("target.out");
    arm_fixture_writer(&tmux, "target", &destination);
    let identity = wait_for_fixture_target_identity(guard.identity_dir(), "target");
    close_fixture_writer(&tmux, "target");

    let deadline = Instant::now() + Duration::from_secs(3);
    while guard.monitor_error.lock().unwrap().is_none() {
        assert!(
            Instant::now() < deadline,
            "fixture close monitor failure did not become observable"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let error = guard.cleanup().unwrap_err();
    assert!(error.to_string().contains("fixture close request failed"));
    wait_for_fixture_pid_start_exit(&identity);
    assert!(!fixture_pid_start_matches(&identity));
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct FixtureProcessIdentity {
    pid: libc::pid_t,
    start_token: String,
    command_token: String,
    source_token: PathBuf,
}

#[cfg(target_os = "linux")]
struct FixtureChildGuard {
    child: Option<std::process::Child>,
    identity: FixtureProcessIdentity,
    release_source: Option<PathBuf>,
}

#[cfg(target_os = "linux")]
impl FixtureChildGuard {
    fn spawn_cat(source: &Path) -> Self {
        let child = std::process::Command::new("cat")
            .arg(source)
            .spawn()
            .expect("spawn identity fixture");
        let pid = child.id() as libc::pid_t;
        let deadline = Instant::now() + Duration::from_secs(2);
        let identity = loop {
            let start_token = fixture_process_start_token(pid).expect("fixture start token");
            let identity = FixtureProcessIdentity {
                pid,
                start_token,
                command_token: "cat".to_string(),
                source_token: source.to_path_buf(),
            };
            if fixture_process_matches(&identity) {
                break identity;
            }
            assert!(
                Instant::now() < deadline,
                "fixture cat did not become ready"
            );
            std::thread::sleep(Duration::from_millis(10));
        };
        Self {
            child: Some(child),
            identity,
            release_source: Some(source.to_path_buf()),
        }
    }

    fn spawn_non_exiting() -> Self {
        let child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("spawn non-exiting fixture");
        let pid = child.id() as libc::pid_t;
        let identity = FixtureProcessIdentity {
            pid,
            start_token: fixture_process_start_token(pid).expect("fixture start token"),
            command_token: "sleep".to_string(),
            source_token: PathBuf::from("30"),
        };
        Self {
            child: Some(child),
            identity,
            release_source: None,
        }
    }

    fn identity(&self) -> &FixtureProcessIdentity {
        &self.identity
    }

    fn is_running(&mut self) -> bool {
        self.child.as_mut().unwrap().try_wait().unwrap().is_none()
    }

    fn wait(&mut self) {
        self.child
            .take()
            .unwrap()
            .wait()
            .expect("reap fixture child");
    }
}

#[cfg(target_os = "linux")]
impl Drop for FixtureChildGuard {
    fn drop(&mut self) {
        let Some(mut child) = self.child.take() else {
            return;
        };
        let deadline = Instant::now() + Duration::from_millis(250);
        loop {
            if child.try_wait().ok().flatten().is_some() {
                return;
            }
            if let Some(source) = &self.release_source {
                release_fixture_fifo_readers(source);
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

#[derive(Clone, Debug)]
enum FixturePidfdFault {
    None,
    #[cfg(target_os = "linux")]
    Errno(i32),
    #[cfg(target_os = "linux")]
    ErrnoTimes {
        errno: i32,
        remaining: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FixtureIdentityState {
    Initializing,
    Ready,
}

#[derive(Clone, Debug)]
struct FixtureIdentityRecord {
    identity: FixtureProcessIdentity,
    target: String,
    state: FixtureIdentityState,
    init_release: Option<PathBuf>,
    generation: String,
    stop_path: PathBuf,
    path: PathBuf,
}

struct FixtureWriterGuard {
    identity_dir: PathBuf,
    monitor_stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    monitor: Option<std::thread::JoinHandle<()>>,
    monitor_error: std::sync::Arc<std::sync::Mutex<Option<String>>>,
    pidfd_fault: FixturePidfdFault,
}

impl FixtureWriterGuard {
    fn new(dir: &Path, source: &Path) -> Self {
        Self::new_with_pidfd_fault(dir, source, FixturePidfdFault::None)
    }

    fn new_with_pidfd_fault(dir: &Path, _source: &Path, pidfd_fault: FixturePidfdFault) -> Self {
        let identity_dir = dir.join("fixture-writer-identities");
        std::fs::create_dir(&identity_dir).expect("create fixture identity directory");
        std::fs::create_dir(identity_dir.join("started"))
            .expect("create fixture identity history directory");
        std::fs::create_dir(identity_dir.join("targets"))
            .expect("create fixture target identity directory");
        let monitor_stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let monitor_error = std::sync::Arc::new(std::sync::Mutex::new(None));
        let monitor = std::thread::spawn({
            let identity_dir = identity_dir.clone();
            let monitor_stop = monitor_stop.clone();
            let monitor_error = monitor_error.clone();
            let pidfd_fault = pidfd_fault.clone();
            move || fixture_close_monitor(&identity_dir, &monitor_stop, &monitor_error, pidfd_fault)
        });
        Self {
            identity_dir,
            monitor_stop,
            monitor: Some(monitor),
            monitor_error,
            pidfd_fault,
        }
    }

    fn identity_dir(&self) -> &Path {
        &self.identity_dir
    }

    #[cfg(target_os = "linux")]
    fn wait_for_started_and_reaped(&self, expected: usize) -> io::Result<()> {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let started = fixture_identity_records(&self.identity_dir.join("started"))?;
            if started.len() == expected
                && started.iter().all(|record| {
                    !fixture_pid_start_matches(&record.identity)
                        && fixture_supervisor_done(&self.identity_dir, record)
                })
            {
                return Ok(());
            }
            if Instant::now() >= deadline {
                let live = started
                    .iter()
                    .filter(|record| fixture_pid_start_matches(&record.identity))
                    .count();
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!(
                        "expected {expected} fixture writers to start and exit; recorded {}, live {live}",
                        started.len()
                    ),
                ));
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn terminate_latest(&self) {
        let records =
            fixture_identity_records(&self.identity_dir).expect("read fixture writer identities");
        let Some(record) = records.last() else { return };
        stop_fixture_record(record, self.pidfd_fault.clone()).expect("terminate fixture writer");
    }

    fn cleanup(&self) -> io::Result<()> {
        std::fs::write(self.identity_dir.join("cleanup.requested"), b"")?;
        wait_for_fixture_monitor_idle(&self.identity_dir)?;
        let monitor_error = self.monitor_error.lock().unwrap().clone();
        let cleanup_result =
            cleanup_fixture_records(&self.identity_dir, None, self.pidfd_fault.clone());
        let supervisor_result = wait_for_fixture_supervisors(&self.identity_dir);
        if let Some(error) = monitor_error {
            return Err(io::Error::other(error));
        }
        cleanup_result?;
        supervisor_result
    }
}

impl Drop for FixtureWriterGuard {
    fn drop(&mut self) {
        let _ = self.cleanup();
        self.monitor_stop
            .store(true, std::sync::atomic::Ordering::Release);
        if let Some(monitor) = self.monitor.take() {
            let _ = monitor.join();
        }
    }
}

fn fixture_close_monitor(
    identity_dir: &Path,
    stop: &std::sync::atomic::AtomicBool,
    monitor_error: &std::sync::Mutex<Option<String>>,
    pidfd_fault: FixturePidfdFault,
) {
    let mut pending_since = HashMap::<PathBuf, Instant>::new();
    while !stop.load(std::sync::atomic::Ordering::Acquire) {
        let requests = fixture_close_request_paths(identity_dir).unwrap_or_default();
        pending_since.retain(|path, _| requests.contains(path));
        for request in requests {
            let since = *pending_since
                .entry(request.clone())
                .or_insert_with(Instant::now);
            let outcome =
                process_fixture_close_request(identity_dir, &request, pidfd_fault.clone());
            let timed_out = since.elapsed() >= Duration::from_secs(2);
            match outcome {
                Ok(true) => {
                    let _ = std::fs::remove_file(&request);
                    pending_since.remove(&request);
                }
                Ok(false) if !timed_out => {}
                Err(_) if !timed_out => {}
                Ok(false) => {
                    monitor_error.lock().unwrap().get_or_insert_with(|| {
                        format!(
                            "fixture close request did not resolve an exact published writer: {}",
                            request.display()
                        )
                    });
                    let _ = std::fs::remove_file(&request);
                    pending_since.remove(&request);
                }
                Err(error) => {
                    monitor_error
                        .lock()
                        .unwrap()
                        .get_or_insert_with(|| format!("fixture close request failed: {error}"));
                    let _ = std::fs::remove_file(&request);
                    pending_since.remove(&request);
                }
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn fixture_close_request_paths(identity_dir: &Path) -> io::Result<Vec<PathBuf>> {
    let mut requests = Vec::new();
    for entry in std::fs::read_dir(identity_dir)? {
        let path = entry?.path();
        if path.extension().and_then(|value| value.to_str()) == Some("request") {
            requests.push(path);
        }
    }
    requests.sort();
    Ok(requests)
}

fn process_fixture_close_request(
    identity_dir: &Path,
    request: &Path,
    pidfd_fault: FixturePidfdFault,
) -> io::Result<bool> {
    let contents = std::fs::read_to_string(request)?;
    let mut fields = contents.lines();
    let target = fields
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing close target"))?;
    let generation = fields
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing close generation"))?;
    let exact = Some((target, generation));
    let records: Vec<_> = fixture_identity_records(identity_dir)?
        .into_iter()
        .filter(|record| record.target == target && record.generation == generation)
        .collect();
    if records.is_empty() {
        let historical: Vec<_> = fixture_identity_records(&identity_dir.join("started"))?
            .into_iter()
            .filter(|record| record.target == target && record.generation == generation)
            .collect();
        return Ok(!historical.is_empty()
            && historical
                .iter()
                .all(|record| !fixture_pid_start_matches(&record.identity)));
    }
    cleanup_fixture_records(identity_dir, exact, pidfd_fault)?;
    Ok(true)
}

fn wait_for_fixture_monitor_idle(identity_dir: &Path) -> io::Result<()> {
    let deadline = Instant::now() + Duration::from_secs(3);
    while !fixture_close_request_paths(identity_dir)?.is_empty() {
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "fixture close monitor did not finish pending requests",
            ));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    Ok(())
}

fn fixture_supervisor_done(identity_dir: &Path, record: &FixtureIdentityRecord) -> bool {
    identity_dir
        .join(format!("{}.done", record.generation))
        .exists()
}

fn wait_for_fixture_supervisors(identity_dir: &Path) -> io::Result<()> {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let started = fixture_identity_records(&identity_dir.join("started"))?;
        if started
            .iter()
            .all(|record| fixture_supervisor_done(identity_dir, record))
        {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "fixture writer supervisors did not exit",
            ));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn fixture_identity_records(dir: &Path) -> io::Result<Vec<FixtureIdentityRecord>> {
    let mut records = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("identity") {
            continue;
        }
        let contents = std::fs::read_to_string(&path)?;
        let fields: Vec<_> = contents.lines().collect();
        if fields.len() < 9 {
            continue;
        }
        let Ok(pid) = fields[0].parse() else {
            continue;
        };
        let state = match fields[5] {
            "initializing" => FixtureIdentityState::Initializing,
            "ready" => FixtureIdentityState::Ready,
            _ => continue,
        };
        records.push(FixtureIdentityRecord {
            identity: FixtureProcessIdentity {
                pid,
                start_token: fields[1].to_string(),
                command_token: fields[2].to_string(),
                source_token: PathBuf::from(fields[3]),
            },
            target: fields[4].to_string(),
            state,
            init_release: fields
                .get(6)
                .filter(|value| !value.is_empty())
                .map(PathBuf::from),
            generation: fields[7].to_string(),
            stop_path: PathBuf::from(fields[8]),
            path,
        });
    }
    records.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(records)
}

fn cleanup_fixture_records(
    identity_dir: &Path,
    exact: Option<(&str, &str)>,
    pidfd_fault: FixturePidfdFault,
) -> io::Result<()> {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let records: Vec<_> = fixture_identity_records(identity_dir)?
            .into_iter()
            .filter(|record| {
                exact.is_none_or(|(target, generation)| {
                    record.target == target && record.generation == generation
                })
            })
            .collect();
        if records.is_empty() {
            return Ok(());
        }
        for record in &records {
            if !fixture_pid_start_matches(&record.identity) {
                let _ = std::fs::remove_file(&record.path);
                continue;
            }
            match record.state {
                FixtureIdentityState::Initializing => {
                    if let Some(release) = &record.init_release {
                        let _ = std::fs::write(release, b"");
                    }
                }
                FixtureIdentityState::Ready => {
                    stop_fixture_record(record, pidfd_fault.clone())?;
                }
            }
        }
        if Instant::now() >= deadline {
            let live = fixture_identity_records(identity_dir)?
                .into_iter()
                .filter(|record| {
                    exact.is_none_or(|(target, generation)| {
                        record.target == target && record.generation == generation
                    })
                })
                .filter(|record| fixture_pid_start_matches(&record.identity))
                .count();
            if live == 0 {
                return Ok(());
            }
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("{live} fixture writer identities remain live"),
            ));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn stop_fixture_record(
    record: &FixtureIdentityRecord,
    pidfd_fault: FixturePidfdFault,
) -> io::Result<()> {
    #[cfg(not(target_os = "linux"))]
    let _ = pidfd_fault;
    if record.state != FixtureIdentityState::Ready {
        return Ok(());
    }
    #[cfg(target_os = "linux")]
    let signal_result = signal_fixture_process_if_matches_with_fault(
        &record.identity,
        libc::SIGTERM,
        pidfd_fault.clone(),
    );
    #[cfg(not(target_os = "linux"))]
    let signal_result: io::Result<bool> = Err(io::Error::from_raw_os_error(libc::ENOSYS));
    match signal_result {
        Ok(_) => {}
        Err(error)
            if matches!(
                error.raw_os_error(),
                Some(libc::ENOSYS) | Some(libc::EPERM) | Some(libc::EACCES)
            ) =>
        {
            std::fs::write(&record.stop_path, b"")?;
        }
        Err(error) => return Err(error),
    }
    wait_for_fixture_pid_start_exit(&record.identity);
    if fixture_process_matches(&record.identity) {
        #[cfg(target_os = "linux")]
        signal_fixture_process_if_matches_with_fault(&record.identity, libc::SIGKILL, pidfd_fault)?;
        #[cfg(not(target_os = "linux"))]
        std::fs::write(&record.stop_path, b"")?;
        wait_for_fixture_pid_start_exit(&record.identity);
    }
    if fixture_pid_start_matches(&record.identity) {
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "fixture writer identity remains live",
        ));
    }
    let _ = std::fs::remove_file(&record.path);
    Ok(())
}

#[cfg(target_os = "linux")]
fn release_fixture_fifo_readers(source: &Path) {
    let _ = std::fs::OpenOptions::new()
        .write(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(source);
}

#[cfg(target_os = "linux")]
fn fixture_fifo_has_reader(source: &Path) -> bool {
    match std::fs::OpenOptions::new()
        .write(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(source)
    {
        Ok(_) => true,
        Err(error) if error.raw_os_error() == Some(libc::ENXIO) => false,
        Err(error) => panic!("probe fixture FIFO reader: {error}"),
    }
}

#[cfg(target_os = "linux")]
fn fixture_process_start_token(pid: libc::pid_t) -> Option<String> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let after_command = stat.get(stat.rfind(')')? + 2..)?;
    Some(after_command.split_ascii_whitespace().nth(19)?.to_string())
}

#[cfg(not(target_os = "linux"))]
fn fixture_process_start_token(pid: libc::pid_t) -> Option<String> {
    let output = std::process::Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "lstart="])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8(output.stdout).ok()?;
    let value = value.split_ascii_whitespace().collect::<Vec<_>>().join("_");
    (!value.is_empty()).then_some(value)
}

fn fixture_pid_start_matches(identity: &FixtureProcessIdentity) -> bool {
    fixture_process_start_token(identity.pid).as_deref() == Some(&identity.start_token)
}

fn fixture_process_matches(identity: &FixtureProcessIdentity) -> bool {
    if !fixture_pid_start_matches(identity) {
        return false;
    }

    #[cfg(target_os = "linux")]
    {
        let proc_dir = PathBuf::from(format!("/proc/{}", identity.pid));
        let Ok(command) = std::fs::read(proc_dir.join("comm")) else {
            return false;
        };
        if command.strip_suffix(b"\n") != Some(identity.command_token.as_bytes()) {
            return false;
        }
        let Ok(cmdline) = std::fs::read(proc_dir.join("cmdline")) else {
            return false;
        };
        cmdline
            .split(|byte| *byte == 0)
            .any(|arg| arg == identity.source_token.as_os_str().as_bytes())
            && fixture_process_start_token(identity.pid).as_deref() == Some(&identity.start_token)
    }

    #[cfg(not(target_os = "linux"))]
    {
        let pid = identity.pid.to_string();
        let Ok(command) = std::process::Command::new("ps")
            .args(["-p", &pid, "-o", "comm="])
            .output()
        else {
            return false;
        };
        let command = String::from_utf8_lossy(&command.stdout);
        if Path::new(command.trim())
            .file_name()
            .and_then(|name| name.to_str())
            != Some(&identity.command_token)
        {
            return false;
        }
        let Ok(arguments) = std::process::Command::new("ps")
            .args(["-ww", "-p", &pid, "-o", "command="])
            .output()
        else {
            return false;
        };
        let arguments = String::from_utf8_lossy(&arguments.stdout);
        let source = identity.source_token.to_string_lossy();
        arguments.contains(source.as_ref())
            && fixture_process_start_token(identity.pid).as_deref() == Some(&identity.start_token)
    }
}

#[cfg(target_os = "linux")]
fn signal_fixture_process_if_matches(
    identity: &FixtureProcessIdentity,
    signal: libc::c_int,
) -> io::Result<bool> {
    signal_fixture_process_if_matches_with_fault(identity, signal, FixturePidfdFault::None)
}

#[cfg(target_os = "linux")]
fn signal_fixture_process_if_matches_with_fault(
    identity: &FixtureProcessIdentity,
    signal: libc::c_int,
    fault: FixturePidfdFault,
) -> io::Result<bool> {
    if !fixture_process_matches(identity) {
        return Ok(false);
    }
    match fault {
        FixturePidfdFault::None => {}
        FixturePidfdFault::Errno(errno) => {
            return Err(io::Error::from_raw_os_error(errno));
        }
        FixturePidfdFault::ErrnoTimes { errno, remaining } => {
            if remaining
                .try_update(
                    std::sync::atomic::Ordering::AcqRel,
                    std::sync::atomic::Ordering::Acquire,
                    |value| value.checked_sub(1),
                )
                .is_ok()
            {
                return Err(io::Error::from_raw_os_error(errno));
            }
        }
    }
    // SAFETY: pidfd_open binds the handle to the process occupying this PID.
    let pidfd = unsafe { libc::syscall(libc::SYS_pidfd_open, identity.pid, 0) };
    if pidfd < 0 {
        let error = io::Error::last_os_error();
        return if error.raw_os_error() == Some(libc::ESRCH) {
            Ok(false)
        } else {
            Err(error)
        };
    }
    // SAFETY: pidfd was returned by pidfd_open and is closed on every path.
    let pidfd = unsafe {
        <std::os::fd::OwnedFd as std::os::fd::FromRawFd>::from_raw_fd(pidfd as std::os::fd::RawFd)
    };
    if !fixture_process_matches(identity) {
        return Ok(false);
    }
    // SAFETY: the second identity check above proved that pidfd is bound to
    // the recorded start, command, and exact source tokens. Signaling the
    // stable handle cannot be redirected by subsequent PID reuse.
    let result = unsafe {
        libc::syscall(
            libc::SYS_pidfd_send_signal,
            std::os::fd::AsRawFd::as_raw_fd(&pidfd),
            signal,
            std::ptr::null::<libc::siginfo_t>(),
            0,
        )
    };
    if result == 0 {
        return Ok(true);
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ESRCH) {
        Ok(false)
    } else {
        Err(error)
    }
}

#[cfg(target_os = "linux")]
fn wait_for_fixture_process_exit(identity: &FixtureProcessIdentity) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while fixture_process_matches(identity) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn wait_for_fixture_pid_start_exit(identity: &FixtureProcessIdentity) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while fixture_pid_start_matches(identity) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn fanout_tmux(dir: &Path, calls: &Path, source: &Path, identity_dir: &Path) -> PathBuf {
    fanout_tmux_with_pre_exec_gate(dir, calls, source, identity_dir, None, None)
}

fn fanout_tmux_with_pre_exec_gate(
    dir: &Path,
    calls: &Path,
    source: &Path,
    identity_dir: &Path,
    pre_exec_started: Option<&Path>,
    pre_exec_release: Option<&Path>,
) -> PathBuf {
    let pre_exec_started = pre_exec_started.unwrap_or_else(|| Path::new(""));
    let pre_exec_release = pre_exec_release.unwrap_or_else(|| Path::new(""));
    let calls_q = shell_words::quote(&calls.to_string_lossy()).into_owned();
    let source_q = shell_words::quote(&source.to_string_lossy()).into_owned();
    let identity_dir_q = shell_words::quote(&identity_dir.to_string_lossy()).into_owned();
    let pre_exec_started_q = shell_words::quote(&pre_exec_started.to_string_lossy()).into_owned();
    let pre_exec_release_q = shell_words::quote(&pre_exec_release.to_string_lossy()).into_owned();
    executable(
        &dir.join("tmux"),
        &format!(
            r#"#!/usr/bin/env sh
printf '%s\n' "$*" >> {calls_q}
target=''
previous=''
for arg in "$@"; do
  if [ "$previous" = '-t' ]; then target=$arg; fi
  previous=$arg
done
pre_exec_started={pre_exec_started_q}
pre_exec_release={pre_exec_release_q}
fixture_identity_dir={identity_dir_q}
fixture_source={source_q}
fixture_cleanup_requested() {{
  [ ! -d "$fixture_identity_dir" ] || [ -e "$fixture_identity_dir/cleanup.requested" ]
}}
fixture_start_token() {{
  fixture_pid=$1
  if [ -r "/proc/$fixture_pid/stat" ]; then
    awk '{{print $22}}' "/proc/$fixture_pid/stat" 2>/dev/null
  else
    ps -p "$fixture_pid" -o lstart= 2>/dev/null |
      awk '{{$1=$1; gsub(/ /, "_"); print}}'
  fi
}}
fixture_writer_ready() {{
  [ "$(fixture_start_token "$writer_pid")" = "$writer_start" ] || return 1
  if [ -r "/proc/$writer_pid/comm" ]; then
    [ "$(cat "/proc/$writer_pid/comm" 2>/dev/null)" = cat ] &&
    tr '\000' '\n' < "/proc/$writer_pid/cmdline" 2>/dev/null | grep -Fqx -- "$fixture_source"
  else
    [ "$(ps -p "$writer_pid" -o comm= 2>/dev/null | awk -F/ '{{print $NF}}' | awk '{{$1=$1; print}}')" = cat ] &&
    ps -ww -p "$writer_pid" -o command= 2>/dev/null | grep -Fq -- "$fixture_source"
  fi
}}
write_identity() {{
  identity_state=$1
  identity_tmp=$identity_file.tmp.$$
  printf '%s\n%s\n%s\n%s\n%s\n%s\n%s\n%s\n%s\n' \
    "$writer_pid" "$writer_start" cat "$fixture_source" "$target" "$identity_state" \
    "$pre_exec_release" "$generation" "$stop_path" \
    > "$identity_tmp"
  mv "$identity_tmp" "$identity_file"
}}
if [ "$1" = "capture-pane" ]; then
  printf 'pane\n'
  exit 0
fi
if [ "$1" = "pipe-pane" ]; then
  last=""
  for arg in "$@"; do last="$arg"; done
  launcher_start=$(fixture_start_token $$ || true)
  generation=$$.$launcher_start
  current_file=$fixture_identity_dir/targets/$target.current
  case "$last" in
    "cat > "*)
      if fixture_cleanup_requested; then exit 0; fi
      dest=${{last#cat > }}
      current_tmp=$current_file.tmp.$$
      printf '%s\n%s\n' "$generation" "$target" > "$current_tmp"
      mv "$current_tmp" "$current_file"
      stop_path=$fixture_identity_dir/$generation.stop
      arm_ready=$fixture_identity_dir/$generation.ready
      arm_failed=$fixture_identity_dir/$generation.failed
      supervisor_done=$fixture_identity_dir/$generation.done
      (
      (
        if [ -n "$pre_exec_started" ]; then
          : > "$pre_exec_started"
          while [ ! -e "$pre_exec_release" ]; do sleep 0.01; done
        fi
        exec 3>"$dest"
        exec cat "$fixture_source" >&3
      ) &
      writer_pid=$!
      writer_start=''
      attempts=0
      while [ -z "$writer_start" ] && [ "$attempts" -lt 100 ]; do
        writer_start=$(fixture_start_token "$writer_pid" || true)
        attempts=$((attempts + 1))
      done
        if [ -z "$writer_start" ]; then
          [ -n "$pre_exec_release" ] && : > "$pre_exec_release"
          : > "$arm_failed"
          wait "$writer_pid" 2>/dev/null || true
          : > "$supervisor_done"
          exit 1
      fi
      identity_file=$fixture_identity_dir/$writer_pid.$writer_start.identity
      write_identity initializing
      attempts=0
      while ! fixture_writer_ready && [ "$attempts" -lt 200 ]; do
        if fixture_cleanup_requested || [ -e "$stop_path" ]; then
          [ -n "$pre_exec_release" ] && : > "$pre_exec_release"
        fi
        sleep 0.01
        attempts=$((attempts + 1))
      done
      if ! fixture_writer_ready; then
        [ -n "$pre_exec_release" ] && : > "$pre_exec_release"
          : > "$arm_failed"
          wait "$writer_pid" 2>/dev/null || true
          rm -f "$identity_file"
          : > "$supervisor_done"
          exit 1
      fi
      write_identity ready
      cp "$identity_file" "$fixture_identity_dir/started/$writer_pid.$writer_start.identity"
      : > "$arm_ready"
      while fixture_writer_ready; do
        if fixture_cleanup_requested || [ -e "$stop_path" ]; then
          if fixture_writer_ready; then
            kill -TERM "$writer_pid" 2>/dev/null || true
          fi
        fi
        sleep 0.01
      done
      wait "$writer_pid" 2>/dev/null || true
      rm -f "$identity_file" "$stop_path"
      : > "$supervisor_done"
      ) </dev/null >/dev/null 2>&1 &
      attempts=0
      while [ ! -e "$arm_ready" ] && [ ! -e "$arm_failed" ] && [ "$attempts" -lt 250 ]; do
        sleep 0.01
        attempts=$((attempts + 1))
      done
      if [ -e "$arm_ready" ]; then
        rm -f "$arm_ready"
      else
        : > "$stop_path"
        rm -f "$arm_failed"
        exit 1
      fi
      ;;
    *)
      if [ -r "$current_file" ]; then
        close_generation=$(sed -n '1p' "$current_file")
        close_target=$(sed -n '2p' "$current_file")
        if [ "$close_target" = "$target" ] && [ -n "$close_generation" ]; then
          request_tmp=$fixture_identity_dir/close.$generation.request.tmp
          request_file=$fixture_identity_dir/close.$generation.request
          printf '%s\n%s\n' "$target" "$close_generation" > "$request_tmp"
          mv "$request_tmp" "$request_file"
        fi
      fi
      ;;
  esac
fi
exit 0
"#,
        ),
    )
}

fn snapshot_handoff_tmux(
    dir: &Path,
    calls: &Path,
    fifo_destination: &Path,
    writer_pid: &Path,
    capture_started: &Path,
    capture_release: &Path,
) -> PathBuf {
    executable(
        &dir.join("tmux"),
        &format!(
            r#"#!/usr/bin/env sh
printf '%s\n' "$*" >> {}
if [ "$1" = "capture-pane" ]; then
  : > {}
  while [ ! -e {} ]; do sleep 0.01; done
  printf 'pane\n'
  exit 0
fi
if [ "$1" = "pipe-pane" ]; then
  last=""
  for arg in "$@"; do last="$arg"; done
  case "$last" in
    "cat > "*)
      dest=${{last#cat > }}
      printf '%s' "$dest" > {}
      (exec 3>"$dest"; exec sleep 30) &
      printf '%s\n' "$!" > {}
      ;;
    *)
      if [ -s {} ]; then
        kill "$(cat {})" 2>/dev/null || true
        rm -f {}
      fi
      ;;
  esac
fi
exit 0
"#,
            shell_words::quote(&calls.to_string_lossy()),
            shell_words::quote(&capture_started.to_string_lossy()),
            shell_words::quote(&capture_release.to_string_lossy()),
            shell_words::quote(&fifo_destination.to_string_lossy()),
            shell_words::quote(&writer_pid.to_string_lossy()),
            shell_words::quote(&writer_pid.to_string_lossy()),
            shell_words::quote(&writer_pid.to_string_lossy()),
            shell_words::quote(&writer_pid.to_string_lossy()),
        ),
    )
}

#[test]
fn private_files_and_attach_fifos_are_0600() {
    let tmp = tempfile::TempDir::new().unwrap();
    let file_path = tmp.path().join("private.file");
    crate::write_private_file(&file_path, b"").unwrap();
    let mode = std::fs::metadata(&file_path).unwrap().permissions().mode();
    assert_eq!(
        mode & 0o077,
        0,
        "private files must not be group/world readable"
    );

    let fifo_path = tmp.path().join("attach.fifo");
    create_private_fifo(&fifo_path).unwrap();
    let metadata = std::fs::metadata(&fifo_path).unwrap();
    assert!(metadata.file_type().is_fifo());
    assert_eq!(
        metadata.permissions().mode() & 0o077,
        0,
        "live terminal FIFO must not be group/world readable"
    );
}

#[tokio::test]
async fn broker_start_removes_fifo_after_post_create_failure() {
    let tmp = tempfile::TempDir::new().unwrap();
    let state_dir = tmp.path().join("state");
    seed_session(&state_dir, "rollback", "codex", "hs-codex-rollback");
    let context = CliContext {
        state_dir,
        host: None,
    };
    let record = load_session_record(&context, "rollback").unwrap();
    let fifo_path = session_dir(&context, "rollback").join(ATTACH_LIVE_FIFO_NAME);

    let result =
        AttachBroker::start_with_fifo_opener(&context, Path::new("unused-tmux"), &record, |_| {
            Err(io::Error::other("injected FIFO open failure"))
        })
        .await;
    assert!(result.is_err());
    assert!(
        !fifo_path.exists(),
        "broker startup failure must roll back its ephemeral FIFO"
    );
}

fn logging_tmux(dir: &Path, log: &Path) -> PathBuf {
    let bin = dir.join("tmux");
    let pane_stub = live_pane_stub_prelude(dir);
    std::fs::write(
            &bin,
            format!(
                "#!/usr/bin/env sh\n{pane_stub}\nprintf '%s\\n' \"$*\" >> {}\ncase \"$1\" in\n  has-session) exit 0 ;;\n  new-session) for arg; do case \"$arg\" in AGENT_SESSION_PRIVATE_PROVIDER_ENV=*) rm -f \"${{arg#*=}}\" ;; esac; done; heartbeat=''; slot=0; for arg in \"$@\"; do if [ \"$slot\" = 2 ]; then incarnation=\"$arg\"; break; elif [ \"$slot\" = 1 ]; then slot=2; else case \"$arg\" in */coordination/heartbeat) heartbeat=\"$arg\"; slot=1 ;; esac; fi; done; if [ -n \"$heartbeat\" ] && [ -n \"$incarnation\" ]; then mkdir -p \"$(dirname \"$heartbeat\")\"; printf '%s:%s\\n' \"$incarnation\" \"$(date +%s)\" > \"$heartbeat\"; chmod 600 \"$heartbeat\"; fi; start_live_pane || exit 1; printf '%s\\t%s\\t%s\\n' '$77' '%77' \"$NILS_TEST_PANE_PID\"; exit 0 ;;\n  display-message) set -- $(cat \"$NILS_TEST_PANE_PARENT/.pane-identity\"); printf '%s\\t%s\\t%s\\n' '$77' '%77' \"$1\"; exit 0 ;;\n  capture-pane) printf 'pane\\n'; exit 0 ;;\n  *) exit 0 ;;\nesac\n",
                shell_words::quote(&log.to_string_lossy())
            ),
        )
        .unwrap();
    let mut perms = std::fs::metadata(&bin).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&bin, perms).unwrap();
    bin
}

fn provision_ready_coordination_fixture(
    context: &CliContext,
    record: &crate::SessionRecord,
) -> String {
    let capability_path =
        crate::coordination::provision(context, record).expect("provision fixture");
    let incarnation = record
        .runtime
        .as_ref()
        .map(|runtime| runtime.launch_id.as_str())
        .expect("fixture incarnation");
    let now = crate::coordination::now_epoch();
    let heartbeat = crate::coordination::heartbeat_path(&context.state_dir, &record.id);
    std::fs::write(&heartbeat, format!("{incarnation}:{now}\n")).expect("heartbeat fixture");
    std::fs::set_permissions(&heartbeat, std::fs::Permissions::from_mode(0o600))
        .expect("heartbeat mode");
    let registry_path = context.state_dir.join("coordination/registry.json");
    let mut registry: Value = serde_json::from_slice(
        &std::fs::read(&registry_path).expect("coordination registry fixture"),
    )
    .expect("coordination registry json");
    registry["brokers"][&record.id]["state"] = json!("ready");
    registry["brokers"][&record.id]["heartbeat_at"] = json!("2030-01-01T00:00:00Z");
    registry["brokers"][&record.id]["heartbeat_epoch"] = json!(now);
    std::fs::write(
        &registry_path,
        serde_json::to_vec_pretty(&registry).expect("registry json"),
    )
    .expect("registry fixture");
    std::fs::set_permissions(&registry_path, std::fs::Permissions::from_mode(0o600))
        .expect("registry mode");
    std::fs::read_to_string(capability_path).expect("capability fixture")
}

/// tmux stub that logs every invocation to `calls` and, on `load-buffer`,
/// appends the pasted buffer file's content to `pasted`. `running` toggles
/// the `has-session` result so title-rename gating can be exercised for both
/// live and stopped sessions.
fn rename_probe_tmux(dir: &Path, calls: &Path, pasted: &Path, running: bool) -> PathBuf {
    let bin = dir.join("tmux");
    let has_session_exit = if running { 0 } else { 1 };
    std::fs::write(
            &bin,
            format!(
                "#!/usr/bin/env sh\nprintf '%s\\n' \"$*\" >> {}\ncase \"$1\" in\n  has-session) result={}; [ \"$result\" = 0 ] || printf '%s\\n' \"can't find session: fixture\" >&2; exit \"$result\" ;;\n  load-buffer) cat \"$4\" >> {}; exit 0 ;;\n  capture-pane) printf 'pane\\n'; exit 0 ;;\n  *) exit 0 ;;\nesac\n",
                shell_words::quote(&calls.to_string_lossy()),
                has_session_exit,
                shell_words::quote(&pasted.to_string_lossy()),
            ),
        )
        .unwrap();
    let mut perms = std::fs::metadata(&bin).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&bin, perms).unwrap();
    bin
}

#[tokio::test]
async fn mail_audit_owner_endpoint_is_body_free_read_only_and_authenticates_operator() {
    let tmp = tempfile::TempDir::new().unwrap();
    let st = state(tmp.path(), Some(TOKEN), minimal_tmux(tmp.path()));
    let fixture: Value = serde_json::from_str(include_str!(
        "../../tests/fixtures/coordination/mail-audit-v1.json"
    ))
    .unwrap();
    let root = tmp.path().join("coordination");
    fs::create_dir_all(&root).unwrap();
    let registry = root.join("registry.json");
    let journal = root.join("federation-journal.json");
    for (path, value) in [
        (&registry, &fixture["registry"]),
        (&journal, &fixture["journal"]),
    ] {
        nils_common::fs::write_atomic(path, &serde_json::to_vec(value).unwrap(), 0o600).unwrap();
    }
    let before = (fs::read(&registry).unwrap(), fs::read(&journal).unwrap());
    for token in [None, Some("session-capability-is-not-operator-authority")] {
        let (status, _) = call(
            router(st.clone()),
            get_auth(crate::coordination::audit::ROUTE, token),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
    let (status, body) = call(
        router(st.clone()),
        get_auth(crate::coordination::audit::ROUTE, Some(TOKEN)),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["data"]["schema_version"],
        "agent-session.mail-audit.v1"
    );
    for canary in [
        "BODY_CANARY",
        "TOKEN_CANARY",
        "HASH_CANARY",
        "PATH_CANARY",
        "REASON_CANARY",
    ] {
        assert!(!body.to_string().contains(canary), "{canary}");
    }
    let local = crate::coordination::audit::snapshot(
        &st.context,
        &st.machine,
        crate::coordination::audit::Query::default(),
    )
    .unwrap();
    assert_eq!(body["data"]["records"], local["records"]);
    assert_eq!(
        before,
        (fs::read(&registry).unwrap(), fs::read(&journal).unwrap())
    );
    for query in [
        "?limit=0",
        "?include_healthy=invalid",
        "?body=true",
        "?cursor=invalid",
    ] {
        let (status, _) = call(
            router(st.clone()),
            get_auth(
                &format!("{}{}", crate::coordination::audit::ROUTE, query),
                Some(TOKEN),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }
}

#[tokio::test]
async fn coordination_mutations_require_session_authority_while_reads_are_operator_public() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    seed_session_with_runtime(tmp.path(), "alpha", "codex", "hs-codex-alpha");
    let state = state(tmp.path(), Some(TOKEN), minimal_tmux(tmp.path()));
    let record = load_session_record(&state.context, "alpha").expect("session");
    let capability = provision_ready_coordination_fixture(&state.context, &record);

    let candidate = json!({
        "schema_version": "agent-session.work-context-input.v1",
        "intent": "implementation",
        "tier": "program",
        "repositories": ["example/repository"],
        "worktrees": [],
        "provider_refs": [],
        "plan_refs": [],
        "scopes": [{
            "kind": "path-prefix",
            "repository": "example/repository",
            "value": "src"
        }],
        "summary": "HTTP parity fixture"
    });
    let (status, body) = call(
        router(state.clone()),
        post_coordination(
            "/sessions/alpha/work-context/claim/v1",
            capability.trim(),
            json!({
                "candidate": candidate,
                "idempotency_key": "http-claim-alpha-0001",
                "if_revision": null
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["data"]["coordination"]["context"]["schema_version"],
        "agent-session.work-context.v1"
    );
    assert!(!body.to_string().contains(capability.trim()));

    let (status, shown) = call(
        router(state.clone()),
        get_coordination("/sessions/alpha/work-context/v1", capability.trim()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{shown}");
    assert_eq!(
        shown["data"]["coordination"]["claim_id"],
        body["data"]["coordination"]["context"]["claim_id"]
    );

    let (status, denied) = call(
        router(state),
        get_coordination(
            "/sessions/alpha/work-context/v1",
            "wrong-capability-material",
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{denied}");
    assert_eq!(
        denied["data"]["coordination"]["claim_id"],
        body["data"]["coordination"]["context"]["claim_id"]
    );
}

#[tokio::test]
async fn broker_recovery_http_route_accepts_stale_exact_authority_and_rejects_wrong_incarnations() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    seed_session_with_runtime(tmp.path(), "alpha", "codex", "hs-codex-alpha");
    seed_session_with_runtime(tmp.path(), "beta", "codex", "hs-codex-beta");
    let state = state(tmp.path(), Some(TOKEN), minimal_tmux(tmp.path()));
    let runtime_process = TestProcessGroup::spawn();
    let mut alpha = load_session_record(&state.context, "alpha").expect("alpha");
    let alpha_incarnation = alpha
        .runtime
        .as_ref()
        .expect("alpha runtime")
        .launch_id
        .clone();
    alpha.extra.insert(
        "delete_tmux_identity".to_string(),
        json!({
            "launch_id": alpha_incarnation,
            "session_id": "$77",
            "pane_id": "%77",
            "pane_pid": runtime_process.pid(),
            "process_group_id": runtime_process.process_group_id
        }),
    );
    crate::write_session_record(&state.context, &alpha).expect("persist live runtime identity");
    let alpha_capability = provision_ready_coordination_fixture(&state.context, &alpha);
    let beta = load_session_record(&state.context, "beta").expect("beta");
    let beta_capability = provision_ready_coordination_fixture(&state.context, &beta);

    let registry_path = tmp.path().join("coordination/registry.json");
    let mut registry: Value = serde_json::from_slice(&fs::read(&registry_path).expect("registry"))
        .expect("registry json");
    registry["brokers"]["alpha"]["heartbeat_epoch"] = json!(0);
    registry["brokers"]["alpha"]["lost_since_epoch"] = json!(1);
    fs::write(
        &registry_path,
        serde_json::to_vec_pretty(&registry).expect("registry json"),
    )
    .expect("stale broker registry");
    fs::set_permissions(&registry_path, fs::Permissions::from_mode(0o600)).expect("registry mode");
    fs::remove_file(crate::coordination::heartbeat_path(
        &state.context.state_dir,
        "alpha",
    ))
    .expect("remove alpha heartbeat");

    let proof = json!({
        "schema_version": "agent-session.coordination-recovery-proof.v1",
        "session_incarnation": alpha_incarnation,
        "generation": 1
    });
    let before_bearer_only =
        fs::read(&registry_path).expect("registry before bearer-only recovery");
    let bearer_only = Request::builder()
        .method("POST")
        .uri("/sessions/alpha/broker/adopt/v1")
        .header(AUTHORIZATION, format!("Bearer {TOKEN}"))
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&json!({
                "proof": proof,
                "idempotency_key": "http-broker-bearer-only-0001",
                "operation": null,
                "if_revision": null,
                "attest_inactive": false
            }))
            .expect("bearer-only request"),
        ))
        .expect("bearer-only request");
    let (status, denied) = call(router(state.clone()), bearer_only).await;
    assert_ne!(status, StatusCode::OK, "{denied}");
    assert_eq!(denied["error"]["code"], "coordination-unauthorized");
    assert_eq!(
        fs::read(&registry_path).expect("registry after bearer-only recovery"),
        before_bearer_only,
        "historical bearer-only recovery must fail before mutation"
    );

    let (status, adopted) = call(
        router(state.clone()),
        post_coordination(
            "/sessions/alpha/broker/adopt/v2",
            alpha_capability.trim(),
            json!({
                "proof": proof,
                "idempotency_key": "http-broker-adopt-alpha-0001",
                "operation": null,
                "if_revision": null,
                "attest_inactive": false
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{adopted}");
    assert_eq!(
        adopted["data"]["coordination"]["recovery"], "adopted",
        "{adopted}"
    );

    let before_cross_session = fs::read(&registry_path).expect("registry before cross-session");
    let (status, denied) = call(
        router(state.clone()),
        post_coordination(
            "/sessions/alpha/broker/adopt/v1",
            beta_capability.trim(),
            json!({
                "proof": proof,
                "idempotency_key": "http-broker-cross-session-0001",
                "operation": null,
                "if_revision": null,
                "attest_inactive": false
            }),
        ),
    )
    .await;
    assert_ne!(status, StatusCode::OK, "{denied}");
    assert_eq!(denied["error"]["code"], "coordination-unauthorized");
    assert_eq!(
        fs::read(&registry_path).expect("registry after cross-session"),
        before_cross_session,
        "cross-session HTTP recovery must not mutate coordination state"
    );

    let mut replaced_beta = load_session_record(&state.context, "beta").expect("beta");
    let replaced_incarnation = "launch-beta-replaced";
    let runtime = replaced_beta.runtime.as_mut().expect("beta runtime");
    runtime.launch_id = replaced_incarnation.to_string();
    runtime.generation = 2;
    crate::write_session_record(&state.context, &replaced_beta)
        .expect("persist replacement beta runtime");
    let replacement_capability = "beta-replacement-private-capability-material";
    let replacement_path =
        crate::coordination::capability_path(&state.context, "beta", replaced_incarnation);
    fs::write(&replacement_path, replacement_capability).expect("replacement capability");
    fs::set_permissions(&replacement_path, fs::Permissions::from_mode(0o600))
        .expect("replacement capability mode");
    let mut registry: Value = serde_json::from_slice(&fs::read(&registry_path).expect("registry"))
        .expect("registry json");
    registry["brokers"]["beta"]["incarnation"] = json!(replaced_incarnation);
    registry["brokers"]["beta"]["generation"] = json!(2);
    registry["brokers"]["beta"]["capability_digest"] = json!(crate::coordination::digest_bytes(
        replacement_capability.as_bytes()
    ));
    fs::write(
        &registry_path,
        serde_json::to_vec_pretty(&registry).expect("registry json"),
    )
    .expect("replacement broker registry");
    fs::set_permissions(&registry_path, fs::Permissions::from_mode(0o600)).expect("registry mode");
    let before_replaced =
        fs::read(&registry_path).expect("registry before prior-incarnation capability");
    let (status, denied) = call(
        router(state),
        post_coordination(
            "/sessions/beta/broker/adopt/v1",
            beta_capability.trim(),
            json!({
                "proof": {
                    "schema_version": "agent-session.coordination-recovery-proof.v1",
                    "session_incarnation": replaced_incarnation,
                    "generation": 2
                },
                "idempotency_key": "http-broker-prior-incarnation-0001",
                "operation": null,
                "if_revision": null,
                "attest_inactive": false
            }),
        ),
    )
    .await;
    assert_ne!(status, StatusCode::OK, "{denied}");
    assert_eq!(denied["error"]["code"], "coordination-unauthorized");
    assert_eq!(
        fs::read(&registry_path).expect("registry after prior-incarnation capability"),
        before_replaced,
        "prior-incarnation HTTP recovery must not mutate coordination state"
    );
}

#[tokio::test]
async fn operator_operation_reconcile_uses_server_authority_and_exact_runtime_fences() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    seed_session_with_runtime(tmp.path(), "alpha", "codex", "hs-codex-alpha");
    let state = state(tmp.path(), Some(TOKEN), minimal_tmux(tmp.path()));
    let mut alpha = load_session_record(&state.context, "alpha").expect("alpha");
    let incarnation = alpha
        .runtime
        .as_ref()
        .expect("alpha runtime")
        .launch_id
        .clone();
    let stopped_runtime = json!({
        "launch_id": incarnation,
        "session_id": "$77",
        "pane_id": "%77",
        "pane_pid": i32::MAX,
        "process_group_id": i32::MAX
    });
    #[cfg(target_os = "linux")]
    let stopped_runtime = {
        let mut stopped_runtime = stopped_runtime;
        let namespace = fs::metadata("/proc/self/ns/pid").expect("PID namespace metadata");
        stopped_runtime["pid_namespace"] = json!({
            "device": namespace.st_dev(),
            "inode": namespace.st_ino(),
            "boot_id": fs::read_to_string("/proc/sys/kernel/random/boot_id")
                .expect("boot id")
                .trim()
        });
        stopped_runtime
    };
    alpha
        .extra
        .insert("delete_tmux_identity".to_string(), stopped_runtime);
    crate::write_session_record(&state.context, &alpha).expect("stopped runtime identity");
    let generation = alpha.runtime.as_ref().expect("alpha runtime").generation;
    let runtime_identity_digest = crate::coordination_runtime_evidence(&state.context, &alpha)
        .expect("runtime evidence")
        .identity_digest;
    let _session_capability = provision_ready_coordination_fixture(&state.context, &alpha);
    let registry_path = tmp.path().join("coordination/registry.json");
    let mut registry: Value = serde_json::from_slice(&fs::read(&registry_path).expect("registry"))
        .expect("registry json");
    registry["operations"]
        .as_array_mut()
        .expect("operations")
        .push(json!({
            "schema_version": "agent-session.operation-lease.v1",
            "lease_id": "orphaned-lease",
            "session_id": "alpha",
            "session_incarnation": incarnation,
            "claim_id": "claim",
            "claim_revision": 1,
            "operation": "shell",
            "targets": [],
            "state": "active",
            "revision": 7,
            "started_at": "2030-01-01T00:00:00Z",
            "expires_at": "2030-01-01T01:00:00Z",
            "expires_at_epoch": 1,
            "execution_token_digest": "PRIVATE-EXECUTION-TOKEN-DIGEST",
            "activity_identity_digest": "same-provider-turn",
            "runtime_identity_digest": runtime_identity_digest
        }));
    fs::write(
        &registry_path,
        serde_json::to_vec_pretty(&registry).expect("registry json"),
    )
    .expect("persist operation");
    fs::set_permissions(&registry_path, fs::Permissions::from_mode(0o600)).expect("registry mode");

    let request = json!({
        "schema_version": "agent-session.operator-operation-reconcile-request.v1",
        "expected_session_incarnation": incarnation,
        "expected_session_generation": generation,
        "if_revision": 7,
        "reason": "post-tool-outcome-missing",
        "attest_inactive": true,
        "confirmed": true,
        "idempotency_key": "operator-reconcile-alpha-0001"
    });
    let before_denied = fs::read(&registry_path).expect("registry before denied request");
    let target_capability_only = Request::builder()
        .method("POST")
        .uri("/sessions/alpha/operations/orphaned-lease/operator-reconcile/v1")
        .header(AUTHORIZATION, "Bearer target-session-capability-material")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&request).expect("denied request"),
        ))
        .expect("denied request");
    let (status, denied) = call(router(state.clone()), target_capability_only).await;
    assert_ne!(status, StatusCode::OK, "{denied}");
    assert_eq!(denied["error"]["code"], "unauthorized");
    assert_eq!(
        fs::read(&registry_path).expect("registry after denied request"),
        before_denied,
        "a target session capability must not become cross-session operator authority"
    );

    let mut stale_incarnation = request.clone();
    stale_incarnation["expected_session_incarnation"] = json!("replacement-incarnation");
    stale_incarnation["idempotency_key"] = json!("operator-reconcile-stale-incarnation");
    let mut stale_generation = request.clone();
    stale_generation["expected_session_generation"] = json!(generation + 1);
    stale_generation["idempotency_key"] = json!("operator-reconcile-stale-generation");
    let mut unconfirmed = request.clone();
    unconfirmed["confirmed"] = json!(false);
    unconfirmed["idempotency_key"] = json!("operator-reconcile-unconfirmed");
    let mut unattested = request.clone();
    unattested["attest_inactive"] = json!(false);
    unattested["idempotency_key"] = json!("operator-reconcile-unattested");
    let mut unsupported_reason = request.clone();
    unsupported_reason["reason"] = json!("guess-operation-finished");
    unsupported_reason["idempotency_key"] = json!("operator-reconcile-unsupported-reason");
    let mut stale_revision = request.clone();
    stale_revision["if_revision"] = json!(6);
    stale_revision["idempotency_key"] = json!("operator-reconcile-stale-revision");
    for (body, expected_code) in [
        (stale_incarnation, "session-incarnation-conflict"),
        (stale_generation, "session-incarnation-conflict"),
        (unconfirmed, "operator-reconcile-confirmation-required"),
        (unattested, "operator-reconcile-confirmation-required"),
        (unsupported_reason, "invalid-operator-reconcile-request"),
        (stale_revision, "operation-revision-conflict"),
    ] {
        let before = fs::read(&registry_path).expect("registry before negative request");
        let negative = Request::builder()
            .method("POST")
            .uri("/sessions/alpha/operations/orphaned-lease/operator-reconcile/v1")
            .header(AUTHORIZATION, format!("Bearer {TOKEN}"))
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_vec(&body).expect("negative request"),
            ))
            .expect("negative request");
        let (status, rejected) = call(router(state.clone()), negative).await;
        assert_ne!(status, StatusCode::OK, "{rejected}");
        assert_eq!(rejected["error"]["code"], expected_code, "{rejected}");
        assert_eq!(
            fs::read(&registry_path).expect("registry after negative request"),
            before,
            "operator fence failures must precede registry mutation"
        );
    }

    let operator_request = Request::builder()
        .method("POST")
        .uri("/sessions/alpha/operations/orphaned-lease/operator-reconcile/v1")
        .header(AUTHORIZATION, format!("Bearer {TOKEN}"))
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&request).expect("operator request"),
        ))
        .expect("operator request");
    #[cfg(not(target_os = "linux"))]
    let before_operator =
        fs::read(&registry_path).expect("registry before operator reconciliation");
    let (status, reconciled) = call(router(state.clone()), operator_request).await;
    assert!(
        !reconciled
            .to_string()
            .contains("PRIVATE-EXECUTION-TOKEN-DIGEST")
    );
    #[cfg(not(target_os = "linux"))]
    {
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{reconciled}");
        assert_eq!(reconciled["error"]["code"], "operation-still-running");
        assert_eq!(
            fs::read(&registry_path).expect("registry after unverified reconciliation"),
            before_operator,
            "unknown non-Linux runtime evidence must not mutate the operation"
        );
        let replay = Request::builder()
            .method("POST")
            .uri("/sessions/alpha/operations/orphaned-lease/operator-reconcile/v1")
            .header(AUTHORIZATION, format!("Bearer {TOKEN}"))
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_vec(&request).expect("operator replay"),
            ))
            .expect("operator replay");
        let (replay_status, replayed) = call(router(state), replay).await;
        assert_eq!(
            replay_status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{replayed}"
        );
        assert_eq!(replayed, reconciled);
    }
    #[cfg(target_os = "linux")]
    {
        assert_eq!(status, StatusCode::OK, "{reconciled}");
        assert_eq!(
            reconciled["data"]["coordination"]["operation_reconciliation"]["state"],
            "abandoned"
        );
        assert_eq!(
            reconciled["data"]["coordination"]["operation_reconciliation"]["revision"],
            8
        );
        let replay = Request::builder()
            .method("POST")
            .uri("/sessions/alpha/operations/orphaned-lease/operator-reconcile/v1")
            .header(AUTHORIZATION, format!("Bearer {TOKEN}"))
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_vec(&request).expect("operator replay"),
            ))
            .expect("operator replay");
        let (replay_status, replayed) = call(router(state), replay).await;
        assert_eq!(replay_status, StatusCode::OK, "{replayed}");
        assert_eq!(replayed, reconciled);
    }
}

#[tokio::test]
async fn operator_provider_turn_reconcile_is_bearer_only_fenced_and_replay_safe() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    seed_session_with_runtime(tmp.path(), "alpha", "codex", "hs-codex-alpha");
    seed_session_with_runtime(tmp.path(), "beta", "codex", "hs-codex-beta");
    let state = state(tmp.path(), Some(TOKEN), minimal_tmux(tmp.path()));
    let runtime_process = TestProcessGroup::spawn();
    let mut alpha = load_session_record(&state.context, "alpha").expect("alpha");
    let incarnation = alpha
        .runtime
        .as_ref()
        .expect("alpha runtime")
        .launch_id
        .clone();
    alpha.extra.insert(
        "delete_tmux_identity".to_string(),
        json!({
            "launch_id": incarnation,
            "session_id": "$77",
            "pane_id": "%77",
            "pane_pid": runtime_process.pid(),
            "process_group_id": runtime_process.process_group_id
        }),
    );
    crate::write_session_record(&state.context, &alpha).expect("live runtime identity");
    let generation = alpha.runtime.as_ref().expect("alpha runtime").generation;
    crate::activity::activate_runtime(&state.context, &alpha).expect("activate activity");
    for (event_id, kind) in [
        ("provider-turn-start", "turn_started"),
        ("provider-turn-stop", "stop_observed"),
    ] {
        let event = serde_json::from_value(json!({
            "schema_version": crate::activity::TURN_EVENT_VERSION,
            "event_id": event_id,
            "runtime_id": incarnation,
            "provider": "codex",
            "provider_turn_id": "provider-turn-alpha",
            "kind": kind,
            "confidence": "authoritative"
        }))
        .expect("activity event");
        crate::activity::ingest_event(&state.context, &alpha.id, event).expect("ingest");
    }
    let stopped = crate::activity::activity_status(&state.context, &alpha.id)
        .expect("activity")
        .turn_state;
    let provider_turn_id = stopped
        .current_turn
        .as_ref()
        .and_then(|turn| turn.provider_turn_id.as_deref())
        .expect("exact provider turn")
        .to_string();
    let activity_revision = stopped.revision;
    let target_capability = provision_ready_coordination_fixture(&state.context, &alpha);
    let beta = load_session_record(&state.context, "beta").expect("beta");
    let beta_capability = provision_ready_coordination_fixture(&state.context, &beta);
    let registry_path = tmp.path().join("coordination/registry.json");
    let (status, claimed) = call(
        router(state.clone()),
        post_coordination(
            "/sessions/beta/work-context/claim/v1",
            beta_capability.trim(),
            json!({
                "candidate": {
                    "schema_version": "agent-session.work-context-input.v1",
                    "intent": "implementation",
                    "tier": "program",
                    "repositories": ["example/repository"],
                    "worktrees": [],
                    "provider_refs": [],
                    "plan_refs": [],
                    "scopes": [{
                        "kind": "path-exact",
                        "repository": "example/repository",
                        "value": "src/provider-turn.rs"
                    }],
                    "summary": "preserve provider-turn reconciliation claim"
                },
                "idempotency_key": "provider-turn-preserved-claim",
                "if_revision": null
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{claimed}");
    let (status, canary_message) = call(
        router(state.clone()),
        post_coordination(
            "/sessions/beta/messages/v1",
            beta_capability.trim(),
            json!({
                "body": "provider-turn-unrelated-mailbox-canary",
                "idempotency_key": "provider-turn-unrelated-message",
                "reply_to": null,
                "expires_in": null
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{canary_message}");
    let canary_registry: Value =
        serde_json::from_slice(&fs::read(&registry_path).expect("canary registry"))
            .expect("canary registry json");
    assert!(
        !canary_registry["messages"]
            .as_array()
            .expect("messages")
            .is_empty()
    );
    assert!(
        !canary_registry["receipts"]
            .as_object()
            .expect("receipts")
            .is_empty()
    );
    assert!(
        !canary_registry["notifications"]
            .as_object()
            .expect("notifications")
            .is_empty()
    );
    let request = json!({
        "schema_version": "agent-session.operator-provider-turn-reconcile-request.v1",
        "expected_session_incarnation": incarnation,
        "expected_runtime_launch_id": incarnation,
        "expected_runtime_generation": generation,
        "if_activity_revision": activity_revision,
        "expected_provider_turn_id": provider_turn_id,
        "reason": "authoritative-completion-signal-missing",
        "attest_inactive": true,
        "confirmed": true,
        "idempotency_key": "operator/provider-turn@alpha#0001"
    });

    let activity_path = tmp.path().join("sessions/alpha/activity.json");
    let session_path = tmp.path().join("sessions/alpha/session.json");
    let activity_baseline = fs::read(&activity_path).expect("activity baseline");
    let mut pending_activity: Value =
        serde_json::from_slice(&activity_baseline).expect("activity json");
    pending_activity["pending_journal"] = json!({
        "received_at": "2030-01-01T00:00:00Z",
        "event": {
            "schema_version": crate::activity::TURN_EVENT_VERSION,
            "event_id": "queued-provider-stop",
            "runtime_id": incarnation,
            "provider": "codex",
            "provider_turn_id": "provider-turn-alpha",
            "kind": "stop_observed",
            "confidence": "authoritative"
        }
    });
    let pending_bytes =
        serde_json::to_vec_pretty(&pending_activity).expect("pending activity json");
    fs::write(&activity_path, &pending_bytes).expect("pending activity");
    let mut pending_request = request.clone();
    pending_request["idempotency_key"] = json!("operator-provider-turn-pending-journal");
    let pending = post_operator_provider_turn(&pending_request);
    let (status, rejected) = call(router(state.clone()), pending).await;
    assert_ne!(status, StatusCode::OK, "{rejected}");
    assert_eq!(
        rejected["error"]["code"],
        "operator-provider-turn-reconcile-not-admissible"
    );
    assert_eq!(
        fs::read(&activity_path).expect("activity after pending rejection"),
        pending_bytes
    );

    pending_activity["runtime_id"] = json!("prior-runtime");
    let stale_runtime_bytes =
        serde_json::to_vec_pretty(&pending_activity).expect("stale runtime activity json");
    fs::write(&activity_path, &stale_runtime_bytes).expect("stale runtime activity");
    let mut stale_runtime_request = request.clone();
    stale_runtime_request["idempotency_key"] = json!("operator-provider-turn-stale-runtime");
    let stale_runtime = post_operator_provider_turn(&stale_runtime_request);
    let (status, rejected) = call(router(state.clone()), stale_runtime).await;
    assert_ne!(status, StatusCode::OK, "{rejected}");
    assert_eq!(rejected["error"]["code"], "session-incarnation-conflict");
    assert_eq!(
        fs::read(&activity_path).expect("activity after stale runtime rejection"),
        stale_runtime_bytes
    );
    fs::write(&activity_path, &activity_baseline).expect("restore activity baseline");

    let before_denied = fs::read(&activity_path).expect("activity before denied request");
    let target_capability_only = Request::builder()
        .method("POST")
        .uri("/sessions/alpha/activity/provider-turn/operator-reconcile/v1")
        .header(
            AUTHORIZATION,
            format!("Bearer {}", target_capability.trim()),
        )
        .header("x-agent-session-capability", target_capability.trim())
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&request).expect("denied request"),
        ))
        .expect("denied request");
    let (status, denied) = call(router(state.clone()), target_capability_only).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{denied}");
    assert_eq!(denied["error"]["code"], "unauthorized");
    assert_eq!(
        fs::read(&activity_path).expect("activity after denied request"),
        before_denied,
        "target session capability must not become server operator authority"
    );

    let mut invalid_key = request.clone();
    invalid_key["idempotency_key"] = json!("short");
    let mut unconfirmed = request.clone();
    unconfirmed["confirmed"] = json!(false);
    unconfirmed["idempotency_key"] = json!("operator-provider-turn-unconfirmed");
    let mut unattested = request.clone();
    unattested["attest_inactive"] = json!(false);
    unattested["idempotency_key"] = json!("operator-provider-turn-unattested");
    let mut stale_incarnation = request.clone();
    stale_incarnation["expected_session_incarnation"] = json!("replacement-incarnation");
    stale_incarnation["idempotency_key"] = json!("operator-provider-turn-incarnation");
    let mut stale_launch = request.clone();
    stale_launch["expected_runtime_launch_id"] = json!("replacement-launch");
    stale_launch["idempotency_key"] = json!("operator-provider-turn-launch");
    let mut stale_generation = request.clone();
    stale_generation["expected_runtime_generation"] = json!(generation + 1);
    stale_generation["idempotency_key"] = json!("operator-provider-turn-generation");
    let mut stale_turn = request.clone();
    stale_turn["expected_provider_turn_id"] = json!("replacement-provider-turn");
    stale_turn["idempotency_key"] = json!("operator-provider-turn-selector");
    for (body, expected_code) in [
        (invalid_key, "invalid-idempotency-key"),
        (
            unconfirmed,
            "operator-provider-turn-reconcile-confirmation-required",
        ),
        (
            unattested,
            "operator-provider-turn-reconcile-confirmation-required",
        ),
        (stale_incarnation, "session-incarnation-conflict"),
        (stale_launch, "session-incarnation-conflict"),
        (stale_generation, "session-incarnation-conflict"),
        (stale_turn, "provider-turn-id-mismatch"),
    ] {
        let negative = post_operator_provider_turn(&body);
        let (status, rejected) = call(router(state.clone()), negative).await;
        assert_ne!(status, StatusCode::OK, "{rejected}");
        assert_eq!(rejected["error"]["code"], expected_code, "{rejected}");
        assert!(
            !rejected.to_string().contains("replacement-provider-turn"),
            "selector values must remain redacted: {rejected}"
        );
    }

    let mut stale_revision = request.clone();
    stale_revision["if_activity_revision"] = json!(activity_revision.saturating_sub(1));
    stale_revision["idempotency_key"] = json!("operator-provider-turn-stale-revision");
    let negative = post_operator_provider_turn(&stale_revision);
    let (status, rejected) = call(router(state.clone()), negative).await;
    assert_eq!(status, StatusCode::CONFLICT, "{rejected}");
    assert_eq!(rejected["error"]["code"], "activity-revision-conflict");

    let runtime_identity_digest = crate::coordination_runtime_evidence(&state.context, &alpha)
        .expect("runtime evidence")
        .identity_digest;
    let mut registry: Value = serde_json::from_slice(&fs::read(&registry_path).expect("registry"))
        .expect("registry json");
    registry["operations"]
            .as_array_mut()
            .expect("operations")
            .push(json!({
                "schema_version": "agent-session.operation-lease.v1",
                "lease_id": "provider-turn-active-operation",
                "session_id": "alpha",
                "session_incarnation": incarnation,
                "claim_id": "preserved-claim",
                "claim_revision": 1,
                "operation": "test mutation",
                "targets": [],
                "provider_targets": [],
                "state": "active",
                "revision": 1,
                "started_at": "2030-01-01T00:00:00Z",
                "expires_at": "9999-12-31T23:59:59Z",
                "expires_at_epoch": i64::MAX,
                "terminal_at_epoch": null,
                "execution_token_digest": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "activity_revision": activity_revision,
                "activity_identity_digest": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                "runtime_identity_digest": runtime_identity_digest,
                "descendant": null,
                "reconcile_observed_at_epoch": null,
                "outcome": null
            }));
    fs::write(
        &registry_path,
        serde_json::to_vec_pretty(&registry).expect("registry json"),
    )
    .expect("persist active operation");
    fs::set_permissions(&registry_path, fs::Permissions::from_mode(0o600)).expect("registry mode");
    let operation_conflict = post_operator_provider_turn(&request);
    let (status, rejected) = call(router(state.clone()), operation_conflict).await;
    assert_ne!(status, StatusCode::OK, "{rejected}");
    assert_eq!(
        rejected["error"]["code"],
        "operator-provider-turn-reconcile-operation-conflict"
    );
    let mut registry: Value = serde_json::from_slice(&fs::read(&registry_path).expect("registry"))
        .expect("registry json");
    registry["operations"][0]["state"] = json!("completing");
    fs::write(
        &registry_path,
        serde_json::to_vec_pretty(&registry).expect("registry json"),
    )
    .expect("persist uncertain operation");
    let uncertain_operation = post_operator_provider_turn(&request);
    let (status, rejected) = call(router(state.clone()), uncertain_operation).await;
    assert_ne!(status, StatusCode::OK, "{rejected}");
    assert_eq!(
        rejected["error"]["code"],
        "operator-provider-turn-reconcile-operation-conflict"
    );
    registry["operations"][0]["state"] = json!("future_nonterminal_state");
    let unknown_registry =
        serde_json::to_vec_pretty(&registry).expect("unknown operation registry");
    fs::write(&registry_path, &unknown_registry).expect("persist unknown operation");
    let unknown_activity = fs::read(&activity_path).expect("activity before unknown operation");
    let unknown_session = fs::read(&session_path).expect("session before unknown operation");
    let unknown_operation = post_operator_provider_turn(&request);
    let (status, rejected) = call(router(state.clone()), unknown_operation).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{rejected}");
    assert_eq!(
        rejected["error"]["code"],
        "operator-provider-turn-reconcile-operation-conflict"
    );
    assert_eq!(
        fs::read(&activity_path).expect("activity after unknown operation"),
        unknown_activity
    );
    assert_eq!(
        fs::read(&session_path).expect("session after unknown operation"),
        unknown_session
    );
    assert_eq!(
        fs::read(&registry_path).expect("registry after unknown operation"),
        unknown_registry
    );
    registry["operations"] = json!([{
        "schema_version": "agent-session.operation-lease.v1",
        "lease_id": "provider-turn-unrelated-operation",
        "session_id": "unrelated-session",
        "session_incarnation": "unrelated-incarnation",
        "claim_id": "unrelated-claim",
        "claim_revision": 1,
        "operation": "unrelated mutation",
        "targets": [],
        "provider_targets": [],
        "state": "active",
        "revision": 1,
        "started_at": "2030-01-01T00:00:00Z",
        "expires_at": "9999-12-31T23:59:59Z",
        "expires_at_epoch": i64::MAX,
        "terminal_at_epoch": null,
        "execution_token_digest": "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
        "activity_revision": 0,
        "activity_identity_digest": "",
        "runtime_identity_digest": runtime_identity_digest,
        "descendant": null,
        "reconcile_observed_at_epoch": null,
        "outcome": null
    }]);
    fs::write(
        &registry_path,
        serde_json::to_vec_pretty(&registry).expect("registry json"),
    )
    .expect("persist unrelated operation canary");

    let ready_registry = fs::read(&registry_path).expect("ready registry");
    let ready_registry_json: Value =
        serde_json::from_slice(&ready_registry).expect("ready registry json");
    assert!(
        !ready_registry_json["operations"]
            .as_array()
            .expect("operations")
            .is_empty()
    );
    for (case, mutate) in [
        ("missing-broker", "missing"),
        ("wrong-incarnation", "incarnation"),
        ("wrong-generation", "generation"),
        ("non-ready", "state"),
        ("stale-heartbeat", "heartbeat"),
        ("runtime-digest-mismatch", "digest"),
    ] {
        let mut rejected_registry: Value =
            serde_json::from_slice(&ready_registry).expect("ready registry json");
        match mutate {
            "missing" => {
                rejected_registry["brokers"]
                    .as_object_mut()
                    .expect("brokers")
                    .remove("alpha");
            }
            "incarnation" => {
                rejected_registry["brokers"]["alpha"]["incarnation"] =
                    json!("replacement-incarnation");
            }
            "generation" => {
                rejected_registry["brokers"]["alpha"]["generation"] = json!(generation + 1);
            }
            "state" => {
                rejected_registry["brokers"]["alpha"]["state"] = json!("stopped");
            }
            "heartbeat" => {
                rejected_registry["brokers"]["alpha"]["heartbeat_epoch"] = json!(0);
            }
            "digest" => {
                rejected_registry["brokers"]["alpha"]["runtime_identity_digest"] =
                    json!("cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc");
            }
            _ => unreachable!("admission case"),
        }
        let rejected_registry: crate::coordination::Registry =
            serde_json::from_value(rejected_registry)
                .unwrap_or_else(|error| panic!("case={case} invalid registry: {error}"));
        let rejected_registry = serde_json::to_vec_pretty(&rejected_registry)
            .unwrap_or_else(|error| panic!("case={case} registry encoding failed: {error}"));
        fs::write(&registry_path, &rejected_registry).expect("persist rejected registry");
        let heartbeat_restore = (case == "stale-heartbeat").then(|| {
            let path = crate::coordination::heartbeat_path(&state.context.state_dir, &alpha.id);
            let bytes = fs::read(&path).expect("alpha heartbeat bytes");
            let permissions = fs::metadata(&path)
                .expect("alpha heartbeat metadata")
                .permissions();
            fs::remove_file(&path).expect("remove alpha heartbeat");
            (path, bytes, permissions)
        });
        let activity_before = fs::read(&activity_path).expect("activity before broker rejection");
        let session_before = fs::read(&session_path).expect("session before broker rejection");
        let request = post_operator_provider_turn(&request);
        let (status, rejected) = call(router(state.clone()), request).await;
        assert_eq!(
            status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "case={case} {rejected}"
        );
        assert_eq!(
            rejected["error"]["code"], "operator-provider-turn-reconcile-operation-conflict",
            "case={case} {rejected}"
        );
        assert_eq!(
            fs::read(&activity_path).expect("activity after broker rejection"),
            activity_before,
            "case={case}"
        );
        assert_eq!(
            fs::read(&session_path).expect("session after broker rejection"),
            session_before,
            "case={case}"
        );
        assert_eq!(
            fs::read(&registry_path).expect("registry after broker rejection"),
            rejected_registry,
            "case={case}"
        );
        if let Some((path, bytes, permissions)) = heartbeat_restore {
            fs::write(&path, bytes).expect("restore alpha heartbeat");
            fs::set_permissions(&path, permissions).expect("restore alpha heartbeat mode");
        }
    }
    fs::write(&registry_path, &ready_registry).expect("restore ready registry");

    let capability_path =
        crate::coordination::capability_path(&state.context, &alpha.id, &incarnation);
    let capability_bytes = fs::read(&capability_path).expect("capability");
    fs::remove_file(&capability_path).expect("remove capability");
    let activity_before = fs::read(&activity_path).expect("activity before missing capability");
    let session_before = fs::read(&session_path).expect("session before missing capability");
    let missing_capability = post_operator_provider_turn(&request);
    let (status, rejected) = call(router(state.clone()), missing_capability).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{rejected}");
    assert_eq!(
        rejected["error"]["code"],
        "operator-provider-turn-reconcile-operation-conflict"
    );
    assert_eq!(
        fs::read(&activity_path).expect("activity after missing capability"),
        activity_before
    );
    assert_eq!(
        fs::read(&session_path).expect("session after missing capability"),
        session_before
    );
    assert_eq!(
        fs::read(&registry_path).expect("registry after missing capability"),
        ready_registry
    );
    fs::write(&capability_path, capability_bytes).expect("restore capability");
    fs::set_permissions(&capability_path, fs::Permissions::from_mode(0o600))
        .expect("capability mode");

    let unhealthy_path = tmp.path().join("sessions/alpha/activity.unhealthy.json");
    fs::write(
        &unhealthy_path,
        serde_json::to_vec_pretty(&json!({
            "schema_version": "agent-session.activity-unhealthy.v1",
            "runtime_id": incarnation,
            "runtime_generation": generation,
            "reason": "provider-turn-test-unhealthy",
            "marked_at": "2030-01-01T00:00:00Z"
        }))
        .expect("unhealthy marker"),
    )
    .expect("persist unhealthy marker");
    fs::set_permissions(&unhealthy_path, fs::Permissions::from_mode(0o600))
        .expect("unhealthy marker mode");
    let activity_before = fs::read(&activity_path).expect("activity before unhealthy");
    let session_before = fs::read(&session_path).expect("session before unhealthy");
    let unhealthy = post_operator_provider_turn(&request);
    let (status, rejected) = call(router(state.clone()), unhealthy).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{rejected}");
    assert_eq!(rejected["error"]["code"], "activity-runtime-unhealthy");
    assert_eq!(
        fs::read(&activity_path).expect("activity after unhealthy"),
        activity_before
    );
    assert_eq!(
        fs::read(&session_path).expect("session after unhealthy"),
        session_before
    );
    assert_eq!(
        fs::read(&registry_path).expect("registry after unhealthy"),
        ready_registry
    );
    fs::remove_file(&unhealthy_path).expect("remove unhealthy marker");

    let live_session = fs::read(&session_path).expect("live session");
    let mut stopped_alpha = alpha.clone();
    stopped_alpha.extra.insert(
        "delete_tmux_identity".to_string(),
        json!({
            "launch_id": incarnation,
            "session_id": "$77",
            "pane_id": "%77",
            "pane_pid": i32::MAX,
            "process_group_id": i32::MAX
        }),
    );
    crate::write_session_record(&state.context, &stopped_alpha).expect("stopped runtime");
    let activity_before = fs::read(&activity_path).expect("activity before stopped runtime");
    let session_before = fs::read(&session_path).expect("stopped session");
    let stopped_runtime = post_operator_provider_turn(&request);
    let (status, rejected) = call(router(state.clone()), stopped_runtime).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{rejected}");
    assert_eq!(
        rejected["error"]["code"],
        "operator-provider-turn-reconcile-runtime-conflict"
    );
    assert_eq!(
        fs::read(&activity_path).expect("activity after stopped runtime"),
        activity_before
    );
    assert_eq!(
        fs::read(&session_path).expect("session after stopped runtime"),
        session_before
    );
    assert_eq!(
        fs::read(&registry_path).expect("registry after stopped runtime"),
        ready_registry
    );
    fs::write(&session_path, live_session).expect("restore live session");
    fs::set_permissions(&session_path, fs::Permissions::from_mode(0o600)).expect("session mode");

    let session_before = fs::read(&session_path).expect("session before reconcile");
    let registry_before = fs::read(&registry_path).expect("registry before reconcile");
    let reconcile = post_operator_provider_turn(&request);
    let (status, reconciled) = call(router(state.clone()), reconcile).await;
    assert_eq!(status, StatusCode::OK, "{reconciled}");
    let result = &reconciled["data"]["coordination"];
    assert_eq!(
        result["schema_version"],
        "agent-session.operator-provider-turn-reconcile-result.v1"
    );
    assert_eq!(
        result["provider_turn_reconciliation"]["state"],
        "operator_reconciled"
    );
    assert_eq!(
        result["provider_turn_reconciliation"]["provenance"],
        "server_operator"
    );
    let activity = crate::activity::activity_status(&state.context, &alpha.id)
        .expect("reconciled activity")
        .turn_state;
    assert_eq!(activity.phase, crate::activity::TurnPhase::Waiting);
    assert!(activity.current_turn.is_none());
    assert_eq!(
        activity
            .last_turn
            .as_ref()
            .map(|turn| turn.outcome.as_str()),
        Some("operator_reconciled")
    );
    let activity_json = serde_json::to_value(&activity).expect("activity json");
    assert!(activity_json.get("operator_reconciliation").is_none());
    assert_eq!(
        activity_json["last_turn"]["operator_reconciliation"]["provider_turn_id"],
        provider_turn_id
    );
    assert_eq!(
        fs::read(&session_path).expect("session after reconcile"),
        session_before,
        "reconciliation must preserve the session/runtime record"
    );
    assert_eq!(
        fs::read(&registry_path).expect("registry after reconcile"),
        registry_before,
        "reconciliation must preserve claim, broker, operation, and mailbox state"
    );
    let rendered = reconciled.to_string();
    assert!(!rendered.contains("operator/provider-turn@alpha#0001"));
    assert!(!rendered.contains("hs-codex-alpha"));
    assert!(!rendered.contains("request_digest"));

    let activity_before_replay = fs::read(&activity_path).expect("activity before exact replay");
    let session_before_replay = fs::read(&session_path).expect("session before exact replay");
    let registry_before_replay = fs::read(&registry_path).expect("registry before exact replay");
    let replay = post_operator_provider_turn(&request);
    let (status, replayed) = call(router(state.clone()), replay).await;
    assert_eq!(status, StatusCode::OK, "{replayed}");
    assert_eq!(replayed, reconciled);
    assert_eq!(
        fs::read(&activity_path).expect("activity after exact replay"),
        activity_before_replay
    );
    assert_eq!(
        fs::read(&session_path).expect("session after exact replay"),
        session_before_replay
    );
    assert_eq!(
        fs::read(&registry_path).expect("registry after exact replay"),
        registry_before_replay
    );

    let mut pending_after_success: Value =
        serde_json::from_slice(&activity_before_replay).expect("reconciled activity json");
    pending_after_success["pending_journal"] = json!({
        "received_at": "2030-01-01T00:00:00Z",
        "event": {
            "schema_version": crate::activity::TURN_EVENT_VERSION,
            "event_id": "queued-after-operator-reconciliation",
            "runtime_id": incarnation,
            "provider": "codex",
            "provider_turn_id": "provider-turn-alpha",
            "kind": "stop_observed",
            "confidence": "authoritative"
        }
    });
    let pending_after_success =
        serde_json::to_vec_pretty(&pending_after_success).expect("pending activity");
    fs::write(&activity_path, &pending_after_success).expect("persist pending activity");
    let pending_replay = post_operator_provider_turn(&request);
    let (status, replayed) = call(router(state.clone()), pending_replay).await;
    assert_eq!(status, StatusCode::OK, "{replayed}");
    assert_eq!(replayed, reconciled);

    let mut changed_request = request.clone();
    changed_request["if_activity_revision"] = json!(activity_revision + 1);
    let changed = post_operator_provider_turn(&changed_request);
    let (status, rejected) = call(router(state.clone()), changed).await;
    assert_ne!(status, StatusCode::OK, "{rejected}");
    assert_eq!(rejected["error"]["code"], "idempotency-key-reused");

    let mut changed_key = request.clone();
    changed_key["idempotency_key"] = json!("operator/provider-turn@alpha#0002");
    changed_key["if_activity_revision"] = json!(activity_revision + 1);
    let changed = post_operator_provider_turn(&changed_key);
    let (status, rejected) = call(router(state.clone()), changed).await;
    assert_ne!(status, StatusCode::OK, "{rejected}");
    assert_eq!(
        rejected["error"]["code"],
        "operator-provider-turn-reconcile-not-admissible"
    );
    assert_eq!(
        fs::read(&activity_path).expect("activity after queued replay cases"),
        pending_after_success
    );
    assert_eq!(
        fs::read(&session_path).expect("session after queued replay cases"),
        session_before_replay
    );
    assert_eq!(
        fs::read(&registry_path).expect("registry after queued replay cases"),
        registry_before_replay
    );
    fs::write(&activity_path, activity_before_replay)
        .expect("restore reconciled activity after queued replay cases");

    for (event_id, kind) in [
        ("provider-turn-start-second", "turn_started"),
        ("provider-turn-stop-second", "stop_observed"),
    ] {
        let event = serde_json::from_value(json!({
            "schema_version": crate::activity::TURN_EVENT_VERSION,
            "event_id": event_id,
            "runtime_id": incarnation,
            "provider": "codex",
            "provider_turn_id": "provider-turn-alpha-second",
            "kind": kind,
            "confidence": "authoritative"
        }))
        .expect("second activity event");
        crate::activity::ingest_event(&state.context, &alpha.id, event)
            .expect("ingest second turn");
    }
    let second_stopped = crate::activity::activity_status(&state.context, &alpha.id)
        .expect("second stopped activity")
        .turn_state;
    let mut second_request = request.clone();
    second_request["if_activity_revision"] = json!(second_stopped.revision);
    second_request["expected_provider_turn_id"] = json!(
        second_stopped
            .current_turn
            .as_ref()
            .and_then(|turn| turn.provider_turn_id.as_deref())
            .expect("second projected turn")
    );
    second_request["idempotency_key"] = json!("operator/provider-turn@alpha#second");
    let second = post_operator_provider_turn(&second_request);
    let (status, second_reconciled) = call(router(state.clone()), second).await;
    assert_eq!(status, StatusCode::OK, "{second_reconciled}");

    let first_replay_after_second = post_operator_provider_turn(&request);
    let (status, replayed) = call(router(state), first_replay_after_second).await;
    assert_eq!(status, StatusCode::OK, "{replayed}");
    assert_eq!(
        replayed, reconciled,
        "a later reconciliation in the same runtime must not evict the first 24-hour receipt"
    );
}

async fn assert_provider_event_wins_operator_turn_reconciliation(
    later_kind: &str,
    later_provider_turn_id: Option<&str>,
    expected_phase: crate::activity::TurnPhase,
    expected_outcome: Option<&str>,
    expected_error_code: &str,
) {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    seed_session_with_runtime(tmp.path(), "alpha", "codex", "hs-codex-alpha");
    let state = state(tmp.path(), Some(TOKEN), minimal_tmux(tmp.path()));
    let runtime_process = TestProcessGroup::spawn();
    let mut alpha = load_session_record(&state.context, "alpha").expect("alpha");
    let incarnation = alpha
        .runtime
        .as_ref()
        .expect("alpha runtime")
        .launch_id
        .clone();
    alpha.extra.insert(
        "delete_tmux_identity".to_string(),
        json!({
            "launch_id": incarnation,
            "session_id": "$77",
            "pane_id": "%77",
            "pane_pid": runtime_process.pid(),
            "process_group_id": runtime_process.process_group_id
        }),
    );
    crate::write_session_record(&state.context, &alpha).expect("live runtime identity");
    let generation = alpha.runtime.as_ref().expect("alpha runtime").generation;
    crate::activity::activate_runtime(&state.context, &alpha).expect("activate activity");
    for (event_id, kind) in [
        ("operator-race-start", "turn_started"),
        ("operator-race-stop", "stop_observed"),
    ] {
        let event = serde_json::from_value(json!({
            "schema_version": crate::activity::TURN_EVENT_VERSION,
            "event_id": event_id,
            "runtime_id": incarnation,
            "provider": "codex",
            "provider_turn_id": "provider-turn-alpha",
            "kind": kind,
            "confidence": "authoritative"
        }))
        .expect("activity event");
        crate::activity::ingest_event(&state.context, &alpha.id, event).expect("ingest");
    }
    let stopped = crate::activity::activity_status(&state.context, &alpha.id)
        .expect("stopped activity")
        .turn_state;
    let provider_turn_id = stopped
        .current_turn
        .as_ref()
        .and_then(|turn| turn.provider_turn_id.as_deref())
        .expect("projected provider turn")
        .to_string();
    provision_ready_coordination_fixture(&state.context, &alpha);

    let mut later = json!({
        "schema_version": crate::activity::TURN_EVENT_VERSION,
        "event_id": format!("operator-race-{later_kind}"),
        "runtime_id": incarnation,
        "provider": "codex",
        "kind": later_kind,
        "confidence": "authoritative"
    });
    if let Some(provider_turn_id) = later_provider_turn_id {
        later["provider_turn_id"] = json!(provider_turn_id);
    }
    if later_kind == "attention_requested" {
        later["attention_id"] = json!("operator-race-attention");
        later["attention_kind"] = json!("clarification");
    }
    let later = serde_json::from_value(later).expect("later provider event");
    crate::activity::ingest_event(&state.context, &alpha.id, later)
        .expect("ingest later provider event");
    let latest = crate::activity::activity_status(&state.context, &alpha.id)
        .expect("latest activity")
        .turn_state;
    let activity_path = tmp.path().join("sessions/alpha/activity.json");
    let activity_before = fs::read(&activity_path).expect("activity before rejection");

    let request = json!({
        "schema_version": "agent-session.operator-provider-turn-reconcile-request.v1",
        "expected_session_incarnation": incarnation,
        "expected_runtime_launch_id": incarnation,
        "expected_runtime_generation": generation,
        "if_activity_revision": latest.revision,
        "expected_provider_turn_id": provider_turn_id,
        "reason": "authoritative-completion-signal-missing",
        "attest_inactive": true,
        "confirmed": true,
        "idempotency_key": format!("operator-provider-race-{later_kind}")
    });
    let (status, rejected) = call(router(state), post_operator_provider_turn(&request)).await;
    assert_ne!(status, StatusCode::OK, "{rejected}");
    assert_eq!(rejected["error"]["code"], expected_error_code, "{rejected}");
    assert_eq!(latest.phase, expected_phase);
    assert_eq!(
        latest.last_turn.as_ref().map(|turn| turn.outcome.as_str()),
        expected_outcome
    );
    assert_eq!(
        fs::read(&activity_path).expect("activity after rejection"),
        activity_before,
        "a newer provider event must remain byte-for-byte authoritative"
    );
    assert!(!rejected.to_string().contains("operator-race-attention"));
}

#[tokio::test]
async fn provider_attention_newer_event_and_completion_win_before_turn_reconciliation() {
    assert_provider_event_wins_operator_turn_reconciliation(
        "attention_requested",
        Some("provider-turn-alpha"),
        crate::activity::TurnPhase::NeedsInput,
        None,
        "operator-provider-turn-reconcile-not-admissible",
    )
    .await;
    assert_provider_event_wins_operator_turn_reconciliation(
        "progress",
        Some("provider-turn-alpha"),
        crate::activity::TurnPhase::Working,
        None,
        "operator-provider-turn-reconcile-not-admissible",
    )
    .await;
    assert_provider_event_wins_operator_turn_reconciliation(
        "turn_completed",
        Some("provider-turn-alpha"),
        crate::activity::TurnPhase::Waiting,
        Some("completed"),
        "operator-provider-turn-reconcile-not-admissible",
    )
    .await;
    assert_provider_event_wins_operator_turn_reconciliation(
        "turn_failed",
        Some("provider-turn-alpha"),
        crate::activity::TurnPhase::Waiting,
        Some("failed"),
        "operator-provider-turn-reconcile-not-admissible",
    )
    .await;
    assert_provider_event_wins_operator_turn_reconciliation(
        "turn_started",
        Some("newer-provider-turn"),
        crate::activity::TurnPhase::Working,
        Some("interrupted"),
        "provider-turn-id-mismatch",
    )
    .await;
    assert_provider_event_wins_operator_turn_reconciliation(
        "stop_observed",
        Some("different-provider-turn"),
        crate::activity::TurnPhase::Working,
        None,
        "operator-provider-turn-reconcile-not-admissible",
    )
    .await;
    assert_provider_event_wins_operator_turn_reconciliation(
        "stop_observed",
        None,
        crate::activity::TurnPhase::Working,
        None,
        "operator-provider-turn-reconcile-not-admissible",
    )
    .await;
}

#[tokio::test]
async fn durable_provider_completion_wins_before_operator_reconciliation() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    seed_session_with_runtime(tmp.path(), "alpha", "codex", "hs-codex-alpha");
    let state = state(tmp.path(), Some(TOKEN), minimal_tmux(tmp.path()));
    let mut alpha = load_session_record(&state.context, "alpha").expect("alpha");
    let incarnation = alpha
        .runtime
        .as_ref()
        .expect("alpha runtime")
        .launch_id
        .clone();
    alpha.extra.insert(
        "delete_tmux_identity".to_string(),
        json!({
            "launch_id": incarnation,
            "session_id": "$77",
            "pane_id": "%77",
            "pane_pid": i32::MAX,
            "process_group_id": i32::MAX
        }),
    );
    crate::write_session_record(&state.context, &alpha).expect("stopped runtime identity");
    let generation = alpha.runtime.as_ref().expect("alpha runtime").generation;
    let runtime_identity_digest = crate::coordination_runtime_evidence(&state.context, &alpha)
        .expect("runtime evidence")
        .identity_digest;
    provision_ready_coordination_fixture(&state.context, &alpha);
    let registry_path = tmp.path().join("coordination/registry.json");
    let mut registry: Value = serde_json::from_slice(&fs::read(&registry_path).expect("registry"))
        .expect("registry json");
    registry["operations"]
        .as_array_mut()
        .expect("operations")
        .push(json!({
            "schema_version": "agent-session.operation-lease.v1",
            "lease_id": "completion-wins-lease",
            "session_id": "alpha",
            "session_incarnation": incarnation,
            "claim_id": "claim",
            "claim_revision": 1,
            "operation": "shell",
            "targets": [],
            "state": "active",
            "revision": 7,
            "started_at": "2030-01-01T00:00:00Z",
            "expires_at": "2030-01-01T01:00:00Z",
            "expires_at_epoch": 1,
            "execution_token_digest": "completion-token-digest",
            "runtime_identity_digest": runtime_identity_digest
        }));
    registry["completion_events"]
        .as_array_mut()
        .expect("completion events")
        .push(json!({
            "schema_version": "agent-session.operation-completion-event.v1",
            "event_id": "completion-event",
            "session_id": "alpha",
            "session_incarnation": incarnation,
            "lease_id": "completion-wins-lease",
            "if_revision": 7,
            "execution_token_digest": "completion-token-digest",
            "outcome": "pass",
            "idempotency_key": "provider-complete-alpha-0001",
            "request_digest": "provider-completion-request",
            "created_at_epoch": 1
        }));
    fs::write(
        &registry_path,
        serde_json::to_vec_pretty(&registry).expect("registry json"),
    )
    .expect("persist completion race");
    fs::set_permissions(&registry_path, fs::Permissions::from_mode(0o600)).expect("registry mode");

    let request = Request::builder()
        .method("POST")
        .uri("/sessions/alpha/operations/completion-wins-lease/operator-reconcile/v1")
        .header(AUTHORIZATION, format!("Bearer {TOKEN}"))
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&json!({
                "schema_version": "agent-session.operator-operation-reconcile-request.v1",
                "expected_session_incarnation": incarnation,
                "expected_session_generation": generation,
                "if_revision": 7,
                "reason": "post-tool-outcome-missing",
                "attest_inactive": true,
                "confirmed": true,
                "idempotency_key": "operator-reconcile-alpha-completion-race"
            }))
            .expect("operator request"),
        ))
        .expect("operator request");
    let (status, rejected) = call(router(state), request).await;
    assert_ne!(status, StatusCode::OK, "{rejected}");
    assert_eq!(
        rejected["error"]["code"], "operation-revision-conflict",
        "{rejected}"
    );

    let registry: Value = serde_json::from_slice(&fs::read(&registry_path).expect("registry"))
        .expect("registry json");
    let lease = registry["operations"]
        .as_array()
        .expect("operations")
        .iter()
        .find(|lease| lease["lease_id"] == "completion-wins-lease")
        .expect("lease");
    assert_eq!(lease["state"], "completed");
    assert_eq!(lease["outcome"], "pass");
    assert!(
        registry["completion_events"]
            .as_array()
            .expect("completion events")
            .is_empty()
    );
    assert!(
        registry["receipts"]
            .as_object()
            .expect("receipts")
            .values()
            .any(|receipt| {
                receipt["operation"] == "work-context-complete"
                    && receipt["outcome"]["state"] == "completed"
            }),
        "provider completion receipt must remain replayable"
    );
}

#[tokio::test]
async fn coordination_message_routes_keep_body_out_of_send_and_inbox() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    seed_session_with_runtime(tmp.path(), "alpha", "codex", "hs-codex-alpha");
    seed_session_with_runtime(tmp.path(), "beta", "codex", "hs-codex-beta");
    let state = state(tmp.path(), Some(TOKEN), minimal_tmux(tmp.path()));
    let alpha = load_session_record(&state.context, "alpha").expect("alpha");
    let beta = load_session_record(&state.context, "beta").expect("beta");
    let alpha_capability = provision_ready_coordination_fixture(&state.context, &alpha);
    let beta_capability = provision_ready_coordination_fixture(&state.context, &beta);
    let canary = "HTTP-PRIVATE-MESSAGE-CANARY";

    let (status, sent) = call(
        router(state.clone()),
        post_coordination(
            "/sessions/beta/messages/v1",
            alpha_capability.trim(),
            json!({
                "body": canary,
                "idempotency_key": "http-message-alpha-0001",
                "reply_to": null,
                "expires_in": null
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{sent}");
    assert!(!sent.to_string().contains(canary));
    assert_eq!(
        sent["data"]["coordination"]["notification"]["state"],
        "queued"
    );
    assert_eq!(
        sent["data"]["coordination"]["notification"]["generation"],
        1
    );
    assert_eq!(
        sent["data"]["coordination"]["notification"]["last_reason"],
        "notification-pending"
    );
    assert_eq!(
        sent["data"]["coordination"]["notification"]["controller_available"],
        true
    );
    let message_id = sent["data"]["coordination"]["message_id"]
        .as_str()
        .expect("message id")
        .to_string();
    let registry: Value = serde_json::from_slice(
        &fs::read(tmp.path().join("coordination/registry.json")).expect("registry"),
    )
    .expect("registry json");
    let notification = registry["notifications"]
        .as_object()
        .expect("notifications")
        .values()
        .next()
        .expect("notification generation");
    assert_eq!(
        notification["schema_version"],
        "agent-session.notification-generation.v1"
    );
    assert_eq!(notification["target_session_id"], "beta");
    assert_eq!(notification["generation"], 1);
    assert_eq!(notification["state"], "queued");

    let (status, inbox) = call(
        router(state.clone()),
        get_coordination("/sessions/beta/messages/v1", beta_capability.trim()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{inbox}");
    assert!(!inbox.to_string().contains(canary));

    let reply_canary = "HTTP-PRIVATE-REPLY-CANARY";
    let (status, reply) = call(
        router(state.clone()),
        post_coordination(
            &format!("/sessions/beta/messages/{message_id}/reply/v1"),
            beta_capability.trim(),
            json!({
                "body": reply_canary,
                "if_revision": 1,
                "idempotency_key": "http-message-reply-beta-0001"
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{reply}");
    assert!(!reply.to_string().contains(reply_canary));
    assert_eq!(
        reply["data"]["coordination"]["notification"]["state"],
        "queued"
    );
    assert_eq!(
        reply["data"]["coordination"]["notification"]["controller_available"],
        true
    );

    let (status, shown) = call(
        router(state),
        get_coordination(
            &format!("/sessions/beta/messages/{message_id}/v1"),
            beta_capability.trim(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{shown}");
    assert_eq!(
        shown["data"]["coordination"]["body"]["classification"],
        "untrusted_peer_data"
    );
    assert_eq!(shown["data"]["coordination"]["body"]["text"], canary);
}

#[tokio::test]
async fn coordination_notification_working_codex_uses_safe_in_turn_checkpoint() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let codex_home = tmp.path().join("codex-home");
    let transcript = codex_home.join("sessions/2030/01/active.jsonl");
    fs::create_dir_all(transcript.parent().expect("transcript parent")).expect("transcript dir");
    let provider_id = "notification-active-provider";
    let _codex_home = EnvGuard::set(&lock, "CODEX_HOME", codex_home.to_str().unwrap());
    let _broker = EnvGuard::set(
        &lock,
        "AGENT_SESSION_CODEX_ACCOUNT_BROKER",
        r#"["/configured/broker"]"#,
    );
    seed_session_with_runtime(tmp.path(), "alpha", "codex", "hs-codex-alpha");
    let launch_id = seed_codex_app_server_session(tmp.path(), "beta");
    let record_path = tmp.path().join("sessions/beta/session.json");
    let mut record: Value =
        serde_json::from_slice(&fs::read(&record_path).expect("beta record")).expect("beta json");
    record["provider_resume"] = json!({
        "provider": "codex",
        "session_id": provider_id,
        "captured_at": "2030-01-01T00:00:00Z",
        "capture_method": "fixture",
        "resume_args": []
    });
    fs::write(
        &record_path,
        serde_json::to_vec_pretty(&record).expect("beta record json"),
    )
    .expect("write beta provider resume");
    let state = state(tmp.path(), Some(TOKEN), minimal_tmux(tmp.path()));
    let alpha = load_session_record(&state.context, "alpha").expect("alpha");
    let beta = load_session_record(&state.context, "beta").expect("beta");
    let alpha_capability = provision_ready_coordination_fixture(&state.context, &alpha);
    provision_ready_coordination_fixture(&state.context, &beta);
    crate::activity::activate_runtime(&state.context, &beta).expect("activate beta");
    let turn = serde_json::from_value(json!({
        "schema_version": crate::activity::TURN_EVENT_VERSION,
        "event_id": "notification-active-turn-start",
        "runtime_id": launch_id,
        "provider": "codex",
        "provider_session_id": provider_id,
        "provider_turn_id": "notification-active-turn",
        "kind": "turn_started",
        "confidence": "authoritative"
    }))
    .expect("activity event");
    crate::activity::ingest_event(&state.context, &beta.id, turn).expect("ingest");
    let projected_turn_id = crate::activity::state_for_view(&state.context, &beta)
        .and_then(|turn| turn.current_turn)
        .and_then(|turn| turn.provider_turn_id)
        .expect("projected active turn id");
    assert_ne!(projected_turn_id, "notification-active-turn");
    crate::codex_account::queue_next_account_with_unbound(
        &state.context,
        &beta.id,
        &launch_id,
        "omega",
    )
    .expect("queue next account during the active turn");

    let canary = "ACTIVE-TURN-MAILBOX-BODY-MUST-NOT-REACH-CHECKPOINT";
    let (status, sent) = call(
        router(state.clone()),
        post_coordination(
            "/sessions/beta/messages/v1",
            alpha_capability.trim(),
            json!({
                "body": canary,
                "idempotency_key": "notification-active-turn-0001",
                "reply_to": null,
                "expires_in": null
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{sent}");

    let (handle, mut commands) = codex_app_server::control_channel();
    state.codex_controls.lock().unwrap().insert(
        beta.id.clone(),
        CodexControlEntry {
            launch_id: launch_id.clone(),
            handle,
        },
    );

    let registry_path = tmp.path().join("coordination/registry.json");
    let mut registry: Value = serde_json::from_slice(&fs::read(&registry_path).expect("registry"))
        .expect("registry json");
    registry["operations"]
            .as_array_mut()
            .expect("operations")
            .push(json!({
                "schema_version": "agent-session.operation-lease.v1",
                "lease_id": "active-turn-checkpoint-operation",
                "session_id": "beta",
                "session_incarnation": launch_id,
                "claim_id": "active-turn-checkpoint-claim",
                "claim_revision": 1,
                "operation": "atomic mutation",
                "targets": [],
                "provider_targets": [],
                "state": "active",
                "revision": 1,
                "started_at": "2030-01-01T00:00:00Z",
                "expires_at": "9999-12-31T23:59:59Z",
                "expires_at_epoch": i64::MAX,
                "terminal_at_epoch": null,
                "execution_token_digest": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "activity_revision": 1,
                "activity_identity_digest": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                "runtime_identity_digest": "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
                "descendant": null,
                "reconcile_observed_at_epoch": null,
                "outcome": null
            }));
    fs::write(
        &registry_path,
        serde_json::to_vec_pretty(&registry).expect("registry json"),
    )
    .expect("write active operation");
    drain_coordination_notifications(state.clone()).await;
    assert!(
        commands.try_recv().is_err(),
        "an active atomic operation must defer the in-turn checkpoint"
    );
    assert_eq!(
        notification_fixture(tmp.path())["last_reason"],
        "recipient-coordination-not-quiescent"
    );

    let mut registry: Value = serde_json::from_slice(&fs::read(&registry_path).expect("registry"))
        .expect("registry json");
    registry["operations"] = json!([]);
    registry["notifications"]
        .as_object_mut()
        .expect("notifications")
        .values_mut()
        .next()
        .expect("notification")["next_attempt_at_epoch"] = json!(0);
    fs::write(
        &registry_path,
        serde_json::to_vec_pretty(&registry).expect("registry json"),
    )
    .expect("clear active operation");
    let lock_probe_context = state.context.clone();
    let lifecycle_context = state.context.clone();
    let lifecycle_tmux = state.tmux_bin.clone();
    let maintenance_context = state.context.clone();
    let maintenance_tmux = state.tmux_bin.clone();
    let maintenance_preview = crate::maintenance::preview(
        &maintenance_context,
        "beta",
        &maintenance_tmux,
        crate::maintenance::MaintenanceOperation::Delete,
        crate::maintenance::MaintenanceContract::V1,
    )
    .expect("maintenance preview before notification submission");
    let inspect_preview = crate::maintenance::preview(
        &maintenance_context,
        "beta",
        &maintenance_tmux,
        crate::maintenance::MaintenanceOperation::Inspect,
        crate::maintenance::MaintenanceContract::V1,
    )
    .expect("inspect preview before notification submission");
    let attach_preview = crate::maintenance::preview(
        &maintenance_context,
        "beta",
        &maintenance_tmux,
        crate::maintenance::MaintenanceOperation::Attach,
        crate::maintenance::MaintenanceContract::V1,
    )
    .expect("attach preview before notification submission");
    let expected_prompt = fixture_notification_prompt(tmp.path());
    let responder = tokio::spawn(async move {
        let command = tokio::time::timeout(Duration::from_secs(60), commands.recv())
            .await
            .expect("working Codex notification must reach an in-turn checkpoint")
            .expect("Codex control command");
        let codex_app_server::ControlCommand::Steer {
            message,
            expected_turn_id,
            response,
        } = command
        else {
            panic!("working Codex notification must use turn/steer");
        };
        assert_eq!(expected_turn_id, projected_turn_id);
        assert_eq!(message, expected_prompt);
        assert!(!message.contains(canary));
        let lock_probe = tokio::task::spawn_blocking(move || {
            let _record_lock = crate::acquire_session_record_lock(&lock_probe_context, "beta")
                .expect("checkpoint record lock must be reacquirable");
        });
        tokio::time::timeout(Duration::from_secs(60), lock_probe)
            .await
            .expect("checkpoint must release the record lock before provider I/O")
            .expect("record-lock probe worker");
        let lifecycle = tokio::task::spawn_blocking(move || {
            crate::resume_session_by_id(&lifecycle_context, "beta", &lifecycle_tmux)
        });
        let error = tokio::time::timeout(Duration::from_secs(60), lifecycle)
            .await
            .expect("lifecycle admission must remain bounded during checkpoint provider I/O")
            .expect("lifecycle probe worker")
            .expect_err("lifecycle mutation must be fenced while checkpoint steer is pending");
        assert_eq!(
            error.code(),
            "coordination-notification-submission-in-progress"
        );
        let maintenance = tokio::task::spawn_blocking(move || {
            let inspect = crate::maintenance::execute_with_resume_guard(
                &maintenance_context,
                "beta",
                &maintenance_tmux,
                crate::maintenance::MaintenanceActionRequest {
                    schema_version: crate::maintenance::MaintenanceContract::V1,
                    operation: crate::maintenance::MaintenanceOperation::Inspect,
                    action: crate::maintenance::MaintenanceActionId::Inspect,
                    expected_session_incarnation: inspect_preview.session_incarnation.clone(),
                    expected_session_generation: inspect_preview.session_generation,
                    expected_preview_digest: inspect_preview.preview_digest.clone(),
                    confirmed: false,
                },
                |_| Ok(()),
            );
            let attach = crate::maintenance::execute_with_resume_guard(
                &maintenance_context,
                "beta",
                &maintenance_tmux,
                crate::maintenance::MaintenanceActionRequest {
                    schema_version: crate::maintenance::MaintenanceContract::V1,
                    operation: crate::maintenance::MaintenanceOperation::Attach,
                    action: crate::maintenance::MaintenanceActionId::RetryAttach,
                    expected_session_incarnation: attach_preview.session_incarnation.clone(),
                    expected_session_generation: attach_preview.session_generation,
                    expected_preview_digest: attach_preview.preview_digest.clone(),
                    confirmed: false,
                },
                |_| Ok(()),
            );
            let delete = crate::maintenance::execute_with_resume_guard(
                &maintenance_context,
                "beta",
                &maintenance_tmux,
                crate::maintenance::MaintenanceActionRequest {
                    schema_version: crate::maintenance::MaintenanceContract::V1,
                    operation: crate::maintenance::MaintenanceOperation::Delete,
                    action: crate::maintenance::MaintenanceActionId::RetryDelete,
                    expected_session_incarnation: maintenance_preview.session_incarnation.clone(),
                    expected_session_generation: maintenance_preview.session_generation,
                    expected_preview_digest: maintenance_preview.preview_digest.clone(),
                    confirmed: true,
                },
                |_| Ok(()),
            );
            (inspect, attach, delete)
        });
        let (inspect, attach, delete) = tokio::time::timeout(Duration::from_secs(60), maintenance)
            .await
            .expect("maintenance admission must remain bounded during checkpoint provider I/O")
            .expect("maintenance probe worker");
        let inspect = serde_json::to_value(
            inspect.expect("read-only inspect must remain available during checkpoint steer"),
        )
        .expect("inspect result json");
        assert_eq!(inspect["outcome"], "inspected");
        let attach = serde_json::to_value(
            attach.expect("read-only attach must remain available during checkpoint steer"),
        )
        .expect("attach result json");
        assert_eq!(attach["outcome"], "inspected");
        let maintenance_error =
            delete.expect_err("maintenance mutation must preserve the checkpoint retry contract");
        assert_eq!(
            maintenance_error.code(),
            "coordination-notification-submission-in-progress"
        );
        let maintenance_details = maintenance_error
            .0
            .details
            .expect("notification submission retry details");
        assert_eq!(maintenance_details["retryable"], true);
        assert_eq!(
            maintenance_details["next_action"],
            "wait-for-notification-outcome"
        );
        response
            .send(Ok(expected_turn_id))
            .expect("acknowledge checkpoint");
    });

    drain_coordination_notifications(state.clone()).await;
    responder.await.expect("checkpoint responder");

    let notification = notification_fixture(tmp.path());
    assert_eq!(notification["state"], "prompt_submitted");
    assert_eq!(notification["generation"], 1);
    assert_eq!(notification["notified_generation"], 1);
    let registry: Value = serde_json::from_slice(
        &fs::read(tmp.path().join("coordination/registry.json")).expect("registry"),
    )
    .expect("registry json");
    assert_eq!(registry["messages"][0]["state"], "unread");

    send_notification_fixture(
        &state,
        &alpha_capability,
        "notification-active-turn-unknown-0002",
    )
    .await;
    let (handle, mut commands) = codex_app_server::control_channel();
    state.codex_controls.lock().unwrap().insert(
        beta.id.clone(),
        CodexControlEntry {
            launch_id: launch_id.clone(),
            handle,
        },
    );

    let drain = tokio::spawn(drain_coordination_notifications(state.clone()));
    let command = tokio::time::timeout(Duration::from_secs(60), commands.recv())
        .await
        .expect("unknown steer attempt")
        .expect("steer command");
    let codex_app_server::ControlCommand::Steer {
        expected_turn_id,
        response,
        ..
    } = command
    else {
        panic!("active notification retry must use turn/steer");
    };
    assert_eq!(
        expected_turn_id,
        crate::activity::state_for_view(&state.context, &beta)
            .and_then(|turn| turn.current_turn)
            .and_then(|turn| turn.provider_turn_id)
            .expect("same projected active turn")
    );
    response
        .send(Err("provider rejected stale turn".to_string()))
        .expect("reject steer");
    drain.await.expect("unknown steer drain");
    assert_eq!(notification_fixture(tmp.path())["state"], "attempt_unknown");

    assert!(
        crate::coordination::pending_notifications(&state.context)
            .expect("pending notifications")
            .is_empty(),
        "an unresolved steer must not become a pending retry"
    );
    let duplicate_drain = tokio::spawn(drain_coordination_notifications(state.clone()));
    // A drain that submitted would block on its response, so finishing
    // first proves nothing was sent.
    tokio::select! {
        biased;
        joined = duplicate_drain => joined.expect("empty notification drain"),
        command = commands.recv() => match command {
            Some(codex_app_server::ControlCommand::Steer { response, .. }) => {
                let _ = response.send(Err("duplicate test steer".to_string()));
                panic!("an unresolved steer must not be submitted twice");
            }
            Some(_) => panic!("an unresolved steer emitted another control command"),
            None => panic!("the active control channel closed unexpectedly"),
        },
    }
    write_codex_notification_transcript(&transcript, provider_id, &[]);
    reconcile_coordination_notifications(state.clone()).await;
    assert_eq!(
        notification_fixture(tmp.path())["state"],
        "queued",
        "an absent transcript observation permits a bounded retry"
    );

    let drain = tokio::spawn(drain_coordination_notifications(state.clone()));
    let command = tokio::time::timeout(Duration::from_secs(60), commands.recv())
        .await
        .expect("reconciled steer retry")
        .expect("steer retry command");
    let codex_app_server::ControlCommand::Steer { response, .. } = command else {
        panic!("reconciled active notification must use turn/steer");
    };
    response
        .send(Err("steer acknowledgement lost".to_string()))
        .expect("lose steer acknowledgement");
    drain.await.expect("ambiguous steer drain");
    assert_eq!(notification_fixture(tmp.path())["state"], "attempt_unknown");
    write_codex_notification_transcript(
        &transcript,
        provider_id,
        &[(
            "9999-01-01T00:00:00Z",
            fixture_notification_prompt(tmp.path()),
        )],
    );
    reconcile_coordination_notifications(state.clone()).await;
    let notification = notification_fixture(tmp.path());
    assert_eq!(notification["state"], "prompt_submitted");
    assert_eq!(notification["notified_generation"], 2);
}

#[tokio::test]
async fn coordination_notification_startup_catchup_uses_one_acknowledged_codex_submission() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    seed_session_with_runtime(tmp.path(), "alpha", "codex", "hs-codex-alpha");
    let launch_id = seed_codex_app_server_session(tmp.path(), "beta");
    let state = state(tmp.path(), Some(TOKEN), minimal_tmux(tmp.path()));
    let alpha = load_session_record(&state.context, "alpha").expect("alpha");
    let beta = load_session_record(&state.context, "beta").expect("beta");
    let alpha_capability = provision_ready_coordination_fixture(&state.context, &alpha);
    provision_ready_coordination_fixture(&state.context, &beta);
    crate::activity::activate_runtime(&state.context, &beta).expect("activate beta");
    for (event_id, kind) in [
        ("notification-turn-start", "turn_started"),
        ("notification-turn-complete", "turn_completed"),
    ] {
        let event = serde_json::from_value(json!({
            "schema_version": crate::activity::TURN_EVENT_VERSION,
            "event_id": event_id,
            "runtime_id": launch_id,
            "provider": "codex",
            "provider_turn_id": "notification-idle-turn",
            "kind": kind,
            "confidence": "authoritative"
        }))
        .expect("activity event");
        crate::activity::ingest_event(&state.context, &beta.id, event).expect("ingest");
    }

    let canary = "CODEX-MAILBOX-BODY-MUST-NOT-REACH-PROMPT";
    let (status, sent) = call(
        router(state.clone()),
        post_coordination(
            "/sessions/beta/messages/v1",
            alpha_capability.trim(),
            json!({
                "body": canary,
                "idempotency_key": "notification-catchup-0001",
                "reply_to": null,
                "expires_in": null
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{sent}");

    let (handle, mut commands) = codex_app_server::control_channel();
    state.codex_controls.lock().unwrap().insert(
        beta.id.clone(),
        CodexControlEntry {
            launch_id: launch_id.clone(),
            handle,
        },
    );

    let registry_path = tmp.path().join("coordination/registry.json");
    let mut registry: Value = serde_json::from_slice(&fs::read(&registry_path).expect("registry"))
        .expect("registry json");
    registry["claims"]
        .as_array_mut()
        .expect("claims")
        .push(json!({
            "schema_version": "agent-session.work-context.v1",
            "session_id": "beta",
            "session_incarnation": launch_id,
            "claim_id": "app-server-notification-race-claim",
            "revision": 1,
            "state": "active",
            "intent": "implementation",
            "tier": "direct",
            "repositories": [],
            "worktrees": [],
            "provider_refs": [],
            "plan_refs": [],
            "scopes": [],
            "summary": "structured notification race fixture",
            "updated_at": "2030-01-01T00:00:00Z",
            "expires_at": "9999-12-31T23:59:59Z",
            "expires_at_epoch": i64::MAX
        }));
    fs::write(
        &registry_path,
        serde_json::to_vec_pretty(&registry).expect("registry json"),
    )
    .expect("write active claim");
    let claimed_drain = tokio::spawn(drain_coordination_notifications(state.clone()));
    tokio::select! {
        biased;
        joined = claimed_drain => joined.expect("claimed drain"),
        command = commands.recv() => {
            if let Some(codex_app_server::ControlCommand::Prompt { response, .. }) = command {
                let _ = response.send(Ok("turn-claimed-notification".to_string()));
            }
            panic!("active claim must prevent structured notification submission");
        }
    }

    let mut registry: Value = serde_json::from_slice(&fs::read(&registry_path).expect("registry"))
        .expect("registry json");
    registry["claims"] = json!([]);
    registry["notifications"]
        .as_object_mut()
        .expect("notifications")
        .values_mut()
        .next()
        .expect("notification")["next_attempt_at_epoch"] = json!(0);
    registry["operations"]
            .as_array_mut()
            .expect("operations")
            .push(json!({
                "schema_version": "agent-session.operation-lease.v1",
                "lease_id": "app-server-notification-race-operation",
                "session_id": "beta",
                "session_incarnation": launch_id,
                "claim_id": "app-server-notification-race-claim",
                "claim_revision": 1,
                "operation": "test mutation",
                "targets": [],
                "provider_targets": [],
                "state": "reconcile_pending",
                "revision": 1,
                "started_at": "2030-01-01T00:00:00Z",
                "expires_at": "9999-12-31T23:59:59Z",
                "expires_at_epoch": i64::MAX,
                "terminal_at_epoch": null,
                "execution_token_digest": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "activity_revision": 1,
                "activity_identity_digest": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                "runtime_identity_digest": "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
                "descendant": null,
                "reconcile_observed_at_epoch": null,
                "outcome": null
            }));
    fs::write(
        &registry_path,
        serde_json::to_vec_pretty(&registry).expect("registry json"),
    )
    .expect("write uncertain operation");
    let uncertain_drain = tokio::spawn(drain_coordination_notifications(state.clone()));
    tokio::select! {
        biased;
        joined = uncertain_drain => joined.expect("uncertain drain"),
        command = commands.recv() => {
            if let Some(codex_app_server::ControlCommand::Prompt { response, .. }) = command {
                let _ = response.send(Ok("turn-uncertain-notification".to_string()));
            }
            panic!("uncertain operation must prevent structured notification submission");
        }
    }

    let mut registry: Value = serde_json::from_slice(&fs::read(&registry_path).expect("registry"))
        .expect("registry json");
    registry["operations"] = json!([]);
    registry["notifications"]
        .as_object_mut()
        .expect("notifications")
        .values_mut()
        .next()
        .expect("notification")["next_attempt_at_epoch"] = json!(0);
    fs::write(
        &registry_path,
        serde_json::to_vec_pretty(&registry).expect("registry json"),
    )
    .expect("clear uncertain operation");

    let expected_prompt = fixture_notification_prompt(tmp.path());
    let responder = tokio::spawn(async move {
        let Some(codex_app_server::ControlCommand::Prompt { message, response }) =
            commands.recv().await
        else {
            panic!("notification did not reach the Codex control plane");
        };
        assert_eq!(message, expected_prompt);
        assert!(!message.contains(canary));
        response
            .send(Ok("turn-notification-catchup".to_string()))
            .expect("acknowledge prompt");
        commands
    });

    let first = tokio::spawn(drain_coordination_notifications(state.clone()));
    let second = tokio::spawn(drain_coordination_notifications(state.clone()));
    first.await.expect("first wakeup");
    second.await.expect("second wakeup");
    let mut commands = responder.await.expect("responder");
    assert!(
        commands.try_recv().is_err(),
        "racing wakeups must not submit a duplicate generation"
    );

    let registry: Value = serde_json::from_slice(
        &fs::read(tmp.path().join("coordination/registry.json")).expect("registry"),
    )
    .expect("registry json");
    let notification = registry["notifications"]
        .as_object()
        .expect("notifications")
        .values()
        .next()
        .expect("notification generation");
    assert_eq!(notification["state"], "prompt_submitted");
    assert_eq!(notification["generation"], 1);
    assert_eq!(notification["notified_generation"], 1);
    assert_eq!(registry["messages"][0]["state"], "unread");
    assert!(!notification.to_string().contains(canary));
}

#[tokio::test]
async fn coordination_notification_skips_a_mailbox_the_recipient_already_drained() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    seed_session_with_runtime(tmp.path(), "alpha", "codex", "hs-codex-alpha");
    let launch_id = seed_codex_app_server_session(tmp.path(), "beta");
    let state = state(tmp.path(), Some(TOKEN), minimal_tmux(tmp.path()));
    let alpha = load_session_record(&state.context, "alpha").expect("alpha");
    let beta = load_session_record(&state.context, "beta").expect("beta");
    let alpha_capability = provision_ready_coordination_fixture(&state.context, &alpha);
    provision_ready_coordination_fixture(&state.context, &beta);
    crate::activity::activate_runtime(&state.context, &beta).expect("activate beta");
    for (event_id, kind) in [
        ("drained-turn-start", "turn_started"),
        ("drained-turn-complete", "turn_completed"),
    ] {
        let event = serde_json::from_value(json!({
            "schema_version": crate::activity::TURN_EVENT_VERSION,
            "event_id": event_id,
            "runtime_id": launch_id,
            "provider": "codex",
            "provider_turn_id": "drained-idle-turn",
            "kind": kind,
            "confidence": "authoritative"
        }))
        .expect("activity event");
        crate::activity::ingest_event(&state.context, &beta.id, event).expect("ingest");
    }
    let (handle, mut commands) = codex_app_server::control_channel();
    state.codex_controls.lock().unwrap().insert(
        beta.id.clone(),
        CodexControlEntry {
            launch_id: launch_id.clone(),
            handle,
        },
    );
    let registry_path = tmp.path().join("coordination/registry.json");
    let drain_recipient_inbox = || {
        let mut registry: Value =
            serde_json::from_slice(&fs::read(&registry_path).expect("registry"))
                .expect("registry json");
        for message in registry["messages"].as_array_mut().expect("messages") {
            message["state"] = json!("acknowledged");
        }
        fs::write(
            &registry_path,
            serde_json::to_vec_pretty(&registry).expect("registry json"),
        )
        .expect("drain recipient inbox");
    };

    send_notification_fixture(&state, &alpha_capability, "drained-mailbox-0001").await;
    let raced = crate::coordination::pending_notifications(&state.context)
        .expect("pending notification")
        .pop()
        .expect("notification candidate");
    // The recipient reads its mail at its own safe boundary before the
    // reminder is submitted.
    drain_recipient_inbox();
    assert!(
        crate::coordination::pending_notifications(&state.context)
            .expect("pending notifications")
            .is_empty(),
        "a drained mailbox must not be offered for delivery"
    );
    assert!(
        !crate::coordination::lock_session_quiescence(&state.context, "beta", &launch_id)
            .expect("quiescence")
            .begin_notification_attempt(&raced)
            .expect("attempt"),
        "the locked attempt must recheck unread mail discovered earlier"
    );
    drain_coordination_notifications(state.clone()).await;
    assert!(
        tokio::time::timeout(Duration::from_millis(100), commands.recv())
            .await
            .is_err(),
        "no stale reminder may reach the recipient"
    );
    assert_eq!(notification_fixture(tmp.path())["state"], "queued");
    assert_eq!(notification_fixture(tmp.path())["notified_generation"], 0);

    send_notification_fixture(&state, &alpha_capability, "drained-mailbox-0002").await;
    let expected_prompt = fixture_notification_prompt(tmp.path());
    assert_eq!(expected_prompt, "Mailbox: agent-session message inbox");
    let responder = tokio::spawn(async move {
        let Some(codex_app_server::ControlCommand::Prompt { message, response }) =
            commands.recv().await
        else {
            panic!("new mail must be announced");
        };
        assert_eq!(message, expected_prompt);
        response
            .send(Ok("turn-drained-mailbox".to_string()))
            .expect("acknowledge prompt");
    });
    drain_coordination_notifications(state.clone()).await;
    responder.await.expect("responder");
    let notification = notification_fixture(tmp.path());
    assert_eq!(notification["state"], "prompt_submitted");
    assert_eq!(notification["notified_generation"], 2);
}

#[tokio::test]
async fn coordination_notification_leaves_dsh_recipients_to_hook_delivery() {
    // A DSH recipient has no serve prompt route: its policy hook claims
    // the reminder at a model-step boundary (sympoies/nils-cli#2011), so
    // serve must neither defer the generation forever nor report the
    // provider unsupported.
    // A retained record of the retired `hermes` kind has no prompt route,
    // whichever profile it was launched from.
    for (agent, dsh_console, expected_reason) in [
        ("dsh", false, "hook-delivered"),
        ("dsh", true, "hook-delivered"),
        ("hermes", true, "provider-unsupported"),
        ("hermes", false, "provider-unsupported"),
    ] {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        seed_session_with_runtime(tmp.path(), "alpha", "codex", "hs-codex-alpha");
        seed_session_with_runtime(tmp.path(), "beta", agent, "hs-beta");
        if dsh_console {
            let record_path = tmp.path().join("sessions/beta/session.json");
            let mut record: Value =
                serde_json::from_str(&fs::read_to_string(&record_path).unwrap()).unwrap();
            record["runtime"]["agent_profile"] = json!("dsh-workbench");
            record["agent_bin"] = json!("/opt/agent-console/bin/run-agent-console-workbench");
            fs::write(record_path, serde_json::to_vec_pretty(&record).unwrap()).unwrap();
        }
        let state = state(tmp.path(), Some(TOKEN), minimal_tmux(tmp.path()));
        let alpha = load_session_record(&state.context, "alpha").expect("alpha");
        let beta = load_session_record(&state.context, "beta").expect("beta");
        let alpha_capability = provision_ready_coordination_fixture(&state.context, &alpha);
        provision_ready_coordination_fixture(&state.context, &beta);
        send_notification_fixture(&state, &alpha_capability, "dsh-hook-delivery-0001").await;
        drain_coordination_notifications(state.clone()).await;
        let notification = notification_fixture(tmp.path());
        assert_eq!(
            notification["state"], "undeliverable",
            "agent={agent} dsh_console={dsh_console} notification={notification}"
        );
        assert_eq!(
            notification["last_reason"], expected_reason,
            "agent={agent} dsh_console={dsh_console}"
        );
        assert_eq!(notification["notified_generation"], 0);
    }
}

#[tokio::test]
async fn coordination_notification_reconcile_rejects_stale_same_second_generation() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let codex_home = tmp.path().join("codex-home");
    let transcript = codex_home.join("sessions/2026/07/session.jsonl");
    fs::create_dir_all(transcript.parent().expect("transcript parent")).expect("transcript dir");
    let provider_id = "notification-reconcile-stale";
    write_codex_notification_transcript(&transcript, provider_id, &[]);
    let _codex_home = EnvGuard::set(&lock, "CODEX_HOME", codex_home.to_str().unwrap());

    seed_session_with_runtime(tmp.path(), "alpha", "codex", "hs-codex-alpha");
    seed_fresh_provider_session(
        tmp.path(),
        "beta",
        "codex",
        "hs-codex-beta",
        tmp.path(),
        Some((provider_id, "codex-explicit-session-id")),
    );
    let state = state(tmp.path(), Some(TOKEN), minimal_tmux(tmp.path()));
    let alpha = load_session_record(&state.context, "alpha").expect("alpha");
    let beta = load_session_record(&state.context, "beta").expect("beta");
    let alpha_capability = provision_ready_coordination_fixture(&state.context, &alpha);
    provision_ready_coordination_fixture(&state.context, &beta);

    send_notification_fixture(&state, &alpha_capability, "notification-stale-gen-1").await;
    let generation_one = crate::coordination::pending_notifications(&state.context)
        .expect("generation one")
        .pop()
        .expect("generation one candidate");
    assert!(
        crate::coordination::begin_notification_attempt(&state.context, &generation_one)
            .expect("begin generation one")
    );
    assert!(
        crate::coordination::mark_notification_submitted(&state.context, &generation_one)
            .expect("submit generation one")
    );

    send_notification_fixture(&state, &alpha_capability, "notification-stale-gen-2").await;
    let generation_two = crate::coordination::pending_notifications(&state.context)
        .expect("generation two")
        .pop()
        .expect("generation two candidate");
    assert!(
        crate::coordination::begin_notification_attempt(&state.context, &generation_two)
            .expect("begin generation two")
    );
    assert!(
        crate::coordination::mark_notification_unknown(
            &state.context,
            &generation_two,
            "submission-outcome-unknown",
        )
        .expect("fail generation two")
    );
    write_codex_notification_transcript(
        &transcript,
        provider_id,
        &[(
            "2030-01-01T00:01:40.200000000Z",
            crate::coordination::notification_prompt(&generation_two),
        )],
    );
    set_notification_attempted_at(tmp.path(), "2030-01-01T00:01:40.700000000Z");

    reconcile_coordination_notifications(state.clone()).await;

    let notification = notification_fixture(tmp.path());
    assert_eq!(notification["state"], "queued");
    assert_eq!(notification["generation"], 2);
    assert_eq!(notification["notified_generation"], 1);
    assert_eq!(
        crate::coordination::pending_notifications(&state.context)
            .expect("requeued notification")
            .len(),
        1
    );
}

mod notification_reconciliation {
    include!("tests/notification_reconciliation.rs");
}

mod terminal_mailbox {
    include!("tests/terminal_mailbox.rs");
}

mod claude_mailbox {
    include!("tests/claude_mailbox.rs");
}

mod provider_prompt_source {
    include!("tests/provider_prompt_source.rs");
}

fn write_codex_notification_transcript(path: &Path, provider_id: &str, prompts: &[(&str, String)]) {
    let mut lines = vec![json!({
        "type":"session_meta",
        "payload":{
            "id":provider_id,
            "session_id":provider_id,
            "cwd":"/tmp",
            "source":"cli",
            "timestamp":"2030-01-01T00:00:00Z"
        }
    })];
    lines.extend(prompts.iter().map(|(timestamp, prompt)| {
        json!({
            "type":"event_msg",
            "timestamp":timestamp,
            "payload":{"type":"user_message","message":prompt}
        })
    }));
    let transcript = lines
        .into_iter()
        .map(|line| format!("{line}\n"))
        .collect::<String>();
    fs::write(path, transcript).expect("provider transcript");
}

async fn send_notification_fixture(
    state: &Arc<ServeState>,
    sender_capability: &str,
    idempotency_key: &str,
) {
    let (status, body) = call(
        router(state.clone()),
        post_coordination(
            "/sessions/beta/messages/v1",
            sender_capability.trim(),
            json!({
                "body": "notification reconciliation fixture",
                "idempotency_key": idempotency_key,
                "reply_to": null,
                "expires_in": null
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

fn set_notification_attempted_at(state_dir: &Path, attempted_at: &str) {
    let path = state_dir.join("coordination/registry.json");
    let mut registry: Value =
        serde_json::from_slice(&fs::read(&path).expect("registry")).expect("registry json");
    let notification = registry["notifications"]
        .as_object_mut()
        .expect("notifications")
        .values_mut()
        .next()
        .expect("notification");
    let timestamp = attempted_at
        .parse::<jiff::Timestamp>()
        .expect("attempted timestamp");
    notification["attempted_at"] = json!(attempted_at);
    notification["attempted_at_epoch"] = json!(timestamp.as_second());
    fs::write(
        path,
        serde_json::to_vec_pretty(&registry).expect("registry json"),
    )
    .expect("registry");
}

use notification_reconciliation::fixture_notification_prompt;

fn notification_fixture(state_dir: &Path) -> Value {
    let registry: Value = serde_json::from_slice(
        &fs::read(state_dir.join("coordination/registry.json")).expect("registry"),
    )
    .expect("registry json");
    registry["notifications"]
        .as_object()
        .expect("notifications")
        .values()
        .next()
        .expect("notification")
        .clone()
}

#[tokio::test]
async fn coordination_wait_cancellation_releases_the_bounded_worker() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    seed_session_with_runtime(tmp.path(), "alpha", "codex", "hs-codex-alpha");
    seed_session_with_runtime(tmp.path(), "beta", "codex", "hs-codex-beta");
    let state = state(tmp.path(), Some(TOKEN), minimal_tmux(tmp.path()));
    let alpha = load_session_record(&state.context, "alpha").expect("alpha");
    let beta = load_session_record(&state.context, "beta").expect("beta");
    let alpha_capability = provision_ready_coordination_fixture(&state.context, &alpha);
    let beta_capability = provision_ready_coordination_fixture(&state.context, &beta);
    let (status, sent) = call(
        router(state.clone()),
        post_coordination(
            "/sessions/beta/messages/v1",
            alpha_capability.trim(),
            json!({
                "body": "wait-cancellation-fixture",
                "idempotency_key": "http-wait-cancel-0001",
                "reply_to": null,
                "expires_in": null
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{sent}");
    let message_id = sent["data"]["coordination"]["message_id"]
        .as_str()
        .expect("message id")
        .to_string();
    let waiting_state = state.clone();
    let wait_path =
        format!("/sessions/beta/messages/{message_id}/wait/v1?if_revision=1&timeout=60s");
    let wait_capability = beta_capability.trim().to_string();
    let request = tokio::spawn(async move {
        call(
            router(waiting_state),
            get_coordination(&wait_path, &wait_capability),
        )
        .await
    });
    for _ in 0..50 {
        if state.coordination_wait_workers.available_permits() == COORDINATION_WAIT_WORKER_LIMIT - 1
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(
        state.coordination_wait_workers.available_permits(),
        COORDINATION_WAIT_WORKER_LIMIT - 1
    );
    request.abort();
    let _ = request.await;
    for _ in 0..50 {
        if state.coordination_wait_workers.available_permits() == COORDINATION_WAIT_WORKER_LIMIT {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(
        state.coordination_wait_workers.available_permits(),
        COORDINATION_WAIT_WORKER_LIMIT
    );
}

#[tokio::test]
async fn emergency_shell_routes_require_authentication() {
    let tmp = tempfile::TempDir::new().unwrap();
    let app = router(state(tmp.path(), Some(TOKEN), minimal_tmux(tmp.path())));
    let (status, _) = call(app, get("/shells/alice")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    for uri in [
        "/shells/alice/attach",
        "/shells/alice/attach?incarnation=old",
        "/shells/alice/attach?incarnation[]=bad",
    ] {
        let response = router(state(tmp.path(), Some(TOKEN), minimal_tmux(tmp.path())))
            .oneshot(get(uri))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
}

#[tokio::test]
async fn emergency_shell_status_does_not_create_a_runtime() {
    let tmp = tempfile::TempDir::new().unwrap();
    let tmux = executable(
        &tmp.path().join("tmux-stopped"),
        "#!/bin/sh\nprintf 'no server running\\n' >&2\nexit 1\n",
    );
    let app = router(state(tmp.path(), Some(TOKEN), tmux));
    let (status, body) = call(app, get_auth("/shells/alice", Some(TOKEN))).await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["data"]["shell"]["status"], "stopped");
    assert_eq!(body["data"]["shell"]["owner"], "alice");
    assert!(!tmp.path().join("sessions").exists());
}

fn test_record(id: &str, tmux_session: &str) -> crate::SessionRecord {
    crate::SessionRecord {
        schema_version: crate::SESSION_DOCUMENT_VERSION.to_string(),
        id: id.to_string(),
        agent: "codex".to_string(),
        mode: "interactive".to_string(),
        coordination_mode: crate::cli::CoordinationMode::Advisory,
        title: None,
        title_state: None,
        title_revision: 0,
        cwd: "/tmp".to_string(),
        tmux_session: tmux_session.to_string(),
        prompt_file: None,
        log_file: None,
        created_at: "2000-01-01T00:00:00Z".to_string(),
        updated_at: "2000-01-01T00:00:00Z".to_string(),
        provider_resume: None,
        runtime: None,
        public_metadata: None,
        agent_args: Vec::new(),
        agent_bin: None,
        extra: std::collections::BTreeMap::new(),
        lineage: None,
        work: None,
        lineage_adoption: None,
        role: None,
        resume_sidecar_extra: std::collections::BTreeMap::new(),
    }
}

pub(super) fn provider_discovery_record(
    id: &str,
    tmux_session: &str,
    launch_id: &str,
    generation: u64,
) -> crate::SessionRecord {
    let mut record = test_record(id, tmux_session);
    record.agent = "claude".to_string();
    record.provider_resume = Some(crate::ProviderResume {
        provider: "claude".to_string(),
        session_id: format!("{id}-provider"),
        captured_at: "2026-07-11T00:00:00Z".to_string(),
        capture_method: "claude-explicit-session-id".to_string(),
        resume_args: vec!["--resume".to_string(), format!("{id}-provider")],
        extra: std::collections::BTreeMap::new(),
    });
    record.runtime = Some(crate::RuntimeInfo {
        kind: "tmux".to_string(),
        tmux_session: tmux_session.to_string(),
        generation,
        started_at: "2026-07-11T00:00:00Z".to_string(),
        launch_id: launch_id.to_string(),
        extra: std::collections::BTreeMap::new(),
    });
    record
}

#[path = "tests/prompt_composer.rs"]
mod prompt_composer;
use prompt_composer::{
    empty_composer_tmux, empty_composer_tmux_marking_enter, minimal_tmux_showing,
};
#[path = "tests/prompt_receipts.rs"]
mod prompt_receipts_tests;

#[path = "tests/launch_env.rs"]
mod launch_env_tests;
