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

#[cfg(target_os = "linux")]
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
fn descriptor_is_emptied_when_the_lock_is_released() {
    let (_temp, context) = context();
    let held = lock_registry_observational(&context).unwrap();
    let sidecar = coordination_root(&context).unwrap().join(HOLDER_FILE);
    assert!(fs::metadata(&sidecar).unwrap().len() > 0);
    drop(held);
    assert_eq!(fs::metadata(&sidecar).unwrap().len(), 0);
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
        "process_start": 1_u64,
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
    assert_eq!(
        keys,
        ["acquired_at_epoch_ms", "operation", "pid", "process_start"]
    );
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

#[cfg(target_os = "linux")]
#[test]
fn live_pid_with_a_different_start_time_is_reported_stale() {
    let (_temp, context) = context();
    let held = lock_private_store(&context, REGISTRY_LOCK).unwrap();
    let sidecar = coordination_root(&context).unwrap().join(HOLDER_FILE);
    // This process is alive, but the recorded start time belongs to a previous
    // process that held the same pid.
    let reused = json!({
        "operation": "full-maintenance",
        "pid": std::process::id(),
        "process_start": 1_u64,
        "acquired_at_epoch_ms": 1_000_u64,
    });
    write_atomic(
        &sidecar,
        &serde_json::to_vec(&reused).unwrap(),
        SECRET_FILE_MODE,
    )
    .unwrap();

    let holder = timeout_details(&context)["holder"].clone();

    assert_eq!(holder, json!({ "state": "stale" }));
    drop(held);
}

/// Holds the lock without a descriptor and writes `descriptor` for this live
/// process, so only the field under test decides what the timeout reports.
#[cfg(target_os = "linux")]
fn hold_with_sidecar(context: &CliContext, mut descriptor: Value, pad_to: usize) -> File {
    let held = lock_private_store(context, REGISTRY_LOCK).unwrap();
    let object = descriptor.as_object_mut().unwrap();
    object.insert("pid".into(), json!(std::process::id()));
    object.insert(
        "process_start".into(),
        json!(lock_holder::process_start(std::process::id())),
    );
    object.insert("acquired_at_epoch_ms".into(), json!(1_000_u64));
    let mut bytes = serde_json::to_vec(&descriptor).unwrap();
    bytes.resize(bytes.len().max(pad_to), b' ');
    let sidecar = coordination_root(context).unwrap().join(HOLDER_FILE);
    write_atomic(&sidecar, &bytes, SECRET_FILE_MODE).unwrap();
    held
}

#[cfg(target_os = "linux")]
#[test]
fn a_valid_live_descriptor_is_reported_active() {
    let (_temp, context) = context();
    let held = hold_with_sidecar(&context, json!({ "operation": "lifecycle" }), 0);

    let holder = timeout_details(&context)["holder"].clone();

    assert_eq!(holder["state"], "active");
    assert_eq!(holder["operation"], "lifecycle");
    drop(held);
}

#[cfg(target_os = "linux")]
#[test]
fn foreign_operation_text_is_never_reported() {
    let (_temp, context) = context();
    let held = hold_with_sidecar(&context, json!({ "operation": "session-secret-id" }), 0);

    let details = timeout_details(&context);

    assert!(details.get("holder").is_none());
    assert!(!details.to_string().contains("session-secret-id"));
    drop(held);
}

#[cfg(target_os = "linux")]
#[test]
fn oversized_descriptor_is_ignored() {
    let (_temp, context) = context();
    let held = hold_with_sidecar(&context, json!({ "operation": "lifecycle" }), 513);

    assert!(timeout_details(&context).get("holder").is_none());
    drop(held);
}

#[cfg(target_os = "linux")]
#[test]
fn descriptor_with_an_extra_field_is_ignored() {
    let (_temp, context) = context();
    let held = hold_with_sidecar(
        &context,
        json!({ "operation": "lifecycle", "session_id": "extra" }),
        0,
    );

    assert!(timeout_details(&context).get("holder").is_none());
    drop(held);
}

#[cfg(target_os = "linux")]
#[test]
fn observational_reminder_and_lifecycle_classes_are_reported_by_name() {
    let (_temp, context) = context();
    let held = lock_registry_observational(&context).unwrap();
    assert_eq!(
        timeout_details(&context)["holder"]["operation"],
        "observational"
    );
    drop(held);

    let held = lock_registry_for_lifecycle(&context).unwrap();
    assert_eq!(
        timeout_details(&context)["holder"]["operation"],
        "lifecycle"
    );
    drop(held);

    let held = lock_registry_within(
        &context,
        RegistryMaintenance::Full,
        lock_holder::HolderClass::Reminder,
        LOCK_TIMEOUT,
    )
    .unwrap();
    assert_eq!(timeout_details(&context)["holder"]["operation"], "reminder");
    drop(held);
}

#[cfg(target_os = "linux")]
#[test]
fn a_successors_descriptor_survives_the_predecessors_release() {
    let (_temp, context) = context();
    for _ in 0..50 {
        let predecessor = lock_registry(&context).unwrap();
        let waiter = {
            let context = context.clone();
            thread::spawn(move || lock_registry_observational(&context).unwrap())
        };
        thread::sleep(Duration::from_millis(15));
        drop(predecessor);
        let successor = waiter.join().unwrap();

        let sidecar = coordination_root(&context).unwrap().join(HOLDER_FILE);
        let text = fs::read_to_string(sidecar).unwrap();
        assert!(text.contains("observational"), "descriptor lost: {text:?}");
        drop(successor);
    }
}

#[test]
fn a_non_timeout_error_is_not_annotated() {
    let (_temp, context) = context();
    let held = lock_registry(&context).unwrap();
    let root = coordination_root(&context).unwrap();

    let error = lock_holder::annotate_timeout(&root, CliError::runtime("other-code", "x", None));

    assert!(error.details().is_none());
    drop(held);
}
