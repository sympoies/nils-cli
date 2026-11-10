//! Modeled launch proves the notice encoder reaches the existing initial prompt staging path.
use nils_test_support::cmd::run_resolved;
use pretty_assertions::assert_eq;
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

struct Server(std::process::Child);
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn get(addr: std::net::SocketAddr, path: &str) -> serde_json::Value {
    let mut stream = TcpStream::connect(addr).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    write!(stream, "GET {path} HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer synthetic-token\r\nConnection: close\r\n\r\n").unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap()
}

#[test]
fn claimed_launch_paste_projects_as_notice_but_unclaimed_paste_remains_user() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let state = root.join("state");
    let config = root.join("provider-config");
    let (tmux, log) = super::cli::fake_tmux(root);
    let agent = super::cli::fake_agent(root, "claude");
    let inner_tmux = root.join("tmux-inner");
    fs::rename(&tmux, &inner_tmux).unwrap();
    fs::write(
        &tmux,
        r##"#!/usr/bin/env python3
import json, os, pathlib, subprocess, sys
result = subprocess.run([os.environ['FIXTURE_TMUX_INNER'], *sys.argv[1:]])
if result.returncode == 0 and sys.argv[1] == 'paste-buffer':
    state = pathlib.Path(os.environ['FIXTURE_STATE'])
    record = json.loads((state / 'sessions/synthetic/session.json').read_text())
    provider = record['provider_resume']['session_id']
    envelope = (state / 'sessions/synthetic/prompt.md').read_text()
    wrapped = '\n\n<pasted_content id="abcd">\n' + envelope + '</pasted_content id="abcd">\n'
    row = {'type':'user','sessionId':provider,'uuid':'11111111-1111-4111-8111-111111111111',
        'cwd':record['cwd'],'version':'2.1.317','isSidechain':False,
        'timestamp':'2026-01-01T00:00:00Z','message':{'role':'user','content':wrapped}}
    project = pathlib.Path(os.environ['CLAUDE_CONFIG_DIR']) / 'projects/synthetic'
    project.mkdir(parents=True, exist_ok=True)
    (project / (provider + '.jsonl')).write_text(json.dumps(row) + '\n')
sys.exit(result.returncode)
"##,
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&tmux, fs::Permissions::from_mode(0o755)).unwrap();
    let body = "Synthetic initial task\nSecond line\n";
    let envelope =
        agent_session::notice::NoticeEnvelope::new("dispatch.bootstrap", "scheduler", None)
            .unwrap()
            .encode(body)
            .unwrap();
    let output = run_resolved(
        "agent-session",
        &[
            "--state-dir",
            state.to_str().unwrap(),
            "--host",
            "example.invalid",
            "start",
            "--agent",
            "claude",
            "--cwd",
            root.to_str().unwrap(),
            "--id",
            "synthetic",
            "--prompt-stdin",
            "--notice-kind",
            "dispatch.bootstrap",
            "--notice-source",
            "scheduler",
            "--tmux-bin",
            tmux.to_str().unwrap(),
            "--agent-bin",
            agent.to_str().unwrap(),
            "--paste-delay-ms",
            "0",
            "--format",
            "json",
        ],
        &super::hermetic_provider_options(root)
            .with_env_remove_prefix("AGENT_SESSION_")
            .with_env("AGENT_SESSION_NOTICE_WRITERS", "1")
            .with_env("AGENT_SESSION_FAKE_TMUX_LOG", log.to_str().unwrap())
            .with_env("CLAUDE_CONFIG_DIR", config.to_str().unwrap())
            .with_env("FIXTURE_STATE", state.to_str().unwrap())
            .with_env("FIXTURE_TMUX_INNER", inner_tmux.to_str().unwrap())
            .with_stdin_str(body),
    );
    assert_eq!(output.code, 0, "{}", output.stderr_text());
    let record: serde_json::Value =
        serde_json::from_slice(&fs::read(state.join("sessions/synthetic/session.json")).unwrap())
            .unwrap();
    let provider = record["provider_resume"]["session_id"].as_str().unwrap();
    let wrapped =
        format!("\n\n<pasted_content id=\"abcd\">\n{envelope}</pasted_content id=\"abcd\">\n");
    let row = |uuid: &str| {
        serde_json::json!({"type":"user", "sessionId":provider,
        "uuid":uuid, "cwd":root, "version":"2.1.317", "isSidechain":false,
        "timestamp":"2026-01-01T00:00:00Z", "message":{"role":"user","content":wrapped}})
    };
    let project = config.join("projects/synthetic");
    fs::create_dir_all(&project).unwrap();
    let mut transcript = fs::OpenOptions::new()
        .append(true)
        .open(project.join(format!("{provider}.jsonl")))
        .unwrap();
    let mut unclaimed_in_session = row("22222222-2222-4222-8222-222222222222");
    unclaimed_in_session["message"]["content"] =
        serde_json::json!(wrapped.replace("Second line", "Human line"));
    writeln!(transcript, "{unclaimed_in_session}").unwrap();
    writeln!(
        transcript,
        "{}",
        row("66666666-6666-4666-8666-666666666666")
    )
    .unwrap();
    let mut human = row("55555555-5555-4555-8555-555555555555");
    human["message"]["content"] = serde_json::json!("actual human request");
    writeln!(transcript, "{human}").unwrap();
    let unclaimed_id = "33333333-3333-4333-8333-333333333333";
    let mut unclaimed = row("44444444-4444-4444-8444-444444444444");
    unclaimed["sessionId"] = serde_json::json!(unclaimed_id);
    fs::write(
        project.join(format!("{unclaimed_id}.jsonl")),
        format!("{unclaimed}\n"),
    )
    .unwrap();
    let addr = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap();
    let mut command = Command::new(nils_test_support::bin::resolve("agent-session"));
    command
        .args([
            "serve",
            "--bind",
            &addr.to_string(),
            "--state-dir",
            state.to_str().unwrap(),
            "--token",
            "synthetic-token",
        ])
        .env("CLAUDE_CONFIG_DIR", &config)
        .env("HOME", root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    nils_test_support::cmd::strip_ambient_managed_session_env(&mut command);
    let _server = Server(command.spawn().unwrap());
    let deadline = Instant::now() + Duration::from_secs(15);
    while TcpStream::connect(addr).is_err() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(20));
    }
    let sessions = get(addr, "/history/sessions?provider=claude");
    let list = sessions["data"]["sessions"].as_array().unwrap();
    let id = list
        .iter()
        .find(|s| s["provider_session_id"] == provider)
        .unwrap()["id"]
        .as_str()
        .unwrap();
    let unclaimed_history = list
        .iter()
        .find(|s| s["provider_session_id"] == unclaimed_id)
        .unwrap()["id"]
        .as_str()
        .unwrap();
    let claimed = list
        .iter()
        .find(|s| s["provider_session_id"] == provider)
        .unwrap();
    for field in ["first_user_prompt_preview", "prompt_preview"] {
        assert!(
            claimed[field].as_str().unwrap().contains("Human line"),
            "{field}"
        );
    }
    assert_eq!(claimed["last_user_prompt_preview"], "actual human request");
    let unclaimed_session = list
        .iter()
        .find(|s| s["provider_session_id"] == unclaimed_id)
        .unwrap();
    assert!(
        unclaimed_session["first_user_prompt_preview"]
            .as_str()
            .unwrap()
            .contains("pasted_content")
    );
    let page = get(
        addr,
        &format!("/history/sessions/{id}/messages?direction=forward"),
    );
    assert_eq!(page["data"]["messages"][0]["role"], "notice");
    assert_eq!(page["data"]["messages"][1]["role"], "user");
    assert_eq!(page["data"]["messages"][2]["role"], "user");
    assert_eq!(page["data"]["messages"][2]["text"], wrapped);
    assert_eq!(page["data"]["messages"][3]["role"], "user");
    let latest = get(
        addr,
        &format!("/history/sessions/{id}/messages?direction=latest"),
    );
    assert_eq!(latest["data"]["messages"][0]["role"], "notice");
    assert_eq!(latest["data"]["messages"][1]["role"], "user");
    assert_eq!(latest["data"]["messages"][2]["role"], "user");
    assert_eq!(latest["data"]["messages"][3]["role"], "user");
    let latest = get(
        addr,
        &format!("/history/sessions/{id}/messages?direction=latest&limit=1"),
    );
    assert_eq!(latest["data"]["messages"][0]["role"], "user");
    let cursor = latest["data"]["older_cursor"].as_str().unwrap();
    let older = get(
        addr,
        &format!("/history/sessions/{id}/messages?direction=older&limit=1&cursor={cursor}"),
    );
    assert_eq!(older["data"]["messages"][0]["role"], "user");
    let cursor = older["data"]["older_cursor"].as_str().unwrap();
    let older = get(
        addr,
        &format!("/history/sessions/{id}/messages?direction=older&limit=1&cursor={cursor}"),
    );
    assert_eq!(older["data"]["messages"][0]["role"], "user");
    let cursor = older["data"]["older_cursor"].as_str().unwrap();
    let older = get(
        addr,
        &format!("/history/sessions/{id}/messages?direction=older&limit=1&cursor={cursor}"),
    );
    assert_eq!(older["data"]["messages"][0]["role"], "notice");
    let page = get(
        addr,
        &format!("/history/sessions/{unclaimed_history}/messages?direction=forward"),
    );
    assert_eq!(page["data"]["messages"][0]["role"], "user");
    let receipt_path = state.join("sessions/synthetic/launch-notice.json");
    let original_receipt = fs::read(&receipt_path).unwrap();
    let mut receipt: serde_json::Value = serde_json::from_slice(&original_receipt).unwrap();
    receipt["source"] = serde_json::json!("other");
    fs::write(&receipt_path, serde_json::to_vec(&receipt).unwrap()).unwrap();
    let page = get(
        addr,
        &format!("/history/sessions/{id}/messages?direction=forward"),
    );
    assert_eq!(page["data"]["messages"][0]["role"], "user");
    fs::write(&receipt_path, original_receipt).unwrap();
    let mut record: serde_json::Value =
        serde_json::from_slice(&fs::read(state.join("sessions/synthetic/session.json")).unwrap())
            .unwrap();
    record["runtime"]["generation"] = serde_json::json!(2);
    fs::write(
        state.join("sessions/synthetic/session.json"),
        serde_json::to_vec(&record).unwrap(),
    )
    .unwrap();
    let page = get(
        addr,
        &format!("/history/sessions/{id}/messages?direction=forward"),
    );
    assert_eq!(page["data"]["messages"][0]["role"], "user");
    record["runtime"]["generation"] = serde_json::json!(1);
    fs::write(
        state.join("sessions/synthetic/session.json"),
        serde_json::to_vec(&record).unwrap(),
    )
    .unwrap();
    let mut changed = row("11111111-1111-4111-8111-111111111111");
    changed["message"]["content"] = serde_json::json!(wrapped.replace("Second line", "Other line"));
    fs::write(
        project.join(format!("{provider}.jsonl")),
        format!("{changed}\n"),
    )
    .unwrap();
    let page = get(
        addr,
        &format!("/history/sessions/{id}/messages?direction=forward"),
    );
    assert_eq!(page["data"]["messages"][0]["role"], "user");
}

#[test]
fn notice_start_stages_exact_encoded_stdin_for_both_providers() {
    let body = "  Synthetic initial task\r\n\nUnicode 本文\n";
    let expected =
        agent_session::notice::NoticeEnvelope::new("dispatch.bootstrap", "scheduler", None)
            .unwrap()
            .encode(body)
            .unwrap();
    for provider in ["claude", "codex"] {
        let temp = tempfile::tempdir().unwrap();
        let state = temp.path().join("state");
        let (tmux, log) = super::cli::fake_tmux(temp.path());
        let agent = super::cli::fake_agent(temp.path(), provider);
        let output = run_resolved(
            "agent-session",
            &[
                "--state-dir",
                state.to_str().unwrap(),
                "--host",
                "example.invalid",
                "start",
                "--agent",
                provider,
                "--cwd",
                temp.path().to_str().unwrap(),
                "--id",
                "synthetic",
                "--prompt-stdin",
                "--notice-kind",
                "dispatch.bootstrap",
                "--notice-source",
                "scheduler",
                "--tmux-bin",
                tmux.to_str().unwrap(),
                "--agent-bin",
                agent.to_str().unwrap(),
                "--paste-delay-ms",
                "0",
                "--format",
                "json",
            ],
            &super::hermetic_provider_options(temp.path())
                .with_env_remove_prefix("AGENT_SESSION_")
                .with_env("AGENT_SESSION_NOTICE_WRITERS", "1")
                .with_env("AGENT_SESSION_FAKE_TMUX_LOG", log.to_str().unwrap())
                .with_stdin_str(body),
        );
        assert_eq!(
            output.code,
            0,
            "{} {}",
            output.stdout_text(),
            output.stderr_text()
        );
        assert_eq!(
            fs::read(state.join("sessions/synthetic/prompt.md")).unwrap(),
            expected.as_bytes()
        );
        assert!(!output.stdout_text().contains(body));
        assert!(
            super::cli::tmux_calls(&log)
                .iter()
                .any(|call| call[0] == "load-buffer")
        );
    }
}

#[test]
fn session_list_skips_only_receipt_bound_launch_row() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let state = root.join("state");
    let config = root.join("provider-config");
    let (tmux, log) = super::cli::fake_tmux(root);
    let agent = super::cli::fake_agent(root, "claude");
    let inner_tmux = root.join("tmux-inner");
    fs::rename(&tmux, &inner_tmux).unwrap();
    fs::write(
        &tmux,
        r##"#!/usr/bin/env python3
import json, os, pathlib, subprocess, sys
result = subprocess.run([os.environ['FIXTURE_TMUX_INNER'], *sys.argv[1:]])
if result.returncode == 0 and sys.argv[1] == 'paste-buffer':
    state = pathlib.Path(os.environ['FIXTURE_STATE'])
    record = json.loads((state / 'sessions/synthetic/session.json').read_text())
    provider = record['provider_resume']['session_id']
    envelope = (state / 'sessions/synthetic/prompt.md').read_text()
    wrapped = '\n\n<pasted_content id="abcd">\n' + envelope + '</pasted_content id="abcd">\n'
    row = {'type':'user','sessionId':provider,'uuid':'11111111-1111-4111-8111-111111111111',
        'cwd':record['cwd'],'version':'2.1.317','isSidechain':False,
        'timestamp':'2026-01-01T00:00:00Z','message':{'role':'user','content':wrapped}}
    project = pathlib.Path(os.environ['CLAUDE_CONFIG_DIR']) / 'projects/synthetic'
    project.mkdir(parents=True, exist_ok=True)
    (project / (provider + '.jsonl')).write_text(json.dumps({'type':'mode','sessionId':provider,'cwd':record['cwd'],'isSidechain':False}) + '\n')
    (state / 'launch-row.jsonl').write_text(json.dumps(row) + '\n')
sys.exit(result.returncode)
"##,
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&tmux, fs::Permissions::from_mode(0o755)).unwrap();
    let body = "Synthetic initial task\nSecond line\n";
    let envelope =
        agent_session::notice::NoticeEnvelope::new("dispatch.bootstrap", "scheduler", None)
            .unwrap()
            .encode(body)
            .unwrap();
    let output = run_resolved(
        "agent-session",
        &[
            "--state-dir",
            state.to_str().unwrap(),
            "--host",
            "example.invalid",
            "start",
            "--agent",
            "claude",
            "--cwd",
            root.to_str().unwrap(),
            "--id",
            "synthetic",
            "--prompt-stdin",
            "--notice-kind",
            "dispatch.bootstrap",
            "--notice-source",
            "scheduler",
            "--tmux-bin",
            tmux.to_str().unwrap(),
            "--agent-bin",
            agent.to_str().unwrap(),
            "--paste-delay-ms",
            "0",
            "--format",
            "json",
        ],
        &super::hermetic_provider_options(root)
            .with_env_remove_prefix("AGENT_SESSION_")
            .with_env("AGENT_SESSION_NOTICE_WRITERS", "1")
            .with_env("AGENT_SESSION_FAKE_TMUX_LOG", log.to_str().unwrap())
            .with_env("CLAUDE_CONFIG_DIR", config.to_str().unwrap())
            .with_env("FIXTURE_STATE", state.to_str().unwrap())
            .with_env("FIXTURE_TMUX_INNER", inner_tmux.to_str().unwrap())
            .with_stdin_str(body),
    );
    assert_eq!(output.code, 0, "{}", output.stderr_text());
    let record: serde_json::Value =
        serde_json::from_slice(&fs::read(state.join("sessions/synthetic/session.json")).unwrap())
            .unwrap();
    let provider = record["provider_resume"]["session_id"].as_str().unwrap();
    let wrapped =
        format!("\n\n<pasted_content id=\"abcd\">\n{envelope}</pasted_content id=\"abcd\">\n");
    let row = |uuid: &str| {
        serde_json::json!({"type":"user", "sessionId":provider,
        "uuid":uuid, "cwd":root, "version":"2.1.317", "isSidechain":false,
        "timestamp":"2026-01-01T00:00:00Z", "message":{"role":"user","content":wrapped}})
    };
    let project = config.join("projects/synthetic");
    fs::create_dir_all(&project).unwrap();
    let addr = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap();
    let mut command = Command::new(nils_test_support::bin::resolve("agent-session"));
    command
        .args([
            "serve",
            "--bind",
            &addr.to_string(),
            "--state-dir",
            state.to_str().unwrap(),
            "--token",
            "synthetic-token",
            "--tmux-bin",
            tmux.to_str().unwrap(),
        ])
        .env("CLAUDE_CONFIG_DIR", &config)
        .env("HOME", root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    nils_test_support::cmd::strip_ambient_managed_session_env(&mut command);
    let _server = Server(command.spawn().unwrap());
    let deadline = Instant::now() + Duration::from_secs(15);
    while TcpStream::connect(addr).is_err() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(20));
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let sessions = get(addr, "/sessions");
        let session = &sessions["data"]["sessions"][0];
        if session["last_prompt_state"] == "current" {
            assert!(
                session.get("last_prompt").is_none(),
                "launch row leaked: {session}"
            );
            break;
        }
        assert!(
            Instant::now() < deadline,
            "projection did not become current: {session}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    let mut launch_transcript = fs::OpenOptions::new()
        .append(true)
        .open(project.join(format!("{provider}.jsonl")))
        .unwrap();
    launch_transcript
        .write_all(&fs::read(state.join("launch-row.jsonl")).unwrap())
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let sessions = get(addr, "/sessions");
        let session = &sessions["data"]["sessions"][0];
        if session["last_prompt_state"] == "current" {
            assert!(
                session.get("last_prompt").is_none(),
                "late launch row leaked: {session}"
            );
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(20));
    }
    let mut transcript = fs::OpenOptions::new()
        .append(true)
        .open(project.join(format!("{provider}.jsonl")))
        .unwrap();
    let mut unclaimed_in_session = row("22222222-2222-4222-8222-222222222222");
    unclaimed_in_session["message"]["content"] =
        serde_json::json!(wrapped.replace("Second line", "Human line"));
    writeln!(transcript, "{unclaimed_in_session}").unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let sessions = get(addr, "/sessions");
        let session = &sessions["data"]["sessions"][0];
        if session["last_prompt"]["text"]
            .as_str()
            .is_some_and(|text| text.contains("Human line"))
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "unclaimed prompt not projected: {session}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}
