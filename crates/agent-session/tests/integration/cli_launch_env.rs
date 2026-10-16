use std::fs;
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

use pretty_assertions::assert_eq;
use serde_json::{Value, json};

use crate::cli::{fake_agent, fake_tmux, run, sha256_hex, spawn_test_process_group, tmux_calls};

#[test]
fn launch_env_cli_persists_projects_and_reapplies_on_resume() {
    for agent in ["codex", "claude"] {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = tmp.path().join("state");
        let (tmux, log) = fake_tmux(tmp.path());
        let provider = fake_agent(tmp.path(), agent);
        let pane = spawn_test_process_group();
        let pane_pid = pane.pid().to_string();
        let values =
            json!({"AGENT_RUNTIME_SUPPRESS_MEMORY":"1", "AGENT_RUNTIME_SUPPRESS_HEALTH":"0"});
        let envs = [
            ("AGENT_SESSION_FAKE_TMUX_LOG", log.to_str().unwrap()),
            ("AGENT_SESSION_CODEX_RUNTIME", "off"),
            ("AGENT_SESSION_TMUX_BIN", tmux.to_str().unwrap()),
            (
                "AGENT_SESSION_LAUNCH_ENV_ALLOWLIST",
                r#"["AGENT_RUNTIME_SUPPRESS_MEMORY","AGENT_RUNTIME_SUPPRESS_HEALTH"]"#,
            ),
        ];
        let start_envs = [
            envs.as_slice(),
            &[
                ("AGENT_SESSION_FAKE_TMUX_PANE_PID", pane_pid.as_str()),
                (
                    "AGENT_SESSION_FAKE_TMUX_PROCESS_GROUP_ID",
                    pane_pid.as_str(),
                ),
                ("AGENT_SESSION_FAKE_TMUX_SESSION_ID", "$77"),
                ("AGENT_SESSION_FAKE_TMUX_AGENT_SESSION_ID", "launch-env"),
                ("AGENT_SESSION_FAKE_TMUX_STATE_DIR", state.to_str().unwrap()),
            ],
        ]
        .concat();
        let start = run(
            tmp.path(),
            &[
                "--state-dir",
                state.to_str().unwrap(),
                "start",
                "--agent",
                agent,
                "--cwd",
                tmp.path().to_str().unwrap(),
                "--id",
                "launch-env",
                "--tmux-bin",
                tmux.to_str().unwrap(),
                "--agent-bin",
                provider.to_str().unwrap(),
                "--env",
                "AGENT_RUNTIME_SUPPRESS_MEMORY=1",
                "--env",
                "AGENT_RUNTIME_SUPPRESS_HEALTH=0",
                "--format",
                "json",
            ],
            &start_envs,
        );
        assert_eq!(
            start.code,
            0,
            "{} {}",
            start.stdout_text(),
            start.stderr_text()
        );
        assert_eq!(start.stdout_json()["data"]["launch_env"], values);
        let path = state.join("sessions/launch-env/session.json");
        let mut record: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(record["launch_env"], values);
        let list = run(
            tmp.path(),
            &[
                "--state-dir",
                state.to_str().unwrap(),
                "list",
                "--format",
                "json",
            ],
            &envs,
        );
        assert_eq!(list.code, 0, "{}", list.stdout_text());
        assert_eq!(list.stdout_json()["data"][0]["launch_env"], values);
        let origin = json!({"parent":{"session_id":"launch-parent","machine":"worker-host"}});
        record["origin"] = origin.clone();
        record["workspace"] = json!("example-workspace");
        fs::write(&path, serde_json::to_vec_pretty(&record).unwrap()).unwrap();
        // Pin a synthetic provider identity; provider history is outside this contract.
        if agent == "codex" {
            record["provider_resume"] = json!({"provider":"codex", "session_id":"fixture-conversation", "captured_at":"2030-01-01T00:00:00Z", "capture_method":"fixture", "resume_args":["resume","fixture-conversation","--cd",tmp.path(),"--no-alt-screen"]});
            fs::write(&path, serde_json::to_vec_pretty(&record).unwrap()).unwrap();
        }
        // macOS also probes the internal session ID; mark that exact target stopped.
        let stopped = Command::new(&tmux)
            .args([
                "kill-session",
                "-t",
                record["delete_tmux_identity"]["session_id"]
                    .as_str()
                    .unwrap(),
            ])
            .envs(start_envs.iter().copied())
            .env(
                "AGENT_SESSION_FAKE_TMUX_RUNTIME_ID",
                record["runtime"]["launch_id"].as_str().unwrap(),
            )
            .output()
            .unwrap();
        assert!(stopped.status.success());
        let deadline = Instant::now() + Duration::from_secs(5);
        while pane.is_running() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(5));
        }
        assert!(!pane.is_running(), "fake tmux must stop the original pane");
        // The fake pane exits without the owned launch wrapper, so emulate the
        // wrapper's authenticated broker stop; macOS resume requires it.
        let capability = state.join(format!(
            "sessions/launch-env/coordination/capability-{}",
            sha256_hex(record["runtime"]["launch_id"].as_str().unwrap())
        ));
        let broker_stopped = run(
            tmp.path(),
            &[
                "--state-dir",
                state.to_str().unwrap(),
                "broker",
                "stop",
                "--session",
                "launch-env",
                "--exit-code",
                "0",
                "--capability-file",
                capability.to_str().unwrap(),
                "--format",
                "json",
            ],
            &[],
        );
        assert_eq!(broker_stopped.code, 0, "{}", broker_stopped.stderr_text());
        // Resume needs a new live identity after the original group is reaped.
        let resumed_pane = spawn_test_process_group();
        let resumed_pane_pid = resumed_pane.pid().to_string();
        let resume_envs = [
            envs.as_slice(),
            &[
                (
                    "AGENT_SESSION_FAKE_TMUX_PANE_PID",
                    resumed_pane_pid.as_str(),
                ),
                (
                    "AGENT_SESSION_FAKE_TMUX_PROCESS_GROUP_ID",
                    resumed_pane_pid.as_str(),
                ),
                ("AGENT_SESSION_FAKE_TMUX_SESSION_ID", "$78"),
                ("AGENT_SESSION_FAKE_TMUX_AGENT_SESSION_ID", "launch-env"),
                ("AGENT_SESSION_FAKE_TMUX_STATE_DIR", state.to_str().unwrap()),
            ],
        ]
        .concat();
        let resume = run(
            tmp.path(),
            &[
                "--state-dir",
                state.to_str().unwrap(),
                "resume",
                "launch-env",
                "--tmux-bin",
                tmux.to_str().unwrap(),
                "--format",
                "json",
            ],
            &resume_envs,
        );
        assert_eq!(
            resume.code,
            0,
            "{} {}",
            resume.stdout_text(),
            resume.stderr_text()
        );
        assert_eq!(resume.stdout_json()["data"]["launch_env"], values);
        assert_eq!(resume.stdout_json()["data"]["origin"], origin);
        assert_eq!(
            resume.stdout_json()["data"]["workspace"],
            "example-workspace"
        );
        let resumed: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(resumed["origin"], origin);
        assert_eq!(resumed["workspace"], "example-workspace");
        let launches: Vec<_> = tmux_calls(&log)
            .into_iter()
            .filter(|args| args.first().is_some_and(|arg| arg == "new-session"))
            .collect();
        assert_eq!(launches.len(), 2);
        for launch in launches {
            assert!(
                launch
                    .windows(2)
                    .any(|args| args == ["-e", "AGENT_RUNTIME_SUPPRESS_MEMORY=1"])
            );
            assert!(
                launch
                    .windows(2)
                    .any(|args| args == ["-e", "AGENT_RUNTIME_SUPPRESS_HEALTH=0"])
            );
        }
    }
}
