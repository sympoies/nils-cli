use super::*;
use pretty_assertions::assert_eq;

#[test]
fn authenticated_forge_identity_projection_is_read_only_and_refuses_cross_session_or_stale_binding()
{
    let tmp = tempfile::TempDir::new().unwrap();
    let state_dir = tmp.path().join("state");
    fs::create_dir(&state_dir).unwrap();
    seed_brokers(
        &state_dir,
        &[
            (
                "alpha",
                "incarnation-alpha",
                "alpha-private-capability-material",
            ),
            (
                "beta",
                "incarnation-beta",
                "beta-private-capability-material",
            ),
        ],
    );
    let path = state_dir.join("sessions/alpha/session.json");
    let mut record: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    record["role"] = json!("reviewer");
    record["lineage"] = json!({
        "schema_version":"agent-session.session-lineage.v1", "parent":null,
        "root":{"machine":"launch-source", "session_id":"alpha", "session_created_at":record["created_at"]},
        "depth":0,"starter":{"kind":"operator", "via":"cli"},
        "forge_context":{"initiator":"operator", "role":"reviewer"}
    });
    write_private_json(&path, &record);
    let registry_path = state_dir.join("coordination/registry.json");
    let registry_before = fs::read(&registry_path).unwrap();
    let record_before = fs::read(&path).unwrap();
    let project = |cap: &str, runtime: &str| {
        run_with_env(
            tmp.path(),
            &[
                "--state-dir",
                state_dir.to_str().unwrap(),
                "broker",
                "identity",
                "--session",
                "alpha",
                "--capability-file",
                cap,
                "--format",
                "json",
            ],
            &[
                ("AGENT_SESSION_ID", "alpha"),
                ("AGENT_SESSION_RUNTIME_ID", runtime),
            ],
        )
    };
    let alpha_cap = capability(&state_dir, "alpha");
    let valid = project(&alpha_cap, "incarnation-alpha");
    assert_eq!(valid.code, 0, "{}", valid.stdout_text());
    assert_eq!(data(&valid)["initiator"], "operator");
    assert_eq!(data(&valid)["role"], "reviewer");
    assert_eq!(data(&valid)["session_incarnation"], "incarnation-alpha");
    assert_eq!(fs::read(&registry_path).unwrap(), registry_before);
    assert_eq!(fs::read(&path).unwrap(), record_before);
    let (tmux, tmux_log) = fake_tmux(tmp.path());
    let agent = fake_agent(tmp.path(), "codex");
    let child = run_with_env(
        tmp.path(),
        &[
            "--state-dir",
            state_dir.to_str().unwrap(),
            "start",
            "--agent",
            "codex",
            "--id",
            "bound-child",
            "--cwd",
            tmp.path().to_str().unwrap(),
            "--tmux-bin",
            tmux.to_str().unwrap(),
            "--agent-bin",
            agent.to_str().unwrap(),
            "--paste-delay-ms",
            "0",
            "--role",
            "tester",
            "--format",
            "json",
        ],
        &[
            ("AGENT_SESSION_ID", "alpha"),
            ("AGENT_SESSION_RUNTIME_ID", "incarnation-alpha"),
            ("AGENT_SESSION_CAPABILITY_FILE", &alpha_cap),
            ("AGENT_SESSION_MACHINE", "launch-source"),
            ("AGENT_SESSION_FAKE_TMUX_LOG", tmux_log.to_str().unwrap()),
        ],
    );
    assert_eq!(child.code, 0, "{}", child.stdout_text());
    let child_record: serde_json::Value = serde_json::from_slice(
        &fs::read(state_dir.join("sessions/bound-child/session.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        child_record["lineage"]["forge_context"]["initiator"],
        "operator"
    );
    assert_eq!(child_record["lineage"]["forge_context"]["role"], "tester");
    assert_eq!(child_record["lineage"]["parent"]["session_id"], "alpha");
    assert_eq!(child_record["role"], "tester");
    let crossed = project(&capability(&state_dir, "beta"), "incarnation-alpha");
    assert_eq!(
        crossed.stdout_json()["error"]["code"],
        "coordination-unauthorized"
    );
    let stale = project(&alpha_cap, "old-incarnation");
    assert_eq!(
        stale.stdout_json()["error"]["message"],
        "forge launch binding refused"
    );
    assert_eq!(
        stale.stdout_json()["error"]["code"],
        "identity_session_binding_mismatch"
    );
    record["lineage"]["forge_context"]["role"] = json!("tester");
    write_private_json(&path, &record);
    assert_eq!(
        project(&alpha_cap, "incarnation-alpha").stdout_json()["error"]["code"],
        "identity_session_binding_invalid"
    );
    assert_eq!(
        project(&alpha_cap, "incarnation-alpha").stdout_json()["error"]["message"],
        "forge launch binding refused"
    );
    record["lineage"]
        .as_object_mut()
        .unwrap()
        .remove("forge_context");
    write_private_json(&path, &record);
    let missing = project(&alpha_cap, "incarnation-alpha").stdout_json();
    assert_eq!(missing["error"]["code"], "identity_session_binding_missing");
    assert_eq!(
        missing["error"]["message"],
        "session has no forge launch binding"
    );
}
