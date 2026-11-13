//! Native composer facts are independent of the synthetic terminal render.
use super::*;
use pretty_assertions::assert_eq;

pub(super) fn unfamiliar_screen(root: &Path) {
    fs::write(
        root.join("captured-layout"),
        json!({"input":"", "pane":"unfamiliar synthetic composer layout\n", "start":[0,0], "prefix_end":[0,0]}).to_string(),
    ).unwrap();
}

#[test]
fn non_idle_native_notice_refuses_at_admission_without_claim_or_text() {
    let fixture = super::super::Fixture::new("claude");
    let peer = Peer::start(&fixture, "accepted");
    let activity = fixture.root().join("state/sessions/guarded/activity.json");
    let mut state: Value = serde_json::from_slice(&fs::read(&activity).unwrap()).unwrap();
    state["state"]["phase"] = json!("working");
    fs::write(activity, serde_json::to_vec(&state).unwrap()).unwrap();

    let output = send_notice(&fixture, "synthetic\n");
    let result = fixture.refused(&output, "send-session-not-idle");
    assert_eq!(result["error"]["details"]["guard"], "idle");
    assert_eq!(result["error"]["details"]["stage"], "admission");
    assert_eq!(result["error"]["details"]["pasted"], false);
    assert_eq!(result["error"]["details"]["submitted"], false);
    assert!(peer.text.lock().unwrap().is_empty());
    assert!(!peer.consumed_before_text.load(Ordering::SeqCst));
    super::refusals::no_claim(&fixture);
}

#[test]
fn authenticated_empty_notice_ignores_unfamiliar_screen_at_authorization() {
    for before_offer in [false, true] {
        let fixture = super::super::Fixture::new("claude");
        let peer = Peer::start(&fixture, "unfamiliar-authorization");
        if before_offer {
            unfamiliar_screen(fixture.root());
        }
        let output = send_notice(&fixture, "synthetic\n");
        assert_eq!(
            output.code,
            0,
            "{} {}",
            output.stdout_text(),
            output.stderr_text()
        );
        assert_eq!(
            output.stdout_json()["data"]["native"]["transport"],
            "claude_mod"
        );
        assert_eq!(peer.text.lock().unwrap().len(), 1);
        assert!(peer.consumed_before_text.load(Ordering::SeqCst));
        fixture.no_delivery();
        assert_eq!(
            fs::read_to_string(fixture.root().join("composer")).unwrap(),
            ""
        );
    }
}

fn module_peer(
    fixture: &super::super::Fixture,
    mode: &'static str,
) -> std::thread::JoinHandle<std::process::Output> {
    let (module, _) = bind_modeled_peer(fixture);
    let root = fixture.root().to_owned();
    std::thread::spawn(move || {
        let mut command = nils_test_support::node::command();
        command
            .arg(
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/fixtures/claude-notice-composer.mjs"),
            )
            .arg(root)
            .arg(module)
            .arg(mode);
        nils_test_support::cmd::strip_ambient_managed_session_env(&mut command);
        command.output().expect("synthetic native module")
    })
}

fn module_trace(thread: std::thread::JoinHandle<std::process::Output>) -> Value {
    let output = thread.join().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn native_draft_and_cursor_refuse_before_authorization() {
    crate::cases::for_each_case(&["draft-before", "cursor-before"], |&mode| {
        let fixture = super::super::Fixture::new("claude");
        let module = module_peer(&fixture, mode);
        let output = send_notice(&fixture, "synthetic\n");
        let trace = module_trace(module);
        let refusal = fixture.refused(&output, "notice-native-unavailable");
        assert_eq!(
            refusal["error"]["details"]["reason"],
            if mode == "draft-before" {
                "authorization-composer-not-empty"
            } else {
                "authorization-cursor"
            }
        );
        // One negative observation request, with no authorization granted.
        assert_eq!(trace["authorizations"], 1);
        assert_eq!(trace["submits"], json!([]));
        assert_eq!(trace["clears"], 0);
        assert!(!trace["reads"].as_array().unwrap().is_empty());
        assert_eq!(
            fs::read_to_string(fixture.root().join("composer")).unwrap(),
            if mode == "draft-before" {
                "native human draft"
            } else {
                ""
            }
        );
        super::refusals::no_claim(&fixture);
    });
}

#[test]
fn module_second_read_preserves_late_draft_and_consumed_attempt() {
    let fixture = super::super::Fixture::new("claude");
    let module = module_peer(&fixture, "draft-after");
    let output = send_notice(&fixture, "synthetic\n");
    let trace = module_trace(module);
    fixture.refused(&output, "notice-native-unconfirmed");
    assert_eq!(
        trace["reads"],
        json!([
            {"text":"", "cursor":0}, {"text":"new human draft", "cursor":15}
        ])
    );
    assert_eq!(trace["authorizations"], 1);
    assert_eq!(trace["submits"], json!([]));
    assert_eq!(trace["clears"], 0);
    assert_eq!(trace["reports"].as_array().unwrap().len(), 1);
    assert_eq!(trace["reports"][0]["outcome"], "unknown");
    assert_eq!(trace["reports"][0]["after_empty"], false);
    assert_eq!(
        fs::read_to_string(fixture.root().join("composer")).unwrap(),
        "new human draft"
    );
    fixture.refused(
        &send_notice(&fixture, "synthetic\n"),
        "send-idempotency-key-used",
    );
    fixture.no_delivery();
    assert!(
        fixture
            .root()
            .join("state/history/claude-notices/ledger.jsonl")
            .is_file()
    );
}
