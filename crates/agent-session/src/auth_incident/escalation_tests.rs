//! A session with no effective parent still gets an independent, deduplicated
//! alert through the daemon-independent observation spool.

use super::*;
use nils_common::observation::{self, Severity};
use pretty_assertions::assert_eq;
use serde_json::json;

fn record() -> SessionRecord {
    serde_json::from_value(json!({
        "schema_version":"agent-session.session.v1", "id":"root-fixture", "agent":"claude", "mode":"interactive", "title":null,
        "cwd":"/fixture/private-checkout", "tmux_session":"root-fixture", "prompt_file":null, "log_file":null,
        "created_at":"2030-01-01T00:00:00Z", "updated_at":"2030-01-01T00:00:00Z",
        "runtime":{"kind":"tmux", "tmux_session":"root-fixture", "generation":1, "started_at":"2030-01-01T00:00:00Z", "launch_id":"runtime-a"}
    }))
    .unwrap()
}

fn with_owner(mut record: SessionRecord) -> SessionRecord {
    record.lineage_adoption = Some(
        serde_json::from_value(json!({
            "adopted_by":{"machine":"fixture","session_id":"owner-fixture","session_created_at":"2030-01-01T00:00:00Z"},
            "revision":1, "updated_at":"2030-01-01T00:00:00Z"
        }))
        .unwrap(),
    );
    record
}

fn failed(record: &SessionRecord) -> (tempfile::TempDir, CliContext) {
    let tmp = tempfile::TempDir::new().unwrap();
    let context = CliContext {
        // The alert spool refuses a state root behind a symlink, and a platform
        // temp dir may itself be one, so root the state at its physical path.
        state_dir: fs::canonicalize(tmp.path()).unwrap().join("state"),
        host: None,
    };
    fs::create_dir_all(session_dir(&context, &record.id)).unwrap();
    crate::write_session_record(&context, record).unwrap();
    observe(
        &context,
        record,
        "event",
        AuthSource::ClaudeStopFailure,
        Confidence::Authoritative,
        "2030-01-01T00:00:00Z",
    )
    .unwrap();
    (tmp, context)
}

fn tick_once(context: &CliContext, record: &SessionRecord) {
    tick(context, record, Path::new("unused"), "fixture", Some(false)).unwrap();
}

fn incident(context: &CliContext, record: &SessionRecord) -> Value {
    serde_json::to_value(&read(context, record).unwrap().incidents[0]).unwrap()
}

fn events(context: &CliContext) -> Vec<observation::Event> {
    observation::read_recent(&context.state_dir, 0).unwrap()
}

#[test]
fn parentless_incident_records_one_deduplicated_alert_without_claiming_delivery() {
    let record = record();
    let (_tmp, context) = failed(&record);
    tick_once(&context, &record);
    tick_once(&context, &record);

    let incident = incident(&context, &record);
    let alerts = events(&context);
    assert_eq!(alerts.len(), 1);
    let alert = &alerts[0];
    assert_eq!(alert.stage, "auth-incident");
    assert_eq!(alert.code, "auth-owner-unavailable");
    assert_eq!(alert.severity, Severity::Error);
    assert_eq!(alert.product.as_deref(), Some("claude"));
    assert_eq!(
        alert.correlation,
        Some(format!(
            "sha256:{}",
            incident["incident_id"].as_str().unwrap()
        ))
    );
    assert_eq!(
        alert.recovery_action.as_deref(),
        Some(
            format!(
                "{} lineage adopt <CHILD> --by <SESSION>",
                crate::command_identity::executable()
            )
            .as_str()
        )
    );

    let receipt = &incident["notification"];
    assert_eq!(receipt["delivered_at"], Value::Null);
    assert_eq!(receipt["message_id"], Value::Null);
    assert_eq!(receipt["degraded_reason"], "owner_unavailable");
    assert_eq!(receipt["escalation"]["route"], "observation_spool");
    assert_eq!(receipt["escalation"]["state"], "recorded");

    let spool: String = fs::read_dir(observation::spool_root(&context.state_dir))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "jsonl"))
        .map(|path| fs::read_to_string(path).unwrap())
        .collect();
    assert!(spool.contains("auth-owner-unavailable"));
    for private in ["root-fixture", "private-checkout", "runtime-a"] {
        assert!(!spool.contains(private), "spool leaked {private}");
    }
}

#[test]
fn unavailable_alert_route_is_explicit_and_retries_after_it_recovers() {
    let record = record();
    let (_tmp, context) = failed(&record);
    let blocker = context.state_dir.join("observation");
    fs::write(&blocker, b"not a directory").unwrap();
    tick_once(&context, &record);

    let receipt = incident(&context, &record)["notification"].clone();
    assert_eq!(receipt["escalation"]["state"], "unavailable");
    assert_eq!(receipt["escalation"]["recorded_at"], Value::Null);
    assert_eq!(receipt["delivered_at"], Value::Null);
    assert_eq!(receipt["degraded_reason"], "owner_unavailable");
    let deliverable = super::alert_delivery::list(&context, 100).unwrap();
    assert_eq!(deliverable["alerts"].as_array().unwrap().len(), 1);
    assert_eq!(deliverable["alerts"][0]["state"], "pending");

    fs::remove_file(&blocker).unwrap();
    tick_once(&context, &record);
    tick_once(&context, &record);
    let receipt = incident(&context, &record)["notification"].clone();
    assert_eq!(receipt["escalation"]["state"], "recorded");
    assert_eq!(events(&context).len(), 1);
}

#[test]
fn owner_route_restored_later_stops_further_alerts_and_keeps_the_audit() {
    let record = record();
    let (_tmp, context) = failed(&record);
    tick_once(&context, &record);
    assert_eq!(events(&context).len(), 1);

    let adopted = with_owner(record);
    crate::write_session_record(&context, &adopted).unwrap();
    tick_once(&context, &adopted);

    let receipt = incident(&context, &adopted)["notification"].clone();
    assert_ne!(receipt["degraded_reason"], "owner_unavailable");
    assert_eq!(receipt["escalation"]["state"], "recorded");
    assert_eq!(events(&context).len(), 1);
}

#[test]
fn child_with_an_owner_route_never_uses_the_alert_spool() {
    let record = with_owner(record());
    let (_tmp, context) = failed(&record);
    tick_once(&context, &record);

    let receipt = incident(&context, &record)["notification"].clone();
    assert_ne!(receipt["degraded_reason"], "owner_unavailable");
    assert_eq!(receipt.get("escalation"), None);
    assert_eq!(events(&context), Vec::new());
}

#[test]
fn quarantined_root_keeps_its_input_guard_while_the_alert_needs_no_input() {
    let record = record();
    let (_tmp, context) = failed(&record);
    tick_once(&context, &record);

    let error = recovery::ensure_input_allowed(&context, &record).unwrap_err();
    assert_eq!(error.code(), "auth-quarantined");
    assert_eq!(events(&context).len(), 1);
}

#[test]
fn settlement_alert_and_path_distinguish_health_from_controller_results() {
    for (result, path, severity) in [
        ("healthy", "model_health", Severity::Info),
        ("credentials_refreshed", "controller", Severity::Info),
        ("refresh_failed", "controller", Severity::Warn),
    ] {
        let record = record();
        let (_tmp, context) = failed(&record);
        tick_once(&context, &record);
        recover(&context, &record, result, "2030-01-01T00:00:05Z").unwrap();
        tick_once(&context, &record);
        tick_once(&context, &record);

        let incident = incident(&context, &record);
        assert_eq!(incident["recovery_result"], result);
        assert_eq!(incident["recovery_path"], path);
        let receipt = &incident["recovery_notification"];
        assert_eq!(receipt["escalation"]["state"], "recorded");
        assert_eq!(receipt["delivered_at"], Value::Null);
        let alerts = events(&context);
        assert_eq!(alerts.len(), 2);
        assert_eq!(alerts[1].code, "auth-recovery-owner-unavailable");
        assert_eq!(alerts[1].severity, severity);
        assert_eq!(alerts[1].recovery_action, None);
        assert_ne!(alerts[0].correlation, alerts[1].correlation);
    }
}

#[test]
fn renamed_executable_is_used_in_owner_escalation() {
    crate::command_identity::with_name(
        "session-control",
        parentless_incident_records_one_deduplicated_alert_without_claiming_delivery,
    );
}

#[test]
fn owner_unavailable_alert_exposes_pending_person_and_watchdog_delivery() {
    use nils_test_support::{EnvGuard, GlobalStateLock};
    let lock = GlobalStateLock::new();
    for configured in ["person", "person,watchdog"] {
        let _targets = EnvGuard::set(&lock, "AGENT_SESSION_AUTH_ALERT_TARGETS", configured);
        let record = record();
        let (_tmp, context) = failed(&record);
        tick_once(&context, &record);
        let bytes = fs::read(session_dir(&context, &record.id).join("auth-alerts.json"))
            .expect("owner-unavailable incident has no deliverable record, only a spool entry");
        let store: Value = serde_json::from_slice(&bytes).unwrap();
        let alert = &store["alerts"][0];
        assert_eq!(alert["state"], "pending");
        let expected = configured.split(',').collect::<Vec<_>>();
        assert_eq!(alert["required_targets"], json!(expected));
        for target in expected {
            assert_eq!(alert["delivery"][target]["state"], "pending");
            assert_eq!(alert["delivery"][target]["delivered_at"], Value::Null);
        }
    }
}

#[test]
fn alert_delivery_acks_only_configured_targets_and_retains_transport_proof() {
    use super::alert_delivery::{self, Target};
    use nils_test_support::{EnvGuard, GlobalStateLock};
    let lock = GlobalStateLock::new();
    for configured in ["person", "person,watchdog"] {
        let _targets = EnvGuard::set(&lock, "AGENT_SESSION_AUTH_ALERT_TARGETS", configured);
        let record = record();
        let (_tmp, context) = failed(&record);
        tick_once(&context, &record);
        let listed = alert_delivery::list(&context, 100).unwrap();
        let alert_id = listed["alerts"][0]["alert_id"].as_str().unwrap();
        let proof = format!("sha256:{}", "a".repeat(64));
        let delivered =
            alert_delivery::acknowledge(&context, &record.id, alert_id, Target::Person, &proof)
                .unwrap();
        let delivered = serde_json::to_value(delivered).unwrap();
        assert_eq!(
            delivered["state"],
            if configured == "person" {
                "delivered"
            } else {
                "pending"
            }
        );
        assert_eq!(delivered["delivery"]["person"]["receipt"], proof);
        let path = session_dir(&context, &record.id).join("auth-alerts.json");
        let before = fs::read(&path).unwrap();
        alert_delivery::acknowledge(&context, &record.id, alert_id, Target::Person, &proof)
            .unwrap();
        assert_eq!(fs::read(&path).unwrap(), before);
        let conflict = format!("sha256:{}", "b".repeat(64));
        assert_eq!(
            alert_delivery::acknowledge(&context, &record.id, alert_id, Target::Person, &conflict)
                .unwrap_err()
                .code(),
            "auth-alert-receipt-conflict"
        );
        assert_eq!(fs::read(&path).unwrap(), before);
        if configured == "person,watchdog" {
            // Configuration is frozen on the alert even if the daemon changes.
            let _changed = EnvGuard::set(&lock, "AGENT_SESSION_AUTH_ALERT_TARGETS", "person");
            let result = alert_delivery::acknowledge(
                &context,
                &record.id,
                alert_id,
                Target::Watchdog,
                &conflict,
            )
            .unwrap();
            assert_eq!(serde_json::to_value(result).unwrap()["state"], "delivered");
        } else {
            assert_eq!(
                alert_delivery::acknowledge(
                    &context,
                    &record.id,
                    alert_id,
                    Target::Watchdog,
                    &proof
                )
                .unwrap_err()
                .code(),
                "auth-alert-target-unconfigured"
            );
        }
        tick_once(&context, &record);
        let listed = alert_delivery::list(&context, 100).unwrap();
        assert_eq!(listed["alerts"].as_array().unwrap().len(), 1);
        assert_eq!(listed["alerts"][0]["state"], "delivered");
        assert!(listed["alerts"][0]["delivered_at"].is_string());
        assert_eq!(
            incident(&context, &record)["notification"]["delivered_at"],
            Value::Null
        );
        assert_eq!(events(&context).len(), 1);
        assert!(recovery::ensure_input_allowed(&context, &record).is_err());
    }
}

#[test]
fn alert_delivery_retries_without_repeating_a_recorded_spool_event() {
    let record = record();
    let (_tmp, context) = failed(&record);
    let blocker = session_dir(&context, &record.id).join("auth-alerts.json");
    fs::create_dir(&blocker).unwrap();
    tick_once(&context, &record);
    assert_eq!(
        incident(&context, &record)["notification"]["escalation"]["state"],
        "delivery_unavailable"
    );
    assert_eq!(events(&context).len(), 1);
    fs::remove_dir(&blocker).unwrap();
    tick_once(&context, &record);
    tick_once(&context, &record);
    assert_eq!(
        incident(&context, &record)["notification"]["escalation"]["state"],
        "recorded"
    );
    assert_eq!(events(&context).len(), 1);
    assert_eq!(
        super::alert_delivery::list(&context, 100).unwrap()["alerts"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn alert_delivery_keeps_incident_and_later_result_acknowledgements_independent() {
    use super::alert_delivery::{self, Target};
    use nils_test_support::{EnvGuard, GlobalStateLock};
    let lock = GlobalStateLock::new();
    let _targets = EnvGuard::set(&lock, "AGENT_SESSION_AUTH_ALERT_TARGETS", "person");
    let record = record();
    let (_tmp, context) = failed(&record);
    tick_once(&context, &record);
    let listed = alert_delivery::list(&context, 100).unwrap();
    let original = listed["alerts"][0]["alert_id"].as_str().unwrap();
    recover(
        &context,
        &record,
        "credentials_refreshed",
        "2030-01-01T00:01:00Z",
    )
    .unwrap();
    tick_once(&context, &record);
    alert_delivery::acknowledge(
        &context,
        &record.id,
        original,
        Target::Person,
        &format!("sha256:{}", "a".repeat(64)),
    )
    .unwrap();
    let listed = alert_delivery::list(&context, 100).unwrap();
    assert_eq!(listed["alerts"].as_array().unwrap().len(), 2);
    assert_eq!(listed["alerts"][0]["phase"], "recovery");
    assert_eq!(listed["alerts"][0]["result"], "credentials_refreshed");
    assert_eq!(listed["alerts"][0]["recovery_action"], Value::Null);
    assert_eq!(listed["alerts"][0]["state"], "pending");
    assert_eq!(listed["alerts"][1]["state"], "delivered");
    let only_pending = alert_delivery::list(&context, 1).unwrap();
    assert_eq!(only_pending["alerts"][0]["state"], "pending");
    assert_eq!(only_pending["truncated"], true);
}

#[test]
fn alert_delivery_defaults_to_person_and_retries_invalid_configuration() {
    use nils_test_support::{EnvGuard, GlobalStateLock};
    let lock = GlobalStateLock::new();
    let _unset = EnvGuard::remove(&lock, "AGENT_SESSION_AUTH_ALERT_TARGETS");
    let record = record();
    let (_tmp, context) = failed(&record);
    {
        let _invalid = EnvGuard::set(&lock, "AGENT_SESSION_AUTH_ALERT_TARGETS", "unknown");
        tick_once(&context, &record);
        assert_eq!(
            incident(&context, &record)["notification"]["escalation"]["state"],
            "delivery_unavailable"
        );
        assert_eq!(
            super::alert_delivery::list(&context, 100).unwrap()["alerts"],
            json!([])
        );
    }
    tick_once(&context, &record);
    let listed = super::alert_delivery::list(&context, 100).unwrap();
    assert_eq!(listed["alerts"][0]["required_targets"], json!(["person"]));
    assert_eq!(events(&context).len(), 1);
}

#[test]
fn alert_delivery_refuses_invalid_proof_without_recording_delivery() {
    use super::alert_delivery::{self, Target};
    let record = record();
    let (_tmp, context) = failed(&record);
    tick_once(&context, &record);
    let listed = alert_delivery::list(&context, 100).unwrap();
    let alert_id = listed["alerts"][0]["alert_id"].as_str().unwrap();
    let path = session_dir(&context, &record.id).join("auth-alerts.json");
    let before = fs::read(&path).unwrap();
    for invalid in [
        "receipt".to_string(),
        format!("sha256:{}", "a".repeat(63)),
        format!("sha256:{}", "A".repeat(64)),
        format!("sha256:{}", "g".repeat(64)),
    ] {
        assert_eq!(
            alert_delivery::acknowledge(&context, &record.id, alert_id, Target::Person, &invalid)
                .unwrap_err()
                .code(),
            "auth-alert-receipt-invalid"
        );
        assert_eq!(fs::read(&path).unwrap(), before);
    }
}

#[test]
fn alert_delivery_drops_a_superseded_unlocked_notification_snapshot() {
    use super::alert_delivery;
    for changed in ["delivered", "result", "owner_restored"] {
        let record = record();
        let (_tmp, context) = failed(&record);
        tick_once(&context, &record);
        recover(&context, &record, "refresh_failed", "2030-01-01T00:01:00Z").unwrap();
        tick_once(&context, &record);
        let stale = read(&context, &record).unwrap().incidents[0].clone();
        fs::remove_file(session_dir(&context, &record.id).join("auth-alerts.json")).unwrap();
        let mut store = read(&context, &record).unwrap();
        if changed == "delivered" {
            store.incidents[0].recovery_notification.delivered_at =
                Some("2030-01-01T00:02:00Z".into());
        } else if changed == "result" {
            store.incidents[0].recovery_result = Some("credentials_refreshed".into());
        } else {
            crate::write_session_record(&context, &with_owner(record.clone())).unwrap();
        }
        save(&context, &record, &store).unwrap();
        alert_delivery::enqueue(&context, &stale, true).unwrap();
        assert_eq!(
            alert_delivery::list(&context, 100).unwrap()["alerts"],
            json!([])
        );
        if changed == "result" {
            tick_once(&context, &record);
            let listed = alert_delivery::list(&context, 100).unwrap();
            assert!(
                listed["alerts"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|alert| alert["result"] != "refresh_failed")
            );
            assert!(
                listed["alerts"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|alert| alert["result"] == "credentials_refreshed")
            );
        }
    }
}
