use super::*;
use pretty_assertions::assert_eq;

#[tokio::test]
async fn list_runtime_exit_recovery_tracks_daemon_resume_readiness() {
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
    record.as_object_mut().unwrap().remove("startup");
    record
        .as_object_mut()
        .unwrap()
        .remove("tmux_runtime_never_launched");
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

    let target = crate::load_session_record(&st.context, "profile-root-removed").unwrap();
    crate::lifecycle::record(
        &st.context,
        Some(&target),
        &target.id,
        "exit-observed",
        "controller",
        Ok(()),
        Some(json!({"code": 130, "signal": null, "reason": "runtime-exited", "stopped_by": null})),
    );

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

    let ready = &ready_body["data"]["sessions"][0];
    assert_eq!(ready["status"], "stopped");
    assert!(ready.get("resume_blocked_reason").is_none());
    assert_eq!(ready["runtime_exit"]["unexpected"], true);
    assert_eq!(ready["runtime_exit"]["recovery"]["label"], "Resume");
    assert_eq!(ready["runtime_exit"]["recovery"]["method"], "POST");
    assert_eq!(
        ready["runtime_exit"]["recovery"]["path"],
        "/sessions/profile-root-removed/resume"
    );

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
    let blocked = &removed_body["data"]["sessions"][0];
    assert_eq!(blocked["runtime_exit"]["reason"], "provider-exited");
    assert_eq!(blocked["runtime_exit"]["code"], 130);
    assert_eq!(
        blocked["runtime_exit"]["message"],
        "Provider exited (status 130)"
    );
    assert_eq!(blocked["runtime_exit"]["unexpected"], true);
    assert!(blocked["runtime_exit"].get("recovery").is_none());
    let mut blocked_exit = blocked["runtime_exit"].clone();
    let mut ready_exit = ready["runtime_exit"].clone();
    blocked_exit.as_object_mut().unwrap().remove("recovery");
    ready_exit.as_object_mut().unwrap().remove("recovery");
    assert_eq!(blocked_exit, ready_exit);
    let calls = fs::read_to_string(log).unwrap_or_default();
    assert!(
        !calls.contains("new-session"),
        "list must not launch a provider"
    );
}
