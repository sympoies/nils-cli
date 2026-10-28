#[cfg(test)]
mod refresh_tests {
    use super::*;

    fn seed_claude(state_dir: &Path, provider_root: &Path) {
        let mut record: SessionRecord = serde_json::from_value(json!({
            "schema_version": "agent-session.session.v1",
            "id": "refresh-route",
            "agent": "claude",
            "mode": "interactive",
            "title": null,
            "cwd": "/workspace/project",
            "tmux_session": "refresh-route",
            "prompt_file": null,
            "log_file": null,
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z",
            "agent_args": ["--model", "old-model"],
            "provider_resume": {
                "provider": "claude", "session_id": "provider-session",
                "captured_at": "2026-01-01T00:00:00Z", "capture_method": "fixture",
                "resume_args": []
            },
            "runtime": {
                "kind": "tmux", "tmux_session": "refresh-route", "generation": 1,
                "started_at": "2026-01-01T00:00:00Z", "launch_id": "refresh-runtime",
                "agent_profile_provider_config_dir": provider_root,
            },
        }))
        .unwrap();
        crate::session_model::ModelSettings::store_launch(&mut record);
        let context = CliContext {
            state_dir: state_dir.to_path_buf(),
            host: None,
        };
        crate::write_session_record(&context, &record).unwrap();
        let projects = provider_root.join("projects/project");
        fs::create_dir_all(&projects).unwrap();
        let row = json!({
            "type": "assistant", "sessionId": "provider-session", "cwd": "/workspace/project",
            "timestamp": "2026-01-01T00:00:00Z", "message": {"model": "claude-opus-5-5"},
        });
        fs::write(projects.join("provider-session.jsonl"), format!("{row}\n")).unwrap();
    }

    #[tokio::test]
    async fn session_refresh_requires_the_token_and_returns_the_changes() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let state_dir = tmp.path().join("state");
        seed_claude(&state_dir, &tmp.path().join("provider"));
        let app = router(state(&state_dir, Some(TOKEN), minimal_tmux(tmp.path())));

        let (status, _) = call(
            app.clone(),
            post_json("/sessions/refresh-route/refresh", None, json!({})),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);

        let (status, body) = call(
            app,
            post_json("/sessions/refresh-route/refresh", Some(TOKEN), json!({})),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["data"]["machine"], MACHINE);
        assert_eq!(body["data"]["refresh"]["changed"], json!(["model"]));
        assert_eq!(body["data"]["refresh"]["session"]["model"], "claude-opus-5-5");
        let context = CliContext {
            state_dir,
            host: None,
        };
        let stored = load_session_record(&context, "refresh-route").unwrap();
        assert_eq!(
            crate::session_model::ModelSettings::for_record(&stored).model.as_deref(),
            Some("claude-opus-5-5")
        );
    }
}
