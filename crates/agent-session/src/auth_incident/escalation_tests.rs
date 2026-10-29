//! A session with no effective parent still gets an independent, deduplicated
//! alert through the daemon-independent observation spool.

use super::*;
use nils_common::observation::{self, Severity};
use pretty_assertions::assert_eq;
use serde_json::json;

const OWNER_ACTION: &str = "agent-session lineage adopt <CHILD> --by <SESSION>";

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
        state_dir: tmp.path().join("state"),
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
    assert_eq!(alert.recovery_action.as_deref(), Some(OWNER_ACTION));

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
