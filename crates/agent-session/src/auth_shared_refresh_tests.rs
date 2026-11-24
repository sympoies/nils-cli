//! Synthetic shared-account rotation through the broker and Claude hook ingress.
use super::*;
use pretty_assertions::{assert_eq, assert_ne};

struct Resident {
    root: PathBuf,
    context: CliContext,
    tmux: crate::restart::tests::fixture::FakeTmux,
    initial: SessionRecord,
}

impl Resident {
    fn start(parent: &Path, id: &str) -> Self {
        let root = parent.join(id);
        fs::create_dir(&root).unwrap();
        let context = context(&root);
        let tmux = fake_tmux(&root);
        let mut seed = seed_exited_session_for(&context, &root, id);
        crate::claude_account::bind_initial(&mut seed, "alpha", "explicit").unwrap();
        crate::write_session_record(&context, &seed).unwrap();
        crate::resume_session_by_id(&context, id, &tmux).unwrap();
        let initial = load_session_record(&context, id).unwrap();
        session_ready_for(&context, id, &launch_id(&initial).unwrap());
        Self {
            root,
            context,
            tmux,
            initial,
        }
    }

    fn config_dir(&self) -> PathBuf {
        let env = fs::read_to_string(self.root.join("env")).unwrap();
        PathBuf::from(
            env.lines()
                .find_map(|line| line.strip_prefix("CLAUDE_CONFIG_DIR="))
                .unwrap(),
        )
    }

    fn observe_revocation(&self) {
        claude_hook_for(
            &self.context,
            &self.initial.id,
            &launch_id(&self.initial).unwrap(),
            json!({"hook_event_name":"StopFailure", "error":"authentication_failed",
                "last_assistant_message":"OAuth token revoked · Please run /login",
                "session_id":CONVERSATION}),
        );
        self.assert_quarantined();
    }

    fn assert_quarantined(&self) {
        let error =
            crate::auth_incident::recovery::ensure_input_allowed(&self.context, &self.initial)
                .unwrap_err();
        assert_eq!(error.code(), "auth-quarantined");
    }

    fn poll(&self, now: i64) {
        tick(&self.context, &self.initial, &self.tmux, now);
    }

    fn recover_at(&self, now: i64) {
        let old_launch = launch_id(&self.initial).unwrap();
        fs::write(self.root.join("exit-on-probe"), "").unwrap();
        let id = self.initial.id.clone();
        let watcher = on_relaunch(&self.context, &old_launch, move |context, launch| {
            session_ready_for(context, &id, launch);
        });
        let before = log(&self.root);
        self.poll(now);
        let current = load_session_record(&self.context, &self.initial.id).unwrap();
        assert_ne!(
            launch_id(&current),
            Some(old_launch),
            "shared publication left the sibling quarantined"
        );
        watcher.join().unwrap();
        let effects = log(&self.root);
        assert!(!effects[before.len()..].contains("paste-buffer"));
        assert_eq!(effects.matches("new-session").count(), 2);
        assert_eq!(
            current.provider_resume.as_ref().unwrap().session_id,
            CONVERSATION
        );
        let shared_dir = self
            .root
            .parent()
            .unwrap()
            .canonicalize()
            .unwrap()
            .join("alpha");
        assert_eq!(self.config_dir(), shared_dir);
        let credential: Value =
            serde_json::from_slice(&fs::read(shared_dir.join(".credentials.json")).unwrap())
                .unwrap();
        assert_eq!(credential, json!({"synthetic":true,"generation":2}));
        assert_eq!(
            crate::claude_account::generation::baseline(&current)
                .unwrap()
                .version
                .generation,
            "2"
        );
        super::super::guarded_send::assert_submitted(
            &self.context,
            &self.root,
            &self.tmux,
            &current,
        );
    }
}

#[test]
fn shared_refresh_recovers_two_revoked_siblings_on_one_late_publication() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::tempdir().unwrap();
    let _broker = synthetic_broker(tmp.path(), &lock);
    let first = Resident::start(tmp.path(), "refreshing-session");
    let siblings = [
        Resident::start(tmp.path(), "revoked-sibling-a"),
        Resident::start(tmp.path(), "revoked-sibling-b"),
    ];
    let first_log = log(&first.root);
    let first_credential = first.config_dir().join(".credentials.json");
    assert_eq!(
        first_credential,
        tmp.path()
            .canonicalize()
            .unwrap()
            .join("alpha/.credentials.json")
    );
    for sibling in &siblings {
        assert_eq!(sibling.config_dir(), first.config_dir());
        assert_eq!(
            crate::claude_account::generation::baseline(&sibling.initial)
                .unwrap()
                .version
                .generation,
            "1"
        );
    }

    // Stage the shared rotation. Both old residents fail before completion
    // is visible, and both remain alive and fenced while the publisher verifies it.
    publisher(tmp.path(), "stage", 2);
    thread::scope(|scope| {
        for sibling in &siblings {
            scope.spawn(move || sibling.observe_revocation());
        }
    });
    for sibling in &siblings {
        sibling.poll(100);
        sibling.poll(130);
        sibling.assert_quarantined();
    }
    publisher(tmp.path(), "publish", 2);
    for sibling in &siblings {
        sibling.recover_at(219);
    }
    assert_eq!(log(&first.root), first_log);
    assert_eq!(
        launch_id(&load_session_record(&first.context, &first.initial.id).unwrap()),
        launch_id(&first.initial)
    );
    assert!(crate::auth_incident::view(&first.context, &first.initial).is_none());
}

#[test]
fn shared_refresh_rejects_wrong_account_and_expires_before_late_adoption() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::tempdir().unwrap();
    let _broker = synthetic_broker(tmp.path(), &lock);
    let sibling = Resident::start(tmp.path(), "revoked-sibling");
    sibling.observe_revocation();
    sibling.poll(100);
    sibling.poll(130);
    publisher(tmp.path(), "stage", 2);
    publisher(tmp.path(), "publish", 2);
    let receipt_path = tmp.path().join("publication.json");
    let correct = fs::read(&receipt_path).unwrap();
    let mut wrong: Value = serde_json::from_slice(&correct).unwrap();
    wrong["account"] = json!("other-account");
    fs::write(&receipt_path, wrong.to_string()).unwrap();
    sibling.poll(219);
    sibling.assert_quarantined();
    assert_eq!(log(&sibling.root).matches("new-session").count(), 1);
    fs::write(&receipt_path, correct).unwrap();
    sibling.poll(220);
    sibling.assert_quarantined();
    assert_eq!(log(&sibling.root).matches("new-session").count(), 1);
}

#[test]
fn shared_refresh_does_not_extend_recovery_for_a_known_older_claude_version() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::tempdir().unwrap();
    let _broker = synthetic_broker(tmp.path(), &lock);
    let mut sibling = Resident::start(tmp.path(), "revoked-sibling");
    let launch = launch_id(&sibling.initial).unwrap();
    sibling.initial.extra.insert(
        "restart_provider_version".into(),
        json!({"launch_id": launch, "version":"2.1.247 (Claude Code)"}),
    );
    crate::write_session_record(&sibling.context, &sibling.initial).unwrap();
    sibling.observe_revocation();
    sibling.poll(100);
    sibling.poll(130);
    publisher(tmp.path(), "stage", 2);
    publisher(tmp.path(), "publish", 2);
    sibling.poll(131);

    sibling.assert_quarantined();
    assert_eq!(log(&sibling.root).matches("new-session").count(), 1);
}
