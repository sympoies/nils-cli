use super::lock_holder::HOLDER_FILE;
use super::*;
use pretty_assertions::assert_eq;
use serde_json::json;

fn context() -> (tempfile::TempDir, CliContext) {
    let temp = tempfile::tempdir().unwrap();
    let context = CliContext {
        state_dir: temp.path().join("state"),
        host: None,
    };
    (temp, context)
}

fn timeout_details(context: &CliContext) -> Value {
    let error = lock_registry_within(
        context,
        RegistryMaintenance::Full,
        lock_holder::HolderClass::Lifecycle,
        Duration::from_millis(50),
    )
    .err()
    .expect("lock is held");
    assert_eq!(error.code(), "coordination-lock-timeout");
    error.details().cloned().unwrap_or_else(|| json!({}))
}

#[test]
fn timeout_reports_holder_class_and_age() {
    let (_temp, context) = context();
    let held = lock_registry(&context).unwrap();
    thread::sleep(Duration::from_millis(150));

    let holder = timeout_details(&context)["holder"].clone();

    assert_eq!(holder["state"], "active");
    assert_eq!(holder["operation"], "full-maintenance");
    assert_eq!(holder["pid"], std::process::id());
    let held_ms = holder["held_ms"].as_u64().unwrap();
    assert!((150..2_000).contains(&held_ms), "held_ms={held_ms}");
    drop(held);
}

#[test]
fn descriptor_is_retired_when_the_lock_is_released() {
    let (_temp, context) = context();
    let held = lock_registry_observational(&context).unwrap();
    let sidecar = coordination_root(&context).unwrap().join(HOLDER_FILE);
    assert!(sidecar.exists());
    drop(held);
    assert!(!sidecar.exists());
    let free = lock_registry(&context).unwrap();
    drop(free);
}

#[test]
fn descriptor_of_a_gone_holder_is_reported_stale_without_trusting_it() {
    let (_temp, context) = context();
    let mut child = std::process::Command::new("true").spawn().unwrap();
    let dead_pid = child.id();
    child.wait().unwrap();
    let held = lock_private_store(&context, REGISTRY_LOCK).unwrap();
    let sidecar = coordination_root(&context).unwrap().join(HOLDER_FILE);
    let stale = json!({
        "operation": "lifecycle",
        "pid": dead_pid,
        "acquired_at_epoch_ms": 1_000_u64,
    });
    write_atomic(
        &sidecar,
        &serde_json::to_vec(&stale).unwrap(),
        SECRET_FILE_MODE,
    )
    .unwrap();

    let holder = timeout_details(&context)["holder"].clone();

    assert_eq!(holder, json!({ "state": "stale" }));
    drop(held);
}

#[test]
fn descriptor_carries_only_class_pid_and_acquisition_time() {
    let (_temp, context) = context();
    let held = lock_registry(&context).unwrap();
    let sidecar = coordination_root(&context).unwrap().join(HOLDER_FILE);
    let text = fs::read_to_string(&sidecar).unwrap();

    let value: Value = serde_json::from_str(&text).unwrap();
    let mut keys: Vec<_> = value.as_object().unwrap().keys().cloned().collect();
    keys.sort();
    assert_eq!(keys, ["acquired_at_epoch_ms", "operation", "pid"]);
    assert!(!text.contains(context.state_dir.to_str().unwrap()));
    assert_eq!(fs::metadata(&sidecar).unwrap().mode() & 0o077, 0);
    drop(held);
}

#[test]
fn timeout_without_a_descriptor_keeps_its_existing_shape() {
    let (_temp, context) = context();
    let held = lock_private_store(&context, REGISTRY_LOCK).unwrap();

    let details = timeout_details(&context);

    assert!(details.get("holder").is_none());
    drop(held);
}
