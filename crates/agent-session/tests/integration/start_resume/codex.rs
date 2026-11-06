//! Canonical Codex admission using only disposable provider fixtures.
use super::*;
use pretty_assertions::assert_eq;

fn fixture() -> Fixture {
    let f = Fixture::new();
    fs::create_dir_all(f.root.join("sessions/2026/01/01")).unwrap();
    fs::write(f.root.join(format!("sessions/2026/01/01/rollout-{CONVERSATION}.jsonl")),
        json!({"type":"session_meta","payload":{"id":CONVERSATION,"cwd":f.tmp.path(),"source":"cli","timestamp":"2026-01-01T00:00:00Z"}}).to_string()).unwrap();
    assert!(
        nils_provider_resume::resolve_resume_source_in_config_dir(
            nils_provider_resume::ResumeProvider::Codex,
            &f.root,
            CONVERSATION
        )
        .is_ok()
    );
    let mut source: Value =
        serde_json::from_slice(&fs::read(f.record_path("source")).unwrap()).unwrap();
    source["agent"] = json!("codex");
    source["provider_resume"]["provider"] = json!("codex");
    source["codex_account_binding"] = json!({"schema_version":"agent-session.codex-account-binding.v1",
        "selected_account":"original","revision":1,"state":"bound","applied_runtime_id":"source-runtime","updated_at":"2026-01-01T00:00:00Z"});
    source
        .as_object_mut()
        .unwrap()
        .remove("claude_account_binding");
    fs::write(f.record_path("source"), source.to_string()).unwrap();
    executable(
        &f.broker,
        &format!(
            r#"#!/bin/sh
printf '%s\n' "$*" >> {}
[ "$1" = list ] || exit 64
printf '%s\n' '{{"schema_version":"agent-session.codex-auth-broker.v1","accounts":[{{"account":"original"}}]}}'
"#,
            shell_words::quote(f.tmp.path().join("broker-calls").to_str().unwrap())
        ),
    );
    executable(
        &f.tmp.path().join("provider"),
        r#"#!/bin/sh
case "$*" in
--version) echo 'codex-cli 0.145.0';;
'app-server --help') echo '--listen <URL> unix://PATH';;
*) exit 1;;
esac
"#,
    );
    f
}
fn run(f: &Fixture, extra: &[&str]) -> nils_test_support::cmd::CmdOutput {
    let (_runtime, socket) = nils_test_support::short_socket_path("resume.sock");
    let runtime_root = socket.parent().unwrap();
    fs::set_permissions(runtime_root, fs::Permissions::from_mode(0o700)).unwrap();
    let runtime_before = deleted_source::snapshot(runtime_root);
    let options = CmdOptions::new()
        .with_env_remove_prefix("AGENT_SESSION_")
        .with_env("HOME", f.tmp.path().to_str().unwrap())
        .with_env("CODEX_HOME", f.root.to_str().unwrap())
        .with_env("XDG_RUNTIME_DIR", runtime_root.to_str().unwrap())
        .with_env(
            "AGENT_HOME",
            f.tmp.path().join("agent-home").to_str().unwrap(),
        )
        .with_env(
            "AGENT_SESSION_CODEX_ACCOUNT_BROKER",
            &json!([f.broker]).to_string(),
        )
        .with_env(
            "AGENT_SESSION_CODEX_BIN",
            f.tmp.path().join("provider").to_str().unwrap(),
        )
        .with_env("AGENT_SESSION_TMUX_BIN", f.tmux.to_str().unwrap())
        .with_env(
            "AGENT_SESSION_FAKE_TMUX_LOG",
            f.tmp.path().join("tmux.log").to_str().unwrap(),
        )
        .with_env("AGENT_SESSION_FAKE_TMUX_ABSENT_BEFORE_LAUNCH", "1");
    let mut args = vec![
        "--state-dir",
        f.state.to_str().unwrap(),
        "--host",
        "test-machine",
        "start",
        "--agent",
        "codex",
        "--resume",
        CONVERSATION,
        "--format",
        "json",
    ];
    args.extend_from_slice(extra);
    let out = run_resolved("agent-session", &args, &options);
    if extra.contains(&"--check-resume") {
        assert_eq!(
            deleted_source::snapshot(runtime_root),
            runtime_before,
            "preflight must not prepare a socket directory"
        );
    }
    out
}
#[test]
fn codex_preflight_is_exact_read_only_and_resolves_original_account() {
    let f = fixture();
    let before = deleted_source::snapshot(&f.state);
    let history = deleted_source::snapshot(&f.root);
    let out = run(
        &f,
        &["--check-resume", "--prompt-file", "/nonexistent/prompt"],
    );
    assert_eq!(
        out.stdout_json(),
        json!({"schema_version":"cli.agent-session.resume-preflight.v1","ok":true,
        "data":{"agent":"codex","provider_session_id":CONVERSATION,"machine":"test-machine","account":"original"}})
    );
    assert_eq!(deleted_source::snapshot(&f.state), before);
    assert_eq!(deleted_source::snapshot(&f.root), history);
    assert!(
        fs::read_to_string(f.tmp.path().join("broker-calls"))
            .unwrap()
            .lines()
            .all(|line| line == "list --format json")
    );
}
#[test]
fn codex_start_uses_same_conversation_new_managed_id_and_normal_registration() {
    let f = fixture();
    super::super::cli::fake_tmux(f.tmp.path());
    let source = fs::read(f.record_path("source")).unwrap();
    let out = run(
        &f,
        &[
            "--id",
            "replacement",
            "--no-parent",
            "--role",
            "coordinator",
            "--workspace",
            "example",
            "--issue",
            "example/project#7",
            "--title",
            "Resume fixture",
            "--title-mode",
            "pinned",
            "--prompt",
            "fixture bootstrap",
            "--agent-arg=--model",
            "--agent-arg=fixture-model",
            "--paste-delay-ms",
            "0",
        ],
    );
    assert_eq!(out.code, 0, "{} {}", out.stdout_text(), out.stderr_text());
    assert_eq!(
        out.stdout_json()["schema_version"],
        "cli.agent-session.start.v1"
    );
    assert_eq!(out.stdout_json()["data"]["session"]["id"], "replacement");
    let created: Value =
        serde_json::from_slice(&fs::read(f.record_path("replacement")).unwrap()).unwrap();
    assert_eq!(created["provider_resume"]["session_id"], CONVERSATION);
    assert_eq!(
        created["provider_resume"]["resume_args"],
        json!([
            "resume",
            CONVERSATION,
            "--cd",
            f.tmp.path(),
            "--no-alt-screen"
        ])
    );
    assert_eq!(
        created["codex_account_binding"]["selected_account"],
        "original"
    );
    assert_eq!(created["role"], "coordinator");
    assert_eq!(created["workspace"], "example");
    assert_eq!(created["title_mode"], "pinned");
    assert_eq!(created["work"]["issues"][0]["number"], 7);
    assert_eq!(fs::read(f.record_path("source")).unwrap(), source);
    let calls = fs::read_to_string(f.tmp.path().join("tmux.log")).unwrap();
    assert!(calls.contains(CONVERSATION));
    assert!(!calls.contains("--session-id"));
    assert!(calls.contains("fixture-model"));
    assert_eq!(
        fs::read_to_string(f.state.join("sessions/replacement/prompt.md")).unwrap(),
        "fixture bootstrap"
    );
}

#[test]
fn codex_admission_refuses_wrong_account_capability_and_unsafe_sources() {
    use std::os::fd::AsRawFd;
    for case in [
        "wrong-account",
        "missing-account",
        "unlisted",
        "broker-unavailable",
        "capability",
        "history",
        "cwd",
        "live",
        "unknown",
        "busy",
        "second-account",
        "pending-switch",
        "reserved-args",
        "alternate-provider",
    ] {
        let f = fixture();
        let mut extra = Vec::new();
        let mut held = None;
        let expected = match case {
            "reserved-args" => {
                extra.push("--agent-arg=--last");
                "resume-unsupported"
            }
            "alternate-provider" => {
                extra.push("--agent-arg=--oss");
                "resume-unsupported"
            }
            "wrong-account" => {
                extra.extend(["--account", "different"]);
                "resume-account-mismatch"
            }
            "missing-account" | "pending-switch" => {
                let mut source: Value =
                    serde_json::from_slice(&fs::read(f.record_path("source")).unwrap()).unwrap();
                if case == "missing-account" {
                    source
                        .as_object_mut()
                        .unwrap()
                        .remove("codex_account_binding");
                } else {
                    source["codex_account_next"] = json!({"schema_version":"agent-session.codex-account-next.v1", "account":"other","revision":1,"state":"queued","updated_at":"2026-01-01T00:00:00Z"});
                }
                fs::write(f.record_path("source"), source.to_string()).unwrap();
                "resume-source-unknown"
            }
            "unlisted" => {
                executable(
                    &f.broker,
                    "#!/bin/sh\necho '{\"schema_version\":\"agent-session.codex-auth-broker.v1\",\"accounts\":[]}'\n",
                );
                "resume-account-mismatch"
            }
            "broker-unavailable" => {
                executable(&f.broker, "#!/bin/sh\nexit 64\n");
                "resume-unsupported"
            }
            "capability" => {
                executable(
                    &f.tmp.path().join("provider"),
                    "#!/bin/sh\necho 'codex-cli 0.1.0'\n",
                );
                "resume-unsupported"
            }
            "history" => {
                fs::remove_file(
                    f.root
                        .join(format!("sessions/2026/01/01/rollout-{CONVERSATION}.jsonl")),
                )
                .unwrap();
                "resume-history-missing"
            }
            "cwd" => {
                extra.extend(["--cwd", "/"]);
                "resume-unsupported"
            }
            "live" => {
                executable(&f.tmux, "#!/bin/sh\nexit 0\n");
                "resume-source-live"
            }
            "unknown" => {
                executable(&f.tmux, "#!/bin/sh\nexit 2\n");
                "resume-source-unknown"
            }
            "second-account" => {
                let mut source: Value =
                    serde_json::from_slice(&fs::read(f.record_path("source")).unwrap()).unwrap();
                source["id"] = json!("second");
                source["codex_account_binding"]["selected_account"] = json!("different");
                fs::create_dir_all(f.state.join("sessions/second")).unwrap();
                fs::write(f.record_path("second"), source.to_string()).unwrap();
                fs::write(f.state.join("session-locks/second.lock"), "").unwrap();
                "resume-source-unknown"
            }
            "busy" => {
                let file = fs::File::open(f.state.join("session-locks/source.lock")).unwrap();
                assert_eq!(
                    unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
                    0
                );
                held = Some(file);
                "resume-source-live"
            }
            _ => unreachable!(),
        };
        let before = deleted_source::snapshot(&f.state);
        for checking in [true, false] {
            let mut args = extra.clone();
            args.extend(["--id", "blocked"]);
            if checking {
                args.push("--check-resume");
            }
            let out = run(&f, &args);
            assert_eq!(
                out.stdout_json()["error"]["code"],
                expected,
                "{case}: {}",
                out.stdout_text()
            );
            assert_eq!(
                out.stdout_json()["error"]["details"],
                json!({"session_created":false,"safe_to_retry":false}),
                "{case}"
            );
            assert_eq!(deleted_source::snapshot(&f.state), before, "{case}");
        }
        drop(held);
    }
}

#[test]
fn codex_start_rechecks_sources_and_history_after_runtime_preparation() {
    for case in ["account", "history", "new-source"] {
        let f = fixture();
        assert_eq!(run(&f, &["--check-resume"]).code, 0);
        let quote = |p: &Path| shell_words::quote(p.to_str().unwrap()).into_owned();
        let hook = match case {
            "history" => format!(
                "rm -- {}",
                quote(
                    &f.root
                        .join(format!("sessions/2026/01/01/rollout-{CONVERSATION}.jsonl"))
                )
            ),
            _ => {
                let mut source: Value =
                    serde_json::from_slice(&fs::read(f.record_path("source")).unwrap()).unwrap();
                let target = if case == "account" {
                    source["codex_account_binding"]["selected_account"] = json!("other");
                    f.record_path("source")
                } else {
                    source["id"] = json!("late-source");
                    source["codex_account_binding"]["state"] = json!("pending");
                    fs::create_dir_all(f.state.join("sessions/late-source")).unwrap();
                    fs::write(f.state.join("session-locks/late-source.lock"), "").unwrap();
                    f.record_path("late-source")
                };
                let staged = f.tmp.path().join("raced-source");
                fs::write(&staged, source.to_string()).unwrap();
                format!("cp {} {}", quote(&staged), quote(&target))
            }
        };
        // The first two probes belong to observational and guarded admission.
        // The third is start-only runtime preparation, before final admission.
        let provider = f.tmp.path().join("provider");
        let counter = quote(&f.tmp.path().join("probe-count"));
        let script = fs::read_to_string(&provider).unwrap();
        executable(
            &provider,
            &script.replace(
                "--version) echo",
                &format!("--version) count=$(cat {counter} 2>/dev/null || printf 0); count=$((count + 1)); printf '%s\\n' \"$count\" > {counter}; if [ \"$count\" -eq 3 ]; then {hook}; fi; echo"),
            ),
        );
        let out = run(&f, &["--id", "raced"]);
        let expected = match case {
            "history" => "resume-history-missing",
            "account" => "resume-account-mismatch",
            _ => "resume-source-unknown",
        };
        assert_eq!(
            out.stdout_json()["error"]["code"],
            expected,
            "{case}: {}",
            out.stdout_text()
        );
        assert_eq!(
            out.stdout_json()["error"]["details"],
            json!({"session_created":false,"safe_to_retry":false})
        );
        assert!(!f.state.join("sessions/raced").exists());
        assert!(!f.tmp.path().join("tmux.log").exists());
    }
}

#[test]
fn codex_preflight_uses_retained_provider_root_without_default_history_fallback() {
    let f = fixture();
    let retained = f.tmp.path().join("retained-provider");
    fs::rename(&f.root, &retained).unwrap();
    let mut source: Value =
        serde_json::from_slice(&fs::read(f.record_path("source")).unwrap()).unwrap();
    source["runtime"]["agent_profile_provider_config_dir"] = json!(retained);
    fs::write(f.record_path("source"), source.to_string()).unwrap();
    let out = run(&f, &["--check-resume"]);
    assert_eq!(out.code, 0, "{}", out.stdout_text());
    assert_eq!(out.stdout_json()["data"]["account"], "original");
    fs::remove_file(retained.join(format!("sessions/2026/01/01/rollout-{CONVERSATION}.jsonl")))
        .unwrap();
    fs::create_dir_all(f.root.join("sessions")).unwrap();
    fs::write(f.root.join(format!("sessions/rollout-{CONVERSATION}.jsonl")),
        json!({"type":"session_meta","payload":{"id":CONVERSATION,"cwd":f.tmp.path(),"source":"cli","timestamp":"2026-01-01T00:00:00Z"}}).to_string()).unwrap();
    let out = run(&f, &["--check-resume"]);
    assert_eq!(out.stdout_json()["error"]["code"], "resume-history-missing");
}
