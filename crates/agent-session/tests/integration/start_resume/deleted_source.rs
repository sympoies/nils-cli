use super::*;
use pretty_assertions::assert_eq;

fn deleted_source() -> Fixture {
    let f = Fixture::new();
    fs::remove_dir_all(f.state.join("sessions/source")).unwrap();
    fs::remove_file(f.state.join("session-locks/source.lock")).unwrap();
    fs::write(
        f.tmp.path().join("history-root.json"),
        json!({"schema_version":"agent-session.account-broker.v2","provider":"claude",
            "account":"original","config_dir":f.root,"history_scope":"account"})
        .to_string(),
    )
    .unwrap();
    fs::write(f.tmp.path().join("materialize-hook"), "").unwrap();
    executable(
        &f.broker,
        &format!(
            r#"#!/bin/sh
printf '%s\n' "$*" >> {calls}
case "$1" in
list) printf '%s\n' '{{"schema_version":"agent-session.account-broker.v2","provider":"claude","accounts":[{{"account":"original"}}]}}' ;;
history-root) cat {response} ;;
materialize) sh {hook}; printf '%s\n' '{prepared}' ;;
*) exit 64 ;;
esac
"#,
            calls = shell_words::quote(f.tmp.path().join("broker-calls").to_str().unwrap()),
            response = shell_words::quote(f.tmp.path().join("history-root.json").to_str().unwrap()),
            hook = shell_words::quote(f.tmp.path().join("materialize-hook").to_str().unwrap()),
            prepared = json!({"schema_version":"agent-session.account-broker.v2","provider":"claude","account":"original","config_dir":f.root}),
        ),
    );
    f
}

fn snapshot(root: &Path) -> Vec<(PathBuf, Option<Vec<u8>>)> {
    fn walk(root: &Path, path: &Path, rows: &mut Vec<(PathBuf, Option<Vec<u8>>)>) {
        let directory = path.is_dir();
        rows.push((
            path.strip_prefix(root).unwrap().to_path_buf(),
            (!directory).then(|| fs::read(path).unwrap()),
        ));
        if directory {
            for entry in fs::read_dir(path).unwrap() {
                walk(root, &entry.unwrap().path(), rows);
            }
        }
    }
    let mut rows = Vec::new();
    walk(root, root, &mut rows);
    rows.sort_by(|a, b| a.0.cmp(&b.0));
    rows
}

#[test]
fn explicit_account_recovers_deleted_source_without_preflight_writes() {
    for shared in [false, true] {
        let f = deleted_source();
        let scope = if shared { "shared" } else { "account" };
        if shared {
            let shared_projects = f.tmp.path().join("shared-projects");
            fs::rename(f.root.join("projects"), &shared_projects).unwrap();
            std::os::unix::fs::symlink(&shared_projects, f.root.join("projects")).unwrap();
            fs::write(f.tmp.path().join("history-root.json"),
            json!({"schema_version":"agent-session.account-broker.v2","provider":"claude","account":"original","config_dir":f.root,"history_scope":scope}).to_string()).unwrap();
        }
        let state = snapshot(&f.state);
        let account = snapshot(&f.root);
        let out = f.run(&[
            "--account",
            "original",
            "--check-resume",
            "--prompt-file",
            "/nonexistent/prompt",
        ]);
        assert_eq!(out.code, 0, "{} {}", out.stdout_text(), out.stderr_text());
        assert_eq!(
            out.stdout_json(),
            json!({"schema_version":"cli.agent-session.resume-preflight.v1","ok":true,
        "data":{"agent":"claude","provider_session_id":CONVERSATION,"machine":"test-machine","account":"original","history_scope":scope}})
        );
        assert_eq!(snapshot(&f.state), state);
        assert_eq!(snapshot(&f.root), account);
        let calls = fs::read_to_string(f.tmp.path().join("broker-calls")).unwrap();
        assert!(
            calls
                .lines()
                .any(|line| line
                    == "history-root --provider claude --account original --format json")
        );
        assert!(
            calls
                .lines()
                .all(|line| line.starts_with("list ") || line.starts_with("history-root "))
        );

        super::super::cli::fake_tmux(f.tmp.path());
        super::super::cli::fake_agent(f.tmp.path(), "provider");
        let out = f.run(&[
            "--account",
            "original",
            "--id",
            "replacement",
            "--no-parent",
            "--role",
            "coordinator",
            "--workspace",
            "example",
            "--issue",
            "example/project#7",
            "--prompt",
            "fixture bootstrap",
            "--paste-delay-ms",
            "0",
        ]);
        assert_eq!(out.code, 0, "{} {}", out.stdout_text(), out.stderr_text());
        assert_eq!(out.stdout_json()["data"]["session"]["id"], "replacement");
        let created: Value =
            serde_json::from_slice(&fs::read(f.record_path("replacement")).unwrap()).unwrap();
        assert_eq!(created["provider_resume"]["session_id"], CONVERSATION);
        assert_eq!(
            created["claude_account_binding"]["selected_account"],
            "original"
        );
        assert_eq!(created["role"], "coordinator");
        assert_eq!(created["workspace"], "example");
        assert_eq!(created["work"]["issues"][0]["number"], 7);
        assert!(!f.record_path("source").exists());
        assert_eq!(fs::read_dir(f.state.join("sessions")).unwrap().count(), 1);
        let calls = fs::read_to_string(f.tmp.path().join("tmux.log")).unwrap();
        assert!(calls.contains("--resume"));
        assert!(calls.contains(CONVERSATION));
        assert!(!calls.contains("--session-id"));
        assert_eq!(
            fs::read_to_string(f.state.join("sessions/replacement/prompt.md")).unwrap(),
            "fixture bootstrap"
        );
        let out = f.run(&["--account", "original", "--id", "second-writer"]);
        assert_eq!(out.stdout_json()["error"]["code"], "resume-source-live");
        assert!(!f.record_path("second-writer").exists());
        assert_eq!(fs::read_dir(f.state.join("sessions")).unwrap().count(), 1);
    }
}

#[test]
fn broken_session_directory_is_reported_not_fatal() {
    let f = deleted_source();
    fs::create_dir_all(f.state.join("sessions/broken")).unwrap();
    let out = f.run(&["--account", "original", "--check-resume"]);
    assert_eq!(out.code, 0, "{} {}", out.stdout_text(), out.stderr_text());
    assert_eq!(out.stdout_json()["ok"], true);
    assert!(
        out.stderr_text().contains("broken"),
        "{}",
        out.stderr_text()
    );
}

#[test]
fn deleted_source_requires_account_authority_and_exact_scoped_history() {
    for case in [
        "omitted",
        "unknown",
        "missing",
        "duplicate",
        "misnamed-history",
        "unsupported",
        "malformed",
        "wrong-account",
        "wrong-provider",
        "wrong-schema",
        "missing-scope",
        "invalid-scope",
        "unsafe-root",
    ] {
        let f = deleted_source();
        let history = f
            .root
            .join(format!("projects/project/{CONVERSATION}.jsonl"));
        let response = f.tmp.path().join("history-root.json");
        let mut account = "original";
        let code = match case {
            "omitted" => {
                // Default-login history still does not establish broker account authority.
                let default = f.tmp.path().join("unrelated-default/projects/project");
                fs::create_dir_all(&default).unwrap();
                fs::copy(&history, default.join(format!("{CONVERSATION}.jsonl"))).unwrap();
                "resume-source-unknown"
            }
            "unknown" => {
                account = "unknown";
                "resume-account-mismatch"
            }
            "missing" => {
                // A copy outside the selected account must not become a fallback.
                let other = f.tmp.path().join("unrelated-default/projects/project");
                fs::create_dir_all(&other).unwrap();
                fs::rename(&history, other.join(format!("{CONVERSATION}.jsonl"))).unwrap();
                "resume-history-missing"
            }
            "duplicate" => {
                let other = f.root.join("projects/duplicate");
                fs::create_dir_all(&other).unwrap();
                fs::copy(&history, other.join(format!("{CONVERSATION}.jsonl"))).unwrap();
                "resume-source-unknown"
            }
            "misnamed-history" => {
                fs::rename(&history, history.with_file_name("unrelated.jsonl")).unwrap();
                "resume-history-missing"
            }
            "unsupported" => {
                executable(
                    &f.broker,
                    &fs::read_to_string(&f.broker)
                        .unwrap()
                        .replace("history-root) cat", "history-root) exit 64; cat"),
                );
                "resume-unsupported"
            }
            "malformed" => {
                fs::write(&response, "{}").unwrap();
                "resume-unsupported"
            }
            other => {
                let mut value: Value =
                    serde_json::from_slice(&fs::read(&response).unwrap()).unwrap();
                match other {
                    "wrong-account" => value["account"] = json!("other"),
                    "wrong-provider" => value["provider"] = json!("codex"),
                    "wrong-schema" => value["schema_version"] = json!("unsupported"),
                    "missing-scope" => {
                        value.as_object_mut().unwrap().remove("history_scope");
                    }
                    "invalid-scope" => value["history_scope"] = json!("unknown"),
                    "unsafe-root" => value["config_dir"] = json!("relative/root"),
                    _ => unreachable!(),
                }
                fs::write(&response, value.to_string()).unwrap();
                if matches!(other, "wrong-account" | "unsafe-root") {
                    "resume-account-mismatch"
                } else {
                    "resume-unsupported"
                }
            }
        };
        let state_before = snapshot(&f.state);
        let account_before = snapshot(&f.root);
        for check in [true, false] {
            let mut args = vec!["--id", "blocked", "--account", account];
            if case == "omitted" {
                args.truncate(2);
            }
            if check {
                args.push("--check-resume");
            }
            let out = f.run(&args);
            assert_eq!(
                out.stdout_json()["error"]["code"],
                code,
                "{case}: {}",
                out.stdout_text()
            );
            assert_eq!(
                out.stdout_json()["error"]["details"],
                json!({"session_created":false,"safe_to_retry":false})
            );
            assert_eq!(snapshot(&f.state), state_before, "{case}");
            assert_eq!(snapshot(&f.root), account_before, "{case}");
        }
        let calls = fs::read_to_string(f.tmp.path().join("broker-calls")).unwrap_or_default();
        assert!(!calls.contains("materialize"), "{case}");
    }
}

#[test]
fn deleted_source_rechecks_history_account_and_new_sources_after_materialize() {
    for case in [
        "missing",
        "duplicate",
        "account-root",
        "live",
        "unknown",
        "busy",
        "account-mismatch",
    ] {
        let f = deleted_source();
        assert_eq!(f.run(&["--account", "original", "--check-resume"]).code, 0);
        let history = f
            .root
            .join(format!("projects/project/{CONVERSATION}.jsonl"));
        let quote = |path: &Path| shell_words::quote(path.to_str().unwrap()).into_owned();
        let hook = match case {
            "missing" => format!("rm -- {}\n", quote(&history)),
            "duplicate" => format!(
                "mkdir -p {0}; cp {1} {0}/{CONVERSATION}.jsonl\n",
                quote(&f.root.join("projects/duplicate")),
                quote(&history)
            ),
            "account-root" => {
                let alternate = f.tmp.path().join("alternate");
                fs::create_dir(&alternate).unwrap();
                fs::write(alternate.join(".credentials.json"), "{}").unwrap();
                fs::create_dir_all(alternate.join("projects/project")).unwrap();
                fs::copy(
                    &history,
                    alternate.join(format!("projects/project/{CONVERSATION}.jsonl")),
                )
                .unwrap();
                let value = json!({"schema_version":"agent-session.account-broker.v2","provider":"claude","account":"original","config_dir":alternate,"history_scope":"account"});
                fs::write(f.tmp.path().join("alternate-response"), value.to_string()).unwrap();
                format!(
                    "cp {} {}\n",
                    quote(&f.tmp.path().join("alternate-response")),
                    quote(&f.tmp.path().join("history-root.json"))
                )
            }
            _ => {
                // The source appears after the first guarded admission, during materialization.
                f.write_source(
                    "late-source",
                    if case == "account-mismatch" {
                        "other"
                    } else {
                        "original"
                    },
                );
                fs::rename(
                    f.state.join("sessions/late-source"),
                    f.tmp.path().join("late-source"),
                )
                .unwrap();
                format!(
                    "mv {} {}\n",
                    quote(&f.tmp.path().join("late-source")),
                    quote(&f.state.join("sessions/late-source"))
                )
            }
        };
        let busy = if case == "busy" {
            use std::os::fd::AsRawFd;
            let lock = fs::File::open(f.state.join("session-locks/late-source.lock")).unwrap();
            assert_eq!(
                unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
                0
            );
            Some(lock)
        } else {
            None
        };
        if case == "live" {
            executable(&f.tmux, "#!/bin/sh\nexit 0\n");
        }
        if case == "unknown" {
            executable(&f.tmux, "#!/bin/sh\nexit 2\n");
        }
        fs::write(f.tmp.path().join("materialize-hook"), hook).unwrap();
        let out = f.run(&["--account", "original", "--id", "blocked"]);
        let code = match case {
            "missing" => "resume-history-missing",
            "account-root" => "resume-account-mismatch",
            "duplicate" | "unknown" => "resume-source-unknown",
            "account-mismatch" => "resume-account-mismatch",
            _ => "resume-source-live",
        };
        assert_eq!(
            out.stdout_json()["error"]["code"],
            code,
            "{case}: {}",
            out.stdout_text()
        );
        assert_eq!(
            out.stdout_json()["error"]["details"],
            json!({"session_created":false,"safe_to_retry":false})
        );
        assert!(!f.record_path("blocked").exists(), "{case}");
        assert!(
            !f.state.join("sessions/blocked").exists(),
            "{case}: no partial prompt/operation"
        );
        assert!(!f.tmp.path().join("tmux.log").exists(), "{case}: no launch");
        drop(busy);
    }
}

fn pending_source(f: &Fixture) -> String {
    let mut source: Value =
        serde_json::from_slice(&fs::read(f.record_path("source")).unwrap()).unwrap();
    source["claude_account_next"] = json!({
        "schema_version":"agent-session.claude-account-next.v1",
        "account":"other", "revision":2, "state":"queued",
        "updated_at":"2026-01-01T00:00:00Z"
    });
    source.to_string()
}

#[test]
fn composition_late_pending_switch_does_not_turn_resolved_account_into_consent() {
    let f = Fixture::new();
    assert_eq!(f.run(&["--check-resume"]).code, 0);
    let pending = pending_source(&f);
    let prepared = f.tmp.path().join("pending-source.json");
    fs::write(&prepared, &pending).unwrap();
    let script = fs::read_to_string(&f.broker).unwrap().replace(
        "materialize)",
        &format!(
            "materialize) cp {} {};",
            shell_words::quote(prepared.to_str().unwrap()),
            shell_words::quote(f.record_path("source").to_str().unwrap())
        ),
    );
    executable(&f.broker, &script);
    let out = f.run(&["--id", "must-not-cancel"]);
    assert_eq!(
        out.stdout_json()["error"]["code"],
        "resume-account-transition-pending",
        "{}",
        out.stdout_text()
    );
    assert_eq!(
        out.stdout_json()["error"]["details"],
        json!({"session_created":false,"safe_to_retry":false})
    );
    assert_eq!(
        fs::read_to_string(f.record_path("source")).unwrap(),
        pending
    );
    assert!(!f.state.join("sessions/must-not-cancel").exists());
    assert_eq!(fs::read_dir(f.state.join("sessions")).unwrap().count(), 1);
}

#[test]
fn composition_history_race_does_not_settle_an_explicit_pending_switch() {
    let f = Fixture::new();
    let pending = pending_source(&f);
    fs::write(f.record_path("source"), &pending).unwrap();
    let history = f
        .root
        .join(format!("projects/project/{CONVERSATION}.jsonl"));
    let script = fs::read_to_string(&f.broker).unwrap().replace(
        "materialize)",
        &format!(
            "materialize) rm -- {};",
            shell_words::quote(history.to_str().unwrap())
        ),
    );
    executable(&f.broker, &script);
    let out = f.run(&["--account", "original", "--id", "must-not-settle"]);
    assert_eq!(
        out.stdout_json()["error"]["code"],
        "resume-history-missing",
        "{}",
        out.stdout_text()
    );
    assert_eq!(
        out.stdout_json()["error"]["details"],
        json!({"session_created":false,"safe_to_retry":false})
    );
    assert_eq!(
        fs::read_to_string(f.record_path("source")).unwrap(),
        pending
    );
    assert!(!f.state.join("sessions/must-not-settle").exists());
    assert_eq!(fs::read_dir(f.state.join("sessions")).unwrap().count(), 1);
}
