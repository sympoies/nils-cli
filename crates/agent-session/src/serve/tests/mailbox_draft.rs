use pretty_assertions::assert_eq;

use super::*;

struct WakeFixture {
    temp: tempfile::TempDir,
    state: Arc<ServeState>,
    candidate: crate::coordination::NotificationCandidate,
}

impl WakeFixture {
    async fn new(agent: AgentKind) -> Self {
        let temp = tempfile::TempDir::new().expect("fixture");
        seed_session_with_runtime(temp.path(), "alpha", "codex", "modeled-alpha");
        seed_session_with_runtime(temp.path(), "beta", agent.as_str(), "modeled-beta");
        fs::write(temp.path().join("composer"), "").expect("composer");
        fs::write(temp.path().join("calls"), "").expect("calls");
        fs::write(temp.path().join("provider"), agent.as_str()).expect("provider");
        fs::write(temp.path().join("transcript"), "").expect("transcript");
        let tmux = temp.path().join("tmux");
        fs::write(&tmux, r#"#!/usr/bin/env python3
import json, pathlib, sys
import subprocess
root = pathlib.Path(__file__).parent
args = sys.argv[1:]
with (root / 'calls').open('a') as out: out.write(json.dumps(args) + '\n')
command = args[0]
if command == 'has-session':
    if (root / 'exited-after-enter').exists():
        sys.stderr.write("can't find session: beta\n")
        sys.exit(1)
    if (root / 'unknown-after-enter').exists():
        sys.stderr.write("status probe failed\n")
        sys.exit(1)
    sys.exit(0)
if command == 'display-message': print('0'); sys.exit(0)
if command == 'show-environment':
    identities = json.loads((root / 'identities.json').read_text())
    print('\n'.join(f'{key}={identities[key]}' for key in args if key in identities))
    sys.exit(0)
if command == 'load-buffer':
    (root / 'buffer').write_bytes(pathlib.Path(args[-1]).read_bytes())
    if (root / 'draft-on-load').exists(): (root / 'composer').write_text('later operator draft')
elif command == 'paste-buffer':
    with (root / 'composer').open('a') as out: out.write((root / 'buffer').read_text())
    if (root / 'codex-collapse-paste').exists():
        text = (root / 'buffer').read_text().replace('\r\n', '\n').replace('\r', '\n')
        (root / 'composer').write_text(f'[Pasted Content {len(text)} chars]')
    (root / 'cursor-x').write_text(str(2 + len((root / 'composer').read_text())))
    if (root / 'working-after-paste').exists():
        subprocess.Popen([sys.executable, str(root / 'hold-ingress.py')], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    if (root / 'draft-after-paste').exists():
        with (root / 'composer').open('a') as out: out.write(' operator addition')
elif command == 'send-keys' and args[-1] == 'Enter':
    submitted = (root / 'composer').read_text()
    if (root / 'codex-collapse-paste').exists(): submitted = (root / 'buffer').read_text()
    with (root / 'submissions').open('a') as out: out.write(json.dumps(submitted) + '\n')
    if (root / 'emit-transcript-after-enter').exists():
        count_file = root / 'enter-count'
        count = int(count_file.read_text()) if count_file.exists() else 0
        count_file.write_text(str(count + 1))
        with (root / 'profile-config/projects/project-27/provider-beta.jsonl').open('a') as out:
            out.write(json.dumps({"type":"user", "sessionId":"provider-beta", "isSidechain":False, "isMeta":False, "promptSource":"typed", "message":{"role":"user", "content":submitted}}) + '\n')
    if (root / 'exit-after-next-enter').exists(): (root / 'exited-after-enter').touch()
    if (root / 'liveness-unknown-after-enter').exists(): (root / 'unknown-after-enter').touch()
    if (root / 'operator-after-first-enter').exists() and not (root / 'first-enter-seen').exists():
        (root / 'first-enter-seen').touch()
        with (root / 'composer').open('a') as out: out.write(' operator addition')
    elif (root / 'swallow-first-enter').exists(): (root / 'swallow-first-enter').unlink()
    elif (root / 'swallow-every-enter').exists(): pass
    elif (root / 'keep-after-enter').exists(): pass
    else: (root / 'composer').write_text('')
elif command == 'send-keys' and args[-1] == 'Delete':
    text = (root / 'composer').read_text()
    count = int(args[-2])
    prompt = (root / 'buffer').read_text()
    if text.startswith(prompt): (root / 'composer').write_text(text[len(prompt):])
elif command == 'send-keys' and args[-1] == 'C-a': (root / 'cursor-x').write_text('2')
elif command == 'send-keys' and args[-2:] == ['Right', 'Left']:
    end = 2 + len((root / 'composer').read_text())
    (root / 'cursor-x').write_text(str(max(2, min(end, int((root / 'cursor-x').read_text()) + 1) - 1)))
elif command == 'send-keys' and args[-1] == 'Right':
    end = 2 + len((root / 'composer').read_text())
    x = end if (root / 'codex-collapse-paste').exists() else min(end, int((root / 'cursor-x').read_text()) + int(args[-2]))
    (root / 'cursor-x').write_text(str(x))
elif command == 'send-keys' and args[-1] == 'Left' and (root / 'codex-collapse-paste').exists():
    (root / 'cursor-x').write_text('2')
elif command == 'send-keys' and args[-1] == 'BSpace':
    if (root / 'ignore-clear').exists(): sys.exit(0)
    text = (root / 'composer').read_text()
    count = int(args[-2])
    cursor = int((root / 'cursor-x').read_text()) - 2
    (root / 'composer').write_text(text[:max(0, cursor-count)] + text[cursor:])
elif command == 'capture-pane':
    if (root / 'edit-after-stash').exists() and (root / 'sessions/beta/drafts/saved').exists():
        (root / 'composer').write_text('owner  draft')
        (root / 'edit-after-stash').unlink()
    if (root / 'capture-unavailable').exists() or ((root / 'capture-unavailable-after-enter').exists() and (root / 'submissions').exists()): sys.exit(1)
    text = (root / 'composer').read_text()
    if (root / 'unknown-layout-after-enter').exists() and (root / 'submissions').exists():
        count_file = root / 'unknown-layout-captures'
        count = int(count_file.read_text()) if count_file.exists() else 0
        if count < 2:
            count_file.write_text(str(count + 1))
            print('unknown pane layout')
            sys.exit(0)
        (root / 'unknown-layout-after-enter').unlink()
    footer = (root / 'footer').read_text() if (root / 'footer').exists() else 'status'
    if (root / 'provider').read_text() == 'claude':
        print('────────────────────\n❯\u00a0' + text + '\n────────────────────\n  ' + footer)
    else: print('› ' + text + '\n\n  ' + footer)
    if 'display-message' in args:
        x = int((root / 'cursor-x').read_text()) if (root / 'cursor-x').exists() else 2
        y = 1 if (root / 'provider').read_text() == 'claude' else 0
        print(f'{x} {y}')
elif command != 'delete-buffer': sys.exit(1)
"#).expect("tmux");
        fs::set_permissions(&tmux, fs::Permissions::from_mode(0o700)).expect("permissions");
        let state = state(temp.path(), Some(TOKEN), tmux);
        let alpha = load_session_record(&state.context, "alpha").expect("sender");
        let beta = load_session_record(&state.context, "beta").expect("recipient");
        fs::write(temp.path().join("cursor-x"), "2").expect("cursor");
        fs::write(
            temp.path().join("identities.json"),
            serde_json::to_vec(&json!({
                "AGENT_SESSION_ID": beta.id,
                "AGENT_SESSION_STATE_DIR": crate::display_path(&state.context.state_dir),
                "AGENT_SESSION_RUNTIME_ID": beta.runtime.as_ref().expect("runtime").launch_id,
            }))
            .expect("identity json"),
        )
        .expect("write identities");
        let capability = provision_ready_coordination_fixture(&state.context, &alpha);
        provision_ready_coordination_fixture(&state.context, &beta);
        crate::activity::activate_runtime(&state.context, &beta).expect("activate");
        let runtime = &beta.runtime.as_ref().expect("runtime").launch_id;
        let kinds: &[&str] = if agent == AgentKind::Claude {
            &["stop_observed"]
        } else {
            &["turn_started", "turn_completed"]
        };
        for kind in kinds {
            let event = serde_json::from_value(json!({
                "schema_version": crate::activity::TURN_EVENT_VERSION,
                "event_id": format!("draft-fixture-{kind}"),
                "runtime_id": runtime, "provider": agent.as_str(),
                "provider_session_id": "provider-beta", "provider_turn_id": "fixture-turn",
                "kind": kind, "confidence": "authoritative"
            }))
            .expect("event");
            crate::activity::ingest_event(&state.context, &beta.id, event).expect("ingest");
        }
        let path = session_dir(&state.context, &beta.id).join("activity.json");
        let mut activity: Value =
            serde_json::from_slice(&fs::read(&path).expect("activity")).expect("json");
        activity["last_provider_event_at"] = json!("2000-01-01T00:00:00Z");
        fs::write(path, serde_json::to_vec(&activity).expect("json")).expect("activity");
        send_notification_fixture(&state, capability.trim(), "draft-wake-fixture").await;
        let candidate = crate::coordination::pending_notifications(&state.context)
            .expect("notifications")
            .pop()
            .expect("candidate");
        Self {
            temp,
            state,
            candidate,
        }
    }

    fn check_draft(&self) -> Result<(), CliError> {
        let record = load_session_record(&self.state.context, "beta")?;
        crate::send_guard::require_empty_composer(
            &self.state.context, &record, &self.state.tmux_bin,
        )
    }

    fn assert_no_input(&self, draft: &str) {
        assert_eq!(
            fs::read_to_string(self.temp.path().join("composer")).unwrap(),
            draft
        );
        assert!(
            !self.temp.path().join("submissions").exists(),
            "operator input must never be submitted"
        );
        let calls = fs::read_to_string(self.temp.path().join("calls")).unwrap();
        assert!(
            !calls.contains("paste-buffer"),
            "deferred wake must not paste"
        );
        assert!(
            !calls.contains("send-keys"),
            "deferred wake must not press a key"
        );
    }
}

mod notices {
    include!("mailbox_notice_terminal.rs");
}

mod stale_drafts {
    include!("mailbox_draft/stale_drafts.rs");
}

mod unresponsive {
    include!("mailbox_draft/unresponsive.rs");
}

mod residue_discard {
    include!("mailbox_draft/residue_discard.rs");
}

mod idle_failure {
    include!("mailbox_draft/idle_failure.rs");
}
