use super::*;
use nils_common::fs::{SECRET_FILE_MODE, write_atomic};
use pretty_assertions::assert_eq;

#[test]
fn reply_deadline_audit_threshold_is_strict_and_independent_of_unread_age() {
    let temp = tempfile::TempDir::new().unwrap();
    let context = CliContext {
        state_dir: temp.path().into(),
        host: None,
    };
    std::fs::create_dir(temp.path().join("coordination")).unwrap();
    let fixture: Value = serde_json::from_str(include_str!(
        "../../tests/fixtures/coordination/mail-audit-v1.json"
    ))
    .unwrap();
    let mut registry = fixture["registry"].clone();
    registry["messages"].as_array_mut().unwrap().truncate(1);
    registry["messages"][0]["reply_by_epoch"] = json!(500);
    registry["messages"][0]["state"] = json!("acknowledged");
    let path = temp.path().join("coordination/registry.json");
    write_atomic(
        &path,
        &serde_json::to_vec(&registry).unwrap(),
        SECRET_FILE_MODE,
    )
    .unwrap();
    let query = Query {
        older_than: 10000,
        include_healthy: true,
        ..Query::default()
    };
    let at = snapshot_at(&context, "destination", query.clone(), 500).unwrap();
    let row = &at["records"][0];
    assert_eq!(row["reply_by"], "1970-01-01T00:08:20Z");
    assert!(
        !row["anomalies"]
            .as_array()
            .unwrap()
            .contains(&json!("overdue-reply"))
    );
    let after = snapshot_at(&context, "destination", query, 501).unwrap();
    assert_eq!(after["records"][0]["anomalies"], json!(["overdue-reply"]));
}
