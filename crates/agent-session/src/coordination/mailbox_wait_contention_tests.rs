//! Scoped controls apply only to this module's waiter threads, never shipped code.
use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};

use pretty_assertions::assert_eq;
use serde_json::json;

use super::*;

const WATCHDOG: Duration = Duration::from_secs(30);
const TOKEN: &str = "synthetic-mailbox-wait-recipient-capability";
const BODY: &str = "MAILBOX-WAIT-CONTENTION-BODY-CANARY";

#[derive(Debug)]
enum Event {
    Polling(usize),
    Contended(usize),
}

struct Control {
    id: usize,
    acquisitions: usize,
    failures: usize,
    events: mpsc::Sender<Event>,
    start_polling: Option<mpsc::Receiver<()>>,
    after_failure: Option<mpsc::Receiver<()>>,
    lock_budget: Option<Duration>,
}

thread_local! {
    static CONTROL: RefCell<Option<Control>> = const { RefCell::new(None) };
}

struct Scope;
impl Scope {
    fn install(control: Control) -> Self {
        CONTROL.with_borrow_mut(|slot| {
            assert!(slot.is_none());
            *slot = Some(control);
        });
        Self
    }
}
impl Drop for Scope {
    fn drop(&mut self) {
        CONTROL.with_borrow_mut(|slot| *slot = None);
    }
}

pub(super) fn before_lock(timeout: Duration) -> Duration {
    CONTROL.with_borrow_mut(|slot| {
        let Some(control) = slot else { return timeout };
        control.acquisitions += 1;
        // Authentication is acquisition 1. Park only the first polling read,
        // after authentication and (for any-unread) watcher installation.
        if control.acquisitions == 2
            && let Some(start) = &control.start_polling
        {
            control.events.send(Event::Polling(control.id)).unwrap();
            start.recv_timeout(WATCHDOG).expect("release first poll");
        }
        if control.acquisitions > 1 {
            timeout.min(control.lock_budget.unwrap_or(timeout))
        } else {
            timeout
        }
    })
}

pub(super) fn lock_failed(error: &CliError) {
    CONTROL.with_borrow_mut(|slot| {
        let Some(control) = slot else { return };
        if (control.start_polling.is_some() && control.acquisitions < 2)
            || error.code() != "coordination-lock-timeout"
        {
            return;
        }
        control.failures += 1;
        control.events.send(Event::Contended(control.id)).unwrap();
        if control.failures == 1
            && let Some(resume) = &control.after_failure
        {
            resume
                .recv_timeout(WATCHDOG)
                .expect("release failed attempt");
        }
    });
}

struct Fixture {
    _temp: tempfile::TempDir,
    context: CliContext,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let context = CliContext {
            state_dir: temp.path().join("state"),
            host: None,
        };
        let record: SessionRecord = serde_json::from_value(json!({
            "schema_version": crate::SESSION_DOCUMENT_VERSION,
            "id": "recipient", "agent": "codex", "mode": "interactive",
            "cwd": "/fixture/repository", "tmux_session": "fixture-session",
            "created_at": "2030-01-01T00:00:00Z", "updated_at": "2030-01-01T00:00:00Z",
            "runtime": { "kind": "tmux", "tmux_session": "fixture-session", "generation": 1,
                "started_at": "2030-01-01T00:00:00Z", "launch_id": "recipient-incarnation" }
        }))
        .unwrap();
        crate::write_session_record(&context, &record).unwrap();
        broker::test_support::seed_live_broker(
            &context,
            "recipient",
            "recipient-incarnation",
            json!({}),
        );
        write_atomic(
            &capability_path(&context, "recipient", "recipient-incarnation"),
            TOKEN.as_bytes(),
            SECRET_FILE_MODE,
        )
        .unwrap();
        let mut locked = lock_registry(&context).unwrap();
        locked
            .registry
            .brokers
            .get_mut("recipient")
            .unwrap()
            .capability_digest = digest_bytes(TOKEN.as_bytes());
        locked.registry.messages.push(serde_json::from_value(json!({
            "schema_version": "agent-session.message.v1", "message_id": "storm-arrival",
            "sender_session_id": "sender", "sender_incarnation": "sender-incarnation",
            "recipient_session_id": "recipient", "recipient_incarnation": "recipient-incarnation",
            "state": "read", "revision": 1, "reply_to": null, "reply_depth": 0,
            "created_at": timestamp(now_epoch()), "created_at_epoch": now_epoch(),
            "created_at_epoch_millis": now_epoch() * 1000,
            "expires_at": timestamp(now_epoch() + 3600), "expires_at_epoch": now_epoch() + 3600,
            "body_bytes": BODY.len(), "body": BODY
        })).unwrap());
        locked.save().unwrap();
        drop(locked);
        Self {
            _temp: temp,
            context,
        }
    }

    fn args(&self, any_unread: bool, timeout: &str) -> cli::MessageWaitArgs {
        cli::MessageWaitArgs {
            session: "recipient".into(),
            message: (!any_unread).then(|| "storm-arrival".into()),
            if_revision: (!any_unread).then_some(1),
            any_unread,
            newer_than: None,
            timeout: timeout.into(),
            capability_file: Some(capability_path(
                &self.context,
                "recipient",
                "recipient-incarnation",
            )),
            format: nils_common::cli_contract::OutputFormat::Json,
        }
    }
}

fn receive(events: &mpsc::Receiver<Event>) -> Event {
    events.recv_timeout(WATCHDOG).expect("waiter checkpoint")
}

#[test]
fn mailbox_write_storm_all_twenty_waiters_observe_arrival() {
    let fixture = Fixture::new();
    let (events, checkpoints) = mpsc::channel();
    let (results, completed) = mpsc::channel();
    let mut starts = Vec::new();
    let mut resumes = Vec::new();
    let mut threads = Vec::new();
    for id in 0..20 {
        let (start, started) = mpsc::channel();
        let (resume, resumed) = mpsc::channel();
        starts.push(start);
        resumes.push(resume);
        let control = Control {
            id,
            acquisitions: 0,
            failures: 0,
            events: events.clone(),
            start_polling: Some(started),
            after_failure: Some(resumed),
            lock_budget: Some(Duration::ZERO),
        };
        let context = fixture.context.clone();
        let args = fixture.args(id % 2 == 0, "60s");
        let results = results.clone();
        threads.push(thread::spawn(move || {
            let _scope = Scope::install(control);
            let result = mailbox::wait_with_cancellation(&context, args, None);
            results.send((id, result)).unwrap();
        }));
    }
    let mut ready = std::collections::BTreeSet::new();
    for _ in 0..20 {
        let Event::Polling(id) = receive(&checkpoints) else {
            panic!("poll before contention")
        };
        assert!(ready.insert(id));
    }
    let mut holder = lock_registry(&fixture.context).unwrap();
    for start in starts {
        start.send(()).unwrap();
    }
    let mut contended = std::collections::BTreeSet::new();
    for _ in 0..20 {
        let Event::Contended(id) = receive(&checkpoints) else {
            panic!("failed acquisition")
        };
        assert!(contended.insert(id));
    }
    assert_eq!(contended, ready);
    // The real atomic registry writer publishes while holding the same flock.
    // No sleep selects a race: every waiter has already failed acquisition.
    holder.registry.messages[0].state = "unread".into();
    holder.registry.messages[0].revision = 2;
    holder.save().unwrap();
    drop(holder);
    for resume in resumes {
        resume.send(()).unwrap();
    }
    let mut failures = Vec::new();
    for _ in 0..20 {
        let (id, result) = completed.recv_timeout(WATCHDOG).unwrap();
        match result {
            Err(error) => failures.push((id, error.code().to_owned())),
            Ok(value) if id % 2 == 0 => {
                assert_eq!(value["status"], "arrived");
                assert_eq!(value["messages"][0]["message_id"], "storm-arrival");
                assert!(!value.to_string().contains(BODY));
            }
            Ok(value) => {
                assert_eq!(value["revision"], 2);
                assert_eq!(value["body"]["text"], BODY);
            }
        }
    }
    for thread in threads {
        thread.join().unwrap();
    }
    assert_eq!(
        failures,
        Vec::<(usize, String)>::new(),
        "every eligible waiter must recover"
    );
}

fn exhausted_deadline(any_unread: bool) {
    let fixture = Fixture::new();
    let (events, checkpoints) = mpsc::channel();
    let (start, started) = mpsc::channel();
    let (result, completed) = mpsc::channel();
    let context = fixture.context.clone();
    let args = fixture.args(any_unread, "1s");
    let thread = thread::spawn(move || {
        let _scope = Scope::install(Control {
            id: 0,
            acquisitions: 0,
            failures: 0,
            events,
            start_polling: Some(started),
            after_failure: None,
            lock_budget: None,
        });
        let outcome = mailbox::wait_with_cancellation(&context, args, None);
        let failures = CONTROL.with_borrow(|slot| slot.as_ref().unwrap().failures);
        result.send((outcome, failures)).unwrap();
    });
    assert!(matches!(receive(&checkpoints), Event::Polling(0)));
    let holder = lock_private_store(&fixture.context, REGISTRY_LOCK).unwrap();
    start.send(()).unwrap();
    // Keep the holder until completion: a reset deadline never finishes. This
    // watchdog detects a hang; it does not choose when contention or expiry occurs.
    let (outcome, attempts) = completed
        .recv_timeout(WATCHDOG)
        .expect("original deadline must exhaust");
    drop(holder);
    thread.join().unwrap();
    let error = outcome.unwrap_err().into_inner();
    assert_eq!(error.code, "wait-timeout");
    assert_eq!(error.exit_code, if any_unread { 69 } else { 1 });
    assert!(
        attempts > 1,
        "contention must retry without restarting the deadline"
    );
    if any_unread {
        assert_eq!(error.details.unwrap()["recovery"]["kind"], "same_cursor");
    }
}

#[test]
fn mailbox_revision_contention_preserves_original_deadline() {
    exhausted_deadline(false);
}

#[test]
fn mailbox_any_unread_contention_preserves_original_deadline() {
    exhausted_deadline(true);
}

#[test]
fn mailbox_contention_cancellation_is_not_retried_as_contention() {
    for any_unread in [false, true] {
        let fixture = Fixture::new();
        let (events, checkpoints) = mpsc::channel();
        let (start, started) = mpsc::channel();
        let (resume, resumed) = mpsc::channel();
        let (result, completed) = mpsc::channel();
        let cancelled = Arc::new(AtomicBool::new(false));
        let flag = cancelled.clone();
        let context = fixture.context.clone();
        let args = fixture.args(any_unread, "60s");
        let thread = thread::spawn(move || {
            let _scope = Scope::install(Control {
                id: 0,
                acquisitions: 0,
                failures: 0,
                events,
                start_polling: Some(started),
                after_failure: Some(resumed),
                lock_budget: Some(Duration::ZERO),
            });
            result
                .send(mailbox::wait_with_cancellation(&context, args, Some(&flag)))
                .unwrap();
        });
        assert!(matches!(receive(&checkpoints), Event::Polling(0)));
        let holder = lock_private_store(&fixture.context, REGISTRY_LOCK).unwrap();
        start.send(()).unwrap();
        assert!(matches!(receive(&checkpoints), Event::Contended(0)));
        cancelled.store(true, Ordering::Release);
        resume.send(()).unwrap();
        let error = completed.recv_timeout(WATCHDOG).unwrap().unwrap_err();
        drop(holder);
        thread.join().unwrap();
        assert_eq!(error.code(), "wait-cancelled");
    }
}

#[test]
fn mailbox_polling_preserves_non_contention_errors_without_retry() {
    for any_unread in [false, true] {
        for case in [
            "capability",
            "incarnation",
            "corrupt",
            "unavailable",
            "untrusted",
        ] {
            let fixture = Fixture::new();
            let (events, checkpoints) = mpsc::channel();
            let (start, started) = mpsc::channel();
            let (result, completed) = mpsc::channel();
            let context = fixture.context.clone();
            let args = fixture.args(any_unread, "60s");
            let thread = thread::spawn(move || {
                let _scope = Scope::install(Control {
                    id: 0,
                    acquisitions: 0,
                    failures: 0,
                    events,
                    start_polling: Some(started),
                    after_failure: None,
                    lock_budget: None,
                });
                let outcome = mailbox::wait_with_cancellation(&context, args, None);
                let acquisitions = CONTROL.with_borrow(|slot| slot.as_ref().unwrap().acquisitions);
                result.send((outcome, acquisitions)).unwrap();
            });
            assert!(matches!(receive(&checkpoints), Event::Polling(0)));
            let root = coordination_root(&fixture.context).unwrap();
            let expected = match case {
                "capability" => {
                    write_atomic(
                        &capability_path(&fixture.context, "recipient", "recipient-incarnation"),
                        b"synthetic-wrong-recipient-capability",
                        SECRET_FILE_MODE,
                    )
                    .unwrap();
                    "coordination-unauthorized"
                }
                "incarnation" => {
                    let mut locked = lock_registry(&fixture.context).unwrap();
                    locked
                        .registry
                        .brokers
                        .get_mut("recipient")
                        .unwrap()
                        .incarnation = "replacement".into();
                    locked.save().unwrap();
                    "coordination-unauthorized"
                }
                "corrupt" => {
                    fs::write(root.join(REGISTRY_FILE), b"{").unwrap();
                    "coordination-store-corrupt"
                }
                "unavailable" => {
                    fs::remove_file(root.join(REGISTRY_FILE)).unwrap();
                    fs::create_dir(root.join(REGISTRY_FILE)).unwrap();
                    "coordination-unavailable"
                }
                "untrusted" => {
                    fs::remove_file(root.join(REGISTRY_LOCK)).unwrap();
                    std::os::unix::fs::symlink(root.join(REGISTRY_FILE), root.join(REGISTRY_LOCK))
                        .unwrap();
                    "coordination-store-untrusted"
                }
                _ => unreachable!(),
            };
            start.send(()).unwrap();
            let (outcome, acquisitions) = completed.recv_timeout(WATCHDOG).unwrap();
            thread.join().unwrap();
            let error = outcome.unwrap_err();
            assert_eq!(error.code(), expected, "{case}, any_unread={any_unread}");
            assert_eq!(acquisitions, 2, "permanent errors must not retry");
            if matches!(case, "capability" | "incarnation") {
                assert_eq!(
                    error.details().unwrap()["reason_code"],
                    if case == "capability" {
                        "capability-mismatch"
                    } else {
                        "runtime-incarnation-mismatch"
                    }
                );
            }
        }
    }
}

#[test]
fn mailbox_authentication_recovers_from_registry_contention() {
    let fixture = Fixture::new();
    let (events, checkpoints) = mpsc::channel();
    thread::scope(|scope| {
        let mut holder = lock_registry(&fixture.context).unwrap();
        let mut resumes = Vec::new();
        let waiters = [(0, false), (1, true)].map(|(id, any_unread)| {
            let (resume, resumed) = mpsc::channel();
            resumes.push(resume);
            let control = Control {
                id,
                acquisitions: 0,
                failures: 0,
                events: events.clone(),
                start_polling: None,
                after_failure: Some(resumed),
                lock_budget: None,
            };
            let context = &fixture.context;
            let args = fixture.args(any_unread, "60s");
            scope.spawn(move || {
                let _scope = Scope::install(control);
                mailbox::wait_with_cancellation(context, args, None)
            })
        });

        let mut contended = std::collections::BTreeSet::new();
        for _ in &waiters {
            let Event::Contended(id) = receive(&checkpoints) else {
                panic!("authentication must contend before polling");
            };
            assert!(contended.insert(id));
        }
        holder.registry.messages[0].state = "unread".into();
        holder.registry.messages[0].revision = 2;
        holder.save().unwrap();
        drop(holder);
        for resume in resumes {
            resume.send(()).unwrap();
        }

        let [revision, unread] = waiters.map(|waiter| {
            waiter
                .join()
                .unwrap()
                .map_err(|error| error.code().to_owned())
        });
        let failures: Vec<_> = [&revision, &unread]
            .into_iter()
            .filter_map(|result| result.as_ref().err())
            .collect();
        assert!(
            failures.is_empty(),
            "revision and any-unread authentication must recover: {failures:?}"
        );
        assert_eq!(revision.unwrap()["revision"], 2);
        let unread = unread.unwrap();
        assert_eq!(unread["status"], "arrived");
        assert_eq!(unread["messages"][0]["message_id"], "storm-arrival");
        assert!(!unread.to_string().contains(BODY));
    });
}

fn authentication_deadline(any_unread: bool) {
    let fixture = Fixture::new();
    let (events, checkpoints) = mpsc::channel();
    let (result, completed) = mpsc::channel();
    thread::scope(|scope| {
        let holder = lock_private_store(&fixture.context, REGISTRY_LOCK).unwrap();
        let context = &fixture.context;
        let args = fixture.args(any_unread, "1s");
        scope.spawn(move || {
            let _scope = Scope::install(Control {
                id: 0,
                acquisitions: 0,
                failures: 0,
                events,
                start_polling: None,
                after_failure: None,
                lock_budget: None,
            });
            let outcome = mailbox::wait_with_cancellation(context, args, None);
            let failures = CONTROL.with_borrow(|slot| slot.as_ref().unwrap().failures);
            result.send((outcome, failures)).unwrap();
        });
        assert!(matches!(receive(&checkpoints), Event::Contended(0)));
        let (outcome, attempts) = completed.recv_timeout(WATCHDOG).expect("wait deadline");
        drop(holder);
        let error = outcome.unwrap_err().into_inner();
        assert_eq!(error.code, "wait-timeout");
        assert_eq!(error.exit_code, if any_unread { 69 } else { 1 });
        assert!(
            attempts > 1,
            "authentication must retry within the original budget"
        );
        if any_unread {
            assert_eq!(error.details.unwrap()["recovery"]["kind"], "same_cursor");
        }
    });
}

#[test]
fn mailbox_authentication_revision_preserves_original_deadline() {
    authentication_deadline(false);
}

#[test]
fn mailbox_authentication_any_unread_preserves_original_deadline() {
    authentication_deadline(true);
}
