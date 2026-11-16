//! Disposable publisher -> broker -> structured hook -> recovery acceptance.
//! Fake runtime readiness/submission is not live provider authentication.
use super::*;
use pretty_assertions::{assert_eq, assert_ne};

fn publisher_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/auth-publication/broker.py")
}

fn publisher_command(root: &Path, verb: &str, generation: u64) -> std::process::Command {
    let mut command = std::process::Command::new("python3");
    command
        .arg(publisher_path())
        .arg("--root")
        .arg(root)
        .arg(verb)
        .arg("--generation")
        .arg(generation.to_string());
    command
}

fn publisher(root: &Path, verb: &str, generation: u64) {
    let result = publisher_command(root, verb, generation).output().unwrap();
    assert!(result.status.success(), "synthetic publisher {verb} failed");
}

fn synthetic_broker(root: &Path, lock: &GlobalStateLock) -> EnvGuard {
    publisher(root, "stage", 1);
    publisher(root, "publish", 1);
    EnvGuard::set(
        lock,
        "AGENT_SESSION_CLAUDE_ACCOUNT_BROKER",
        &json!(["python3", publisher_path(), "--root", root]).to_string(),
    )
}

fn launch_bound(context: &CliContext, root: &Path, tmux: &Path) -> SessionRecord {
    let mut seed = seed_exited_session(context, root);
    // Binding before resume makes the real launch path materialize and capture
    // the publisher's generation, rather than attaching a checkpoint by hand.
    crate::claude_account::bind_initial(&mut seed, "alpha", "explicit").unwrap();
    crate::write_session_record(context, &seed).unwrap();
    let launch = running_session(context, tmux);
    session_ready(context, &launch);
    load_session_record(context, ID).unwrap()
}

// Snapshot the fixture provider's launch store independently of the consumer
// checkpoint. This models a resident read, not a provider acknowledgement.
fn resident_generation(root: &Path) -> u64 {
    let env = fs::read_to_string(root.join("env")).unwrap();
    let config = env
        .lines()
        .find_map(|line| line.strip_prefix("CLAUDE_CONFIG_DIR="))
        .unwrap();
    assert!(Path::new(config).starts_with(root.canonicalize().unwrap()));
    let credentials: Value =
        serde_json::from_slice(&fs::read(Path::new(config).join(".credentials.json")).unwrap())
            .unwrap();
    assert_eq!(credentials["synthetic"], true);
    credentials["generation"].as_u64().unwrap()
}

fn store(context: &CliContext) -> Value {
    serde_json::from_slice(
        &fs::read(crate::session_dir(context, ID).join("auth-incidents.json")).unwrap(),
    )
    .unwrap()
}

#[test]
fn synthetic_publication_recovers_only_after_verified_completion() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::tempdir().unwrap();
    let _broker = synthetic_broker(tmp.path(), &lock);
    let context = context(tmp.path());
    let tmux = fake_tmux(tmp.path());
    let record = launch_bound(&context, tmp.path(), &tmux);
    let launch = launch_id(&record).unwrap();
    let baseline = crate::claude_account::generation::baseline(&record).unwrap();
    let resident = resident_generation(tmp.path());
    assert_eq!(resident, 1);
    assert_eq!(baseline.version.generation, "1");
    assert_eq!(baseline.account, "alpha");
    assert_eq!(
        record.extra["claude_launch_publication"]["runtime_incarnation"],
        launch
    );
    crate::coordination::auth_effects::test_support::seed(&context, &record);
    claude_hook(
        &context,
        &launch,
        json!({"hook_event_name":"StopFailure", "error":"authentication_failed",
            "session_id":CONVERSATION}),
    );
    let incident_id = store(&context)["incidents"][0]["incident_id"].clone();
    let before = log(tmp.path());
    let error = crate::send_input_serialized(
        &context,
        &record,
        Some("synthetic refused prompt"),
        &[SpecialKey::Enter],
        &tmux,
    )
    .unwrap_err();
    assert_eq!(error.code(), "auth-quarantined");
    assert_eq!(error.details().unwrap()["pasted"], false);
    assert_eq!(error.details().unwrap()["submitted"], false);
    tick(&context, &record, &tmux, 100);
    let pending = recovery_receipt(&context);
    assert_eq!(pending["state"], "waiting_generation");

    // New stores alone cannot authorize recovery; last_published is still 1.
    publisher(tmp.path(), "stage", 2);
    assert_ne!(resident, 2, "resident still uses the rejected generation");
    let staged: Value =
        serde_json::from_slice(&fs::read(tmp.path().join("publication.json")).unwrap()).unwrap();
    assert_eq!(
        staged["state"]["publication"]["last_published"]["version"]["generation"],
        "1"
    );
    tick(&context, &record, &tmux, 115);
    assert_eq!(recovery_receipt(&context)["state"], "waiting_generation");
    assert_eq!(
        launch_id(&load_session_record(&context, ID).unwrap()),
        Some(launch.clone())
    );
    assert!(!log(tmp.path())[before.len()..].contains("paste-buffer"));
    assert!(!log(tmp.path())[before.len()..].contains("kill-session"));

    // Completion must verify physical stores, not just stamp a receipt.
    let config = tmp.path().join("alpha/config.json");
    let staged_config = fs::read(&config).unwrap();
    fs::write(&config, b"{}").unwrap();
    assert!(
        !publisher_command(tmp.path(), "publish", 2)
            .output()
            .unwrap()
            .status
            .success()
    );
    let refused: Value =
        serde_json::from_slice(&fs::read(tmp.path().join("publication.json")).unwrap()).unwrap();
    assert_eq!(
        refused, staged,
        "failed verification must not publish completion"
    );
    fs::write(&config, staged_config).unwrap();
    publisher(tmp.path(), "publish", 2);
    let published = crate::claude_account::generation::published("alpha")
        .unwrap()
        .unwrap();
    assert!(published.newer_than(&baseline));
    assert_ne!(published.snapshot_id, baseline.snapshot_id);
    assert_eq!(published.version.generation, "2");
    assert!(crate::auth_incident::recovery::ensure_input_allowed(&context, &record).is_err());
    fs::write(tmp.path().join("exit-on-probe"), "").unwrap();
    let watcher = on_relaunched_prompt(&context, tmp.path(), &launch);
    tick(&context, &record, &tmux, 129);
    watcher.join().unwrap();
    let current = load_session_record(&context, ID).unwrap();
    let receipt = recovery_receipt(&context);
    assert_eq!(resident_generation(tmp.path()), 2);
    assert_eq!(receipt["state"], "runtime_resumed");
    assert_eq!(receipt["operation_id"], pending["operation_id"]);
    assert_eq!(store(&context)["incidents"][0]["incident_id"], incident_id);
    assert_eq!(current.id, record.id);
    assert_eq!(
        current.provider_resume.as_ref().unwrap().session_id,
        CONVERSATION
    );
    assert_ne!(launch_id(&current), Some(launch));
    assert_eq!(
        current.runtime.as_ref().unwrap().generation,
        record.runtime.as_ref().unwrap().generation + 1
    );
    assert_eq!(
        crate::claude_account::generation::baseline(&current),
        Some(published)
    );
    assert_eq!(receipt["replayed"], false);
    assert_eq!(receipt["unknown_operation_count"], 2);
    crate::coordination::auth_effects::test_support::assert_retired(&context, &record);
    assert!(!log(tmp.path())[before.len()..].contains("paste-buffer"));
    guarded_send::assert_submitted(&context, tmp.path(), &tmux, &current);
    let effects = log(tmp.path());
    tick(&context, &current, &tmux, 145);
    assert_eq!(
        log(tmp.path()),
        effects,
        "terminal recovery cannot replay input"
    );
}

#[test]
fn synthetic_publication_before_launch_prevents_initial_stale_generation_refusal() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::tempdir().unwrap();
    let _broker = synthetic_broker(tmp.path(), &lock);
    // This ordering models a launch barrier, not resident token rereading or
    // provider overlap. A live prevention claim still requires those proofs.
    publisher(tmp.path(), "stage", 2);
    publisher(tmp.path(), "publish", 2);
    let context = context(tmp.path());
    let tmux = fake_tmux(tmp.path());
    let record = launch_bound(&context, tmp.path(), &tmux);
    assert_eq!(resident_generation(tmp.path()), 2);
    assert_eq!(
        crate::claude_account::generation::baseline(&record)
            .unwrap()
            .version
            .generation,
        "2"
    );
    assert!(crate::auth_incident::view(&context, &record).is_none());
    guarded_send::assert_submitted(&context, tmp.path(), &tmux, &record);
    assert!(crate::auth_incident::view(&context, &record).is_none());
    assert_eq!(log(tmp.path()).matches("new-session").count(), 1);
}
