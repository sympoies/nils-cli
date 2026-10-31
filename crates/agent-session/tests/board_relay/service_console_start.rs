//! Admitted-service Console starts (`docs/specs/session-coordination-v1.md`,
//! "Service Console starts"): `agent-session start --via-console --service …`
//! through `POST /coordination/services/console-start/v1`, here against a fake
//! aggregator and synthetic services, owners and machines.

use super::*;
use pretty_assertions::assert_eq;
use std::net::TcpStream;

const SERVICE: &str = "svc-a";
const GENERATION: &str = "gen-1";
const OWNER: &str = "owner-a";
const SERVICE_TOKEN: &str = "console-service-credential-0000000000000001";
const MAILBOX_TOKEN: &str = "mailbox-service-credential-00000000000000001";
const ROUTE: &str = "/coordination/services/console-start/v1";

fn at(offset_seconds: i64) -> String {
    jiff::Timestamp::from_second(now_epoch() as i64 + offset_seconds)
        .expect("timestamp")
        .to_string()
}

fn new_operation() -> String {
    uuid::Uuid::now_v7().to_string()
}

/// One admission document with a dedicated credential file.
struct Admission {
    file: PathBuf,
    credential: PathBuf,
}

impl Admission {
    fn new(fixture: &Fixture) -> Self {
        Self::with(fixture, SERVICE_TOKEN, json!({}))
    }

    fn with(fixture: &Fixture, token: &str, overrides: Value) -> Self {
        let credential = fixture.root.join("console-service.credential");
        private_file(&credential, token.as_bytes());
        let mut entry = json!({
            "service_id": SERVICE,
            "service_generation": GENERATION,
            "credential_file": credential,
            "owners": [OWNER],
            "machines": [MACHINE, "host-b"],
            "max_lifetime_seconds": 3600
        });
        for (key, value) in overrides.as_object().expect("overrides") {
            entry[key] = value.clone();
        }
        let file = fixture.root.join("console-start-services.json");
        private_file(
            &file,
            &serde_json::to_vec(&json!({
                "schema_version": "agent-session.console-start-services.v1",
                "services": [entry]
            }))
            .expect("admission"),
        );
        Self { file, credential }
    }

    fn serve(&self, fixture: &Fixture, aggregator: &Aggregator) -> Serve {
        fixture.serve(
            &["--console-start-services-file", self.file.to_str().unwrap()],
            Some(aggregator),
        )
    }
}

type Change<'a> = Box<dyn Fn(&mut Request) + 'a>;

/// The service CLI request; every field can be changed per case.
struct Request {
    service: String,
    generation: String,
    credential: PathBuf,
    owner: String,
    machine: String,
    expires_at: String,
    title: Option<String>,
}

impl Request {
    fn new(admission: &Admission) -> Self {
        Self {
            service: SERVICE.into(),
            generation: GENERATION.into(),
            credential: admission.credential.clone(),
            owner: OWNER.into(),
            machine: MACHINE.into(),
            expires_at: at(600),
            title: None,
        }
    }

    fn run(&self, fixture: &Fixture, operation: &str) -> CmdOutput {
        let cwd = fixture.root.to_string_lossy().to_string();
        let credential = self.credential.to_string_lossy().to_string();
        let mut args = vec![
            "--agent",
            "claude",
            "--cwd",
            &cwd,
            "--service",
            &self.service,
            "--service-generation",
            &self.generation,
            "--credential-file",
            &credential,
            "--owner",
            &self.owner,
            "--machine",
            &self.machine,
            "--expires-at",
            &self.expires_at,
            "--operation-id",
            operation,
            "--format",
            "json",
        ];
        if let Some(title) = &self.title {
            args.extend(["--title", title]);
        }
        fixture.start_via_console(&args, false)
    }

    fn reconcile(&self, fixture: &Fixture, operation: &str) -> CmdOutput {
        let credential = self.credential.to_string_lossy().to_string();
        fixture.start_via_console(
            &[
                "--service",
                &self.service,
                "--service-generation",
                &self.generation,
                "--credential-file",
                &credential,
                "--owner",
                &self.owner,
                "--operation-id",
                operation,
                "--reconcile",
                "--format",
                "json",
            ],
            false,
        )
    }
}

fn created(aggregator: &Aggregator, id: &str) {
    aggregator.reply_json(
        201,
        &json!({"ok":true,"data":{"session":{
            "id": id, "agent": "claude", "session_incarnation": format!("{id}-incarnation")
        }}}),
    );
}

fn creates(aggregator: &Aggregator) -> usize {
    aggregator
        .seen()
        .iter()
        .filter(|seen| seen.path() == "/api/coordination/sessions/v1")
        .count()
}

fn post(serve: &Serve, bearer: &str, body: &Value) -> (u16, Value) {
    let response = reqwest::blocking::Client::new()
        .post(format!("http://{}{ROUTE}", serve.addr))
        .bearer_auth(bearer)
        .json(body)
        .send()
        .expect("route response");
    (
        response.status().as_u16(),
        response.json().expect("route json"),
    )
}

#[test]
fn service_start_creates_one_console_root_and_returns_the_owner_receipt() {
    let fixture = Fixture::new();
    let aggregator = Aggregator::start();
    let admission = Admission::new(&fixture);
    let _serve = admission.serve(&fixture, &aggregator);
    let mut request = Request::new(&admission);
    request.expires_at = at(3);
    let operation = new_operation();
    created(&aggregator, "service-child");

    // No managed session, capability or delegation is present.
    let output = request.run(&fixture, &operation);
    assert_eq!(output.code, 0, "{}", output.stdout_text());
    let data = output.stdout_json()["data"].clone();
    let receipt = json!({
        "schema_version": "agent-session.console-start-service-receipt.v1",
        "service_id": SERVICE,
        "service_generation": GENERATION,
        "owner": OWNER,
        "machine": MACHINE,
        "operation_id": operation,
        "state": "created",
        "safe_to_retry": false,
        "session": {
            "id": "service-child",
            "agent": "claude",
            "session_incarnation": "service-child-incarnation"
        }
    });
    assert_eq!(data["receipt"], receipt);
    assert_eq!(data["session"]["id"], "service-child");

    let seen = aggregator.seen();
    assert_eq!(seen.len(), 1, "{seen:?}");
    assert_eq!(seen[0].path(), "/api/coordination/sessions/v1");
    assert_eq!(
        seen[0].authorization.as_deref(),
        Some(format!("Bearer {RELAY_TOKEN}").as_str())
    );
    assert_eq!(seen[0].operation_id.as_deref(), Some(operation.as_str()));
    let body = seen[0].body.clone().expect("create body");
    assert_eq!(body["source_service_id"], SERVICE);
    assert_eq!(body["source_service_generation"], GENERATION);
    assert_eq!(body["owner"], OWNER);
    assert_eq!(body["machine"], MACHINE);
    for absent in ["source_session_id", "source_incarnation"] {
        assert!(body.get(absent).is_none(), "{body}");
    }
    assert_eq!(body["session"]["agent"], "claude");
    assert_eq!(
        body["session"]["lineage"]["starter"],
        json!({"kind":"console","via":"console"})
    );
    assert_eq!(body["session"]["lineage"]["parent"], Value::Null);

    // The same admitted request replays the stored outcome without a second create.
    let replay = request.run(&fixture, &operation);
    assert_eq!(replay.code, 0, "{}", replay.stdout_text());
    let replay = replay.stdout_json()["data"].clone();
    assert_eq!(replay["receipt"], receipt);
    assert_eq!(replay["session"]["id"], "service-child");
    assert_eq!(creates(&aggregator), 1);

    // After the request expiry the stored outcome is still returned, never recreated.
    std::thread::sleep(Duration::from_secs(4));
    let replay = request.run(&fixture, &operation);
    assert_eq!(replay.code, 0, "{}", replay.stdout_text());
    assert_eq!(replay.stdout_json()["data"]["receipt"], receipt);
    assert_eq!(creates(&aggregator), 1);
}

#[test]
fn service_start_changed_replay_conflicts_without_a_second_create() {
    let fixture = Fixture::new();
    let aggregator = Aggregator::start();
    let admission = Admission::new(&fixture);
    let _serve = admission.serve(&fixture, &aggregator);
    let mut request = Request::new(&admission);
    let operation = new_operation();
    created(&aggregator, "service-child");
    assert_eq!(request.run(&fixture, &operation).code, 0);

    request.title = Some("another request".into());
    let output = request.run(&fixture, &operation);
    assert_eq!(output.code, 64, "{}", output.stdout_text());
    assert_eq!(
        error_of(&output)["code"],
        "console-start-operation-conflict"
    );
    assert_eq!(creates(&aggregator), 1);
}

#[test]
fn service_start_refusals_leave_no_receipt_and_make_no_aggregator_call() {
    let fixture = Fixture::new();
    let aggregator = Aggregator::start();
    let admission = Admission::new(&fixture);
    let _serve = admission.serve(&fixture, &aggregator);
    let wrong_credential = fixture.root.join("wrong.credential");
    private_file(
        &wrong_credential,
        b"console-service-credential-wrong-0000000001",
    );

    let cases: Vec<(&str, Change<'_>, i32, &str)> = vec![
        (
            "wrong credential",
            Box::new(|r| r.credential = wrong_credential.clone()),
            65,
            "console-start-service-unauthorized",
        ),
        (
            "stale generation",
            Box::new(|r| r.generation = "gen-0".into()),
            65,
            "console-start-service-unauthorized",
        ),
        (
            "unknown service",
            Box::new(|r| r.service = "svc-other".into()),
            65,
            "console-start-service-unauthorized",
        ),
        (
            "owner not admitted",
            Box::new(|r| r.owner = "owner-b".into()),
            65,
            "console-start-service-owner-denied",
        ),
        (
            "target not admitted",
            Box::new(|r| r.machine = "host-z".into()),
            65,
            "console-start-service-target-denied",
        ),
        (
            "expired request",
            Box::new(|r| r.expires_at = at(-60)),
            65,
            "console-start-service-expired",
        ),
        (
            "lifetime beyond the admitted maximum",
            Box::new(|r| r.expires_at = at(7200)),
            65,
            "console-start-service-expired",
        ),
    ];
    for (name, change, code, expected) in cases {
        let mut request = Request::new(&admission);
        change(&mut request);
        let operation = new_operation();
        let output = request.run(&fixture, &operation);
        assert_eq!(output.code, code, "{name}: {}", output.stdout_text());
        assert_eq!(error_of(&output)["code"], expected, "{name}");
        // A refused operation id is still unknown to reconciliation.
        let probe = Request::new(&admission).reconcile(&fixture, &operation);
        assert_eq!(
            error_of(&probe)["code"],
            "console-start-operation-not-found",
            "{name}"
        );
    }
    assert_eq!(aggregator.seen().len(), 0);
}

#[test]
fn service_start_credentials_are_never_shared_with_other_authorities() {
    let fixture = Fixture::new();
    let aggregator = Aggregator::start();

    // Disabled: no admission document is configured.
    let admission = Admission::new(&fixture);
    {
        let _serve = fixture.serve(&[], Some(&aggregator));
        let output = Request::new(&admission).run(&fixture, &new_operation());
        assert_eq!(
            error_of(&output)["code"],
            "console-start-service-unauthorized"
        );
    }

    // The operator bearer cannot be admitted as a service credential.
    let operator = Admission::with(&fixture, OPERATOR, json!({}));
    {
        let _serve = operator.serve(&fixture, &aggregator);
        let output = Request::new(&operator).run(&fixture, &new_operation());
        assert_eq!(
            error_of(&output)["code"],
            "console-start-service-unauthorized"
        );
    }

    // A mailbox service credential grants no Console start.
    let shared = Admission::with(&fixture, MAILBOX_TOKEN, json!({}));
    let mailbox = fixture.root.join("mailbox-services.json");
    private_file(
        &mailbox,
        &serde_json::to_vec(&json!({
            "schema_version": "agent-session.mailbox-services.v1",
            "services": [{
                "service_id": "reporter",
                "service_generation": GENERATION,
                "credential_file": shared.credential
            }]
        }))
        .unwrap(),
    );
    {
        let _serve = fixture.serve(
            &[
                "--console-start-services-file",
                shared.file.to_str().unwrap(),
                "--mailbox-services-file",
                mailbox.to_str().unwrap(),
            ],
            Some(&aggregator),
        );
        let output = Request::new(&shared).run(&fixture, &new_operation());
        assert_eq!(
            error_of(&output)["code"],
            "console-start-service-unauthorized"
        );
    }
    assert_eq!(aggregator.seen().len(), 0);
}

#[test]
fn service_route_accepts_only_the_service_credential_and_a_closed_request() {
    let fixture = Fixture::new();
    let aggregator = Aggregator::start();
    let admission = Admission::new(&fixture);
    let serve = admission.serve(&fixture, &aggregator);
    let valid = |operation: &str| {
        json!({
            "service_id": SERVICE,
            "service_generation": GENERATION,
            "owner": OWNER,
            "machine": MACHINE,
            "expires_at": at(600),
            "operation_id": operation,
            "session": {"agent": "claude", "cwd": fixture.root}
        })
    };

    // A managed session capability is not a service credential, and the reverse.
    let (status, body) = post(&serve, CAPABILITY, &valid(&new_operation()));
    assert_eq!(
        (status, body["error"]["code"].as_str()),
        (401, Some("console-start-service-unauthorized"))
    );
    let session_route = reqwest::blocking::Client::new()
        .post(format!(
            "http://{}/sessions/{SESSION}/console-start/v1",
            serve.addr
        ))
        .bearer_auth(SERVICE_TOKEN)
        .json(&json!({"session": {"agent": "claude"}}))
        .send()
        .expect("session route");
    assert_eq!(session_route.status().as_u16(), 401);

    let mut cases = Vec::new();
    for (name, edit) in [
        (
            "unknown member",
            Box::new(|b: &mut Value| b["principal"] = json!("other")) as Box<dyn Fn(&mut Value)>,
        ),
        (
            "session source member",
            Box::new(|b| b["session"]["source_session_id"] = json!(SESSION)),
        ),
        (
            "lineage",
            Box::new(|b| b["session"]["lineage"] = json!({"parent": {}})),
        ),
        ("work", Box::new(|b| b["session"]["work"] = json!({}))),
        (
            "role",
            Box::new(|b| b["session"]["role"] = json!("coordinator")),
        ),
        (
            "resume",
            Box::new(|b| b["session"]["resume"] = json!("prior")),
        ),
        ("origin", Box::new(|b| b["session"]["origin"] = json!({}))),
        (
            "missing machine",
            Box::new(|b| {
                b.as_object_mut().unwrap().remove("machine");
            }),
        ),
        (
            "missing expiry",
            Box::new(|b| {
                b.as_object_mut().unwrap().remove("expires_at");
            }),
        ),
        (
            "malformed expiry",
            Box::new(|b| b["expires_at"] = json!("tomorrow")),
        ),
        (
            "operation id",
            Box::new(|b| b["operation_id"] = json!("not-a-uuid")),
        ),
        (
            "reconcile with a session",
            Box::new(|b| b["reconcile"] = json!(true)),
        ),
    ] {
        let mut body = valid(&new_operation());
        edit(&mut body);
        cases.push((name, body));
    }
    for (name, body) in cases {
        let (status, answer) = post(&serve, SERVICE_TOKEN, &body);
        assert_eq!(
            (status, answer["error"]["code"].as_str()),
            (400, Some("console-start-service-invalid")),
            "{name}: {answer}"
        );
    }
    assert_eq!(creates(&aggregator), 0);
}

#[test]
fn service_start_uncertain_result_reconciles_without_a_second_create() {
    let fixture = Fixture::new();
    let aggregator = Aggregator::start();
    let admission = Admission::with(
        &fixture,
        SERVICE_TOKEN,
        json!({"owners": [OWNER, "owner-b"]}),
    );
    let _serve = admission.serve(&fixture, &aggregator);
    let request = Request::new(&admission);
    let operation = new_operation();

    // The aggregator answers with an unreadable body: the outcome is unknown.
    aggregator.reply(String::new());
    let output = request.run(&fixture, &operation);
    let error = error_of(&output);
    assert_eq!(error["code"], "console-start-unavailable");
    assert_eq!(error["details"]["recovery"]["safe_to_retry"], false);
    assert!(
        error["hint"]
            .as_str()
            .unwrap()
            .contains("--credential-file"),
        "the reconcile hint repeats the service options: {error}"
    );
    assert_eq!(creates(&aggregator), 1);

    // Repeating the request never creates a candidate; it reports the open operation.
    let board = |candidates: Vec<Value>| {
        json!({
            "schema_version": "agent-session.board-view.v1",
            "records": candidates
        })
    };
    let mut mine = record(MACHINE, "candidate-mine", Some("inc"), true);
    mine["lineage"] = json!({"parent": null});
    mine["console_owner"] = json!(OWNER);
    let mut other = record(MACHINE, "candidate-other", Some("inc"), true);
    other["lineage"] = json!({"parent": null});
    other["console_owner"] = json!("owner-b");
    // A record without lineage is not known to be a root.
    let mut unrooted = record(MACHINE, "candidate-unrooted", Some("inc"), true);
    unrooted.as_object_mut().unwrap().remove("lineage");
    unrooted["console_owner"] = json!(OWNER);
    aggregator.reply_json(200, &board(vec![mine, other, unrooted]));
    let replay = request.run(&fixture, &operation);
    assert_eq!(replay.code, 0, "{}", replay.stdout_text());
    let data = replay.stdout_json()["data"].clone();
    assert_eq!(data["receipt"]["state"], "ambiguous");
    assert_eq!(data["receipt"]["owner"], OWNER);
    assert_eq!(data["receipt"]["session"], Value::Null);
    assert_eq!(data["reconciliation"]["safe_to_retry"], false);
    assert_eq!(
        data["reconciliation"]["candidates"],
        json!([{"session_id": "candidate-mine", "machine": MACHINE}])
    );
    assert_eq!(creates(&aggregator), 1);

    // The scan is scoped to the service and owner, never to a session.
    let scan = aggregator
        .seen()
        .into_iter()
        .find(|seen| seen.path() == "/api/coordination/board/v1")
        .expect("board scan");
    let query = scan.query();
    for (key, value) in [
        ("source_service_id", SERVICE),
        ("source_service_generation", GENERATION),
        ("owner", OWNER),
        ("machine", MACHINE),
    ] {
        assert!(
            query.contains(&(key.to_string(), value.to_string())),
            "{key}: {query:?}"
        );
    }
    assert!(
        query
            .iter()
            .all(|(key, _)| !key.starts_with("source_session"))
    );

    // Reconciliation belongs to the service and owner that started the operation.
    let mut stranger = Request::new(&admission);
    stranger.owner = "owner-b".into();
    let output = stranger.reconcile(&fixture, &operation);
    assert_eq!(
        error_of(&output)["code"],
        "console-start-operation-not-found"
    );
}

#[test]
fn service_start_options_are_complete_and_never_name_a_parent_or_resume() {
    let fixture = Fixture::new();
    let admission = Admission::new(&fixture);
    let credential = admission.credential.to_string_lossy().to_string();
    let operation = new_operation();
    let complete = [
        "--service",
        SERVICE,
        "--service-generation",
        GENERATION,
        "--credential-file",
        &credential,
        "--owner",
        OWNER,
        "--operation-id",
        &operation,
        "--agent",
        "claude",
        "--format",
        "json",
    ];
    let without = |flag: &str| -> Vec<&str> {
        let at = complete.iter().position(|arg| *arg == flag).unwrap();
        let mut args = complete.to_vec();
        args.drain(at..at + 2);
        args
    };
    let mut cases = vec![
        without("--owner"),
        without("--credential-file"),
        without("--service-generation"),
    ];
    for extra in [
        vec!["--no-parent"],
        vec!["--role", "coordinator"],
        vec!["--issue", "owner/repo#1"],
        vec!["--no-inherit-work"],
        vec!["--resume", "prior"],
    ] {
        let mut args = complete.to_vec();
        args.extend(extra);
        cases.push(args);
    }
    for args in cases {
        // Each is refused by the parser, before any credential or daemon is touched.
        let output = fixture.start_via_console(&args, false);
        assert_eq!(output.code, 64, "{args:?}: {}", output.stdout_text());
        assert_eq!(error_of(&output)["code"], "parse-error", "{args:?}");
    }
}

fn valid_body(fixture: &Fixture, operation: &str) -> Value {
    json!({
        "service_id": SERVICE,
        "service_generation": GENERATION,
        "owner": OWNER,
        "machine": MACHINE,
        "expires_at": at(600),
        "operation_id": operation,
        "session": {"agent": "claude", "cwd": fixture.root}
    })
}

/// An aggregator that accepts connections and never answers, so a start stays in flight.
struct Stalled {
    aggregator: Aggregator,
    held: Arc<Mutex<Vec<TcpStream>>>,
}

impl Stalled {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind stalled aggregator");
        let url = format!("http://{}", listener.local_addr().expect("stalled addr"));
        let held = Arc::new(Mutex::new(Vec::new()));
        let keep = held.clone();
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                keep.lock().expect("held streams").push(stream);
            }
        });
        Self {
            aggregator: Aggregator {
                url,
                state: Arc::default(),
            },
            held,
        }
    }

    fn connections(&self) -> usize {
        self.held.lock().expect("held streams").len()
    }
}

#[test]
fn service_route_holds_no_worker_for_a_caller_that_never_finishes_a_request() {
    let fixture = Fixture::new();
    let aggregator = Aggregator::start();
    let admission = Admission::new(&fixture);
    let serve = admission.serve(&fixture, &aggregator);

    // Eight local processes send any bearer and then stall their bodies.
    let stalled: Vec<TcpStream> = (0..8)
        .map(|_| {
            let mut stream = TcpStream::connect(serve.addr).expect("connect");
            write!(
                stream,
                "POST {ROUTE} HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer anything\r\nContent-Length: 1000\r\n\r\n{{",
                serve.addr
            )
            .expect("partial request");
            stream
        })
        .collect();
    thread::sleep(Duration::from_millis(500));

    // The admitted service is still served.
    created(&aggregator, "service-child");
    let (status, body) = post(
        &serve,
        SERVICE_TOKEN,
        &valid_body(&fixture, &new_operation()),
    );
    assert_eq!(
        (status, body["receipt"]["state"].as_str()),
        (201, Some("created")),
        "{body}"
    );
    drop(stalled);
}

#[test]
fn service_route_keeps_its_bound_while_a_start_outlives_its_client() {
    let fixture = Fixture::new();
    let stalled = Stalled::start();
    let admission = Admission::new(&fixture);
    let serve = admission.serve(&fixture, &stalled.aggregator);

    // Eight starts reach the aggregator, then their clients disconnect.
    let clients: Vec<_> = (0..8)
        .map(|_| {
            let url = format!("http://{}{ROUTE}", serve.addr);
            let body = valid_body(&fixture, &new_operation());
            thread::spawn(move || {
                let _ = reqwest::blocking::Client::builder()
                    .timeout(Duration::from_secs(1))
                    .build()
                    .expect("client")
                    .post(url)
                    .bearer_auth(SERVICE_TOKEN)
                    .json(&body)
                    .send();
            })
        })
        .collect();
    for client in clients {
        client.join().expect("client thread");
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    while stalled.connections() < 8 {
        assert!(
            Instant::now() < deadline,
            "starts did not reach the aggregator"
        );
        thread::sleep(Duration::from_millis(20));
    }
    thread::sleep(Duration::from_millis(300));

    // The eight starts still run, so a ninth is refused rather than started.
    let response = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .expect("client")
        .post(format!("http://{}{ROUTE}", serve.addr))
        .bearer_auth(SERVICE_TOKEN)
        .json(&valid_body(&fixture, &new_operation()))
        .send()
        .expect("the ninth start is refused promptly");
    let body: Value = response.json().expect("refusal json");
    assert_eq!(body["error"]["code"], "console-start-unavailable", "{body}");
    assert_eq!(stalled.connections(), 8);
}

#[test]
fn service_start_typed_aggregator_rejection_records_a_retryable_not_created() {
    let fixture = Fixture::new();
    let aggregator = Aggregator::start();
    let admission = Admission::new(&fixture);
    let _serve = admission.serve(&fixture, &aggregator);
    let request = Request::new(&admission);

    // A typed refusal of the admitted service, owner or target, and a usage-class rejection.
    for (status, code, exit) in [
        (403, "console-start-service-owner-denied", 65),
        (400, "invalid-request", 64),
    ] {
        let operation = new_operation();
        aggregator.reply_json(
            status,
            &json!({"ok":false,"error":{"code":code,"message":"refused"}}),
        );
        let output = request.run(&fixture, &operation);
        assert_eq!(output.code, exit, "{code}: {}", output.stdout_text());
        let error = error_of(&output);
        assert_eq!(error["code"], code);

        // The operation is recorded as not created and may be retried.
        let probe = request.reconcile(&fixture, &operation);
        assert_eq!(probe.code, 0, "{code}: {}", probe.stdout_text());
        let data = probe.stdout_json()["data"].clone();
        assert_eq!(data["receipt"]["state"], "not_created", "{code}");
        assert_eq!(data["receipt"]["safe_to_retry"], true, "{code}");
        assert_eq!(data["receipt"]["session"], Value::Null, "{code}");
    }
}
