use super::*;
use pretty_assertions::assert_eq;

#[test]
fn native_idle_reason_survives_notice_record_projection_without_provider_values() {
    let mut registry = Registry::default();
    schedule(&mut registry, "target", "incarnation", now_epoch());
    let receipt = registry.notifications.values_mut().next().unwrap();
    receipt.last_reason = Some("provider-not-ready".to_string());
    receipt.last_reason_detail = Some("send-idle-unobservable:thread_status_unknown".to_string());
    let stored = serde_json::to_vec(&registry).unwrap();
    let mut loaded: Registry = serde_json::from_slice(&stored).unwrap();
    let projection = projection_for(&mut loaded, "target", "incarnation", true).unwrap();
    assert_eq!(
        reason_detail(loaded.notifications.values().next().unwrap()).as_deref(),
        Some("send-idle-unobservable:thread_status_unknown")
    );
    assert_eq!(
        projection.last_reason.as_deref(),
        Some("provider-not-ready")
    );
    assert_eq!(
        safe_reason_detail(
            "provider-not-ready",
            "send-idle-unobservable:private-provider-status"
        ),
        None
    );
}
