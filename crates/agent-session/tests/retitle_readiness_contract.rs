//! Black-box contract for the retitle readiness projections.
//!
//! `GET /retitle/readiness` (v2), `GET /retitle/v3/readiness`, and
//! `GET /sessions/{id}/retitle-v3/readiness` are read by clients outside this
//! crate. Each projection has a closed key allowlist and a content-free value
//! vocabulary; these tests pin both through the real `agent-session serve`
//! surface so an added key or a leaked credential, path, or raw provider error
//! fails here instead of in a downstream mirror.

use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use nils_test_support::bin;
use pretty_assertions::assert_eq;
use serde_json::{Value, json};

const TOKEN: &str = "retitle-readiness-contract-bearer";
const MACHINE: &str = "retitle-readiness-contract-machine";
const SESSION_ID: &str = "retitle-readiness-contract";
const PROVIDERLESS_SESSION_ID: &str = "retitle-readiness-no-history";
const PROVIDER_SESSION_ID: &str = "provider-retitle-readiness-contract";
const INCARNATION: &str = "launch-retitle-readiness-contract";
const API_KEY_ENV: &str = "RETITLE_READINESS_CONTRACT_API_KEY";
const API_KEY_CANARY: &str = "sk-readiness-contract-api-key-canary";
const CONFIG_CANARY: &str = "readiness-config-secret-canary";
const MODEL_CANARY: &str = "sk-readiness-credential-model-canary";
const PATH_CANARY: &str = "readiness-private-path-canary";
const ASSISTANT_CANARY: &str = "readiness-assistant-output-canary";
const BROKER_LABEL_CANARY: &str = "readiness-broker-label-canary";
const BROKER_ACCOUNT: &str = "readiness-account";
/// Broker plan metadata is free text, so the fixture uses a realistic value
/// rather than a token-shaped one.
const BROKER_PLAN: &str = "Team Plus";

/// Every key `/retitle/readiness` v2 may emit.
const V2_ALLOWED_KEYS: &[&str] = &[
    "account",
    "capability",
    "context_capabilities",
    "model_label",
    "next_action",
    "plan",
    "provider_kind",
    "reason_code",
    "schema_version",
    "status",
];
/// Keys `/retitle/readiness` v2 always emits.
const V2_REQUIRED_KEYS: &[&str] = &[
    "capability",
    "context_capabilities",
    "next_action",
    "reason_code",
    "schema_version",
    "status",
];
const V2_CONTEXT_CAPABILITY_KEYS: &[&str] = &["claude", "codex", "hermes"];
const V2_CONTEXT_CAPABILITY_VALUES: &[&str] = &["provider_transcript", "unavailable"];
const V2_STATUSES: &[&str] = &["ready", "degraded", "unavailable"];
const V2_REASONS: &[&str] = &[
    "ready",
    "provider_not_configured",
    "config_invalid",
    "account_broker_unavailable",
    "account_missing",
    "api_key_missing",
    "provider_command_unavailable",
    "fallback_ready",
    "legacy_command_provider", // stale-audit: keep-contract (stable v2 reason code)
];
const V2_ACTIONS: &[&str] = &[
    "none",
    "configure_provider",
    "configure_account_broker",
    "select_account",
    "set_api_key",
    "install_provider_command",
    "restore_primary",
    "migrate_provider",
];
const PROVIDER_KINDS: &[&str] = &["codex_subscription", "openai_compatible", "command"];

/// Every key `/retitle/v3/readiness` may emit.
const V3_MACHINE_KEYS: &[&str] = &[
    "capability",
    "next_action",
    "reason_code",
    "schema_version",
    "status",
];
/// `(status, reason_code, next_action)` rows of the machine readiness table.
const V3_MACHINE_ROWS: &[(&str, &str, &str)] = &[
    ("ready", "ready", "none"),
    (
        "degraded",
        "provider_unavailable_memory_supported",
        "use_cached_memory_or_restore_provider",
    ),
    (
        "unavailable",
        "retitle_v3_unavailable",
        "restore_retitle_v3",
    ),
];

/// Every key `/sessions/{id}/retitle-v3/readiness` may emit.
const V3_SESSION_ALLOWED_KEYS: &[&str] = &[
    "capability",
    "context_status",
    "cursor_fence_hash",
    "memory_revision",
    "next_action",
    "pending_operation",
    "provider_status",
    "reason_code",
    "schema_version",
    "status",
    "title_status",
    "usable_memory",
];
const V3_SESSION_STATUSES: &[&str] = &["ready", "catching_up", "stale", "degraded", "unavailable"];
const V3_PROVIDER_STATUSES: &[&str] = &["ready", "unavailable"];
const V3_TITLE_STATUSES: &[&str] = &["current", "stale", "degraded_cached", "missing"];
const V3_SESSION_REASONS: &[&str] = &[
    "ready",
    "memory_not_initialized",
    "history_catching_up",
    "history_advanced",
    "history_stale",
    "history_read_degraded",
    "degraded_cached",
    "provider_history_unavailable",
    "memory_unavailable",
];
const V3_SESSION_ACTIONS: &[&str] = &[
    "none",
    "refresh_memory",
    "use_cached_memory_or_retry",
    "restore_provider_history",
];

#[test]
fn v2_readiness_keys_and_values_stay_inside_the_published_allowlist() {
    let fixture = Fixture::new();
    let root = fixture.root.to_string_lossy().into_owned();
    let command = |argv: &Path| json!({"provider": "command", "argv": [argv], "timeout_ms": 1000});
    let openai = |model: &str| {
        json!({
            "provider": "openai_compatible",
            "base_url": format!("http://127.0.0.1:9/{PATH_CANARY}/v1"),
            "model": model,
            "api_key_env": API_KEY_ENV,
            "timeout_ms": 1000
        })
    };
    let mut fallback = command(&fixture.missing_provider_bin);
    fallback["fallback"] = openai("readiness-contract-model");
    let malformed = format!(
        "{{\"provider\":\"command\",\"argv\":[\"{root}/{PATH_CANARY}\"],\"secret\":\"{CONFIG_CANARY}\""
    );
    let codex = |account: &str| {
        json!({
            "provider": "codex_subscription",
            "account": account,
            "codex_bin": fixture.provider_bin,
            "timeout_ms": 1000
        })
        .to_string()
    };

    struct Case {
        name: &'static str,
        config: Option<String>,
        api_key: bool,
        broker: bool,
        expected: (&'static str, &'static str, &'static str),
        provider_kind: Option<&'static str>,
        model_label: Option<&'static str>,
        account: Option<&'static str>,
        plan: Option<&'static str>,
    }
    let base = || Case {
        name: "",
        config: None,
        api_key: false,
        broker: false,
        expected: ("", "", ""),
        provider_kind: None,
        model_label: None,
        account: None,
        plan: None,
    };
    let cases = [
        Case {
            name: "not configured",
            expected: (
                "unavailable",
                "provider_not_configured",
                "configure_provider",
            ),
            ..base()
        },
        Case {
            name: "unparseable config",
            config: Some(malformed),
            api_key: true,
            expected: ("unavailable", "config_invalid", "configure_provider"),
            ..base()
        },
        Case {
            name: "compatibility command",
            config: Some(command(&fixture.provider_bin).to_string()),
            expected: (
                "degraded",
                "legacy_command_provider", // stale-audit: keep-contract (stable v2 reason code)
                "migrate_provider",
            ),
            provider_kind: Some("command"),
            ..base()
        },
        Case {
            name: "missing command",
            config: Some(command(&fixture.missing_provider_bin).to_string()),
            expected: (
                "unavailable",
                "provider_command_unavailable",
                "install_provider_command",
            ),
            ..base()
        },
        Case {
            name: "openai compatible",
            config: Some(openai("readiness-contract-model").to_string()),
            api_key: true,
            expected: ("ready", "ready", "none"),
            provider_kind: Some("openai_compatible"),
            model_label: Some("readiness-contract-model"),
            ..base()
        },
        Case {
            name: "credential-shaped model label",
            config: Some(openai(MODEL_CANARY).to_string()),
            api_key: true,
            expected: ("ready", "ready", "none"),
            provider_kind: Some("openai_compatible"),
            ..base()
        },
        Case {
            name: "missing api key",
            config: Some(openai("readiness-contract-model").to_string()),
            expected: ("unavailable", "api_key_missing", "set_api_key"),
            ..base()
        },
        Case {
            name: "fallback ready",
            config: Some(fallback.to_string()),
            api_key: true,
            expected: ("degraded", "fallback_ready", "restore_primary"),
            provider_kind: Some("openai_compatible"),
            model_label: Some("readiness-contract-model"),
            ..base()
        },
        Case {
            name: "codex subscription",
            config: Some(codex(BROKER_ACCOUNT)),
            broker: true,
            expected: ("ready", "ready", "none"),
            provider_kind: Some("codex_subscription"),
            account: Some(BROKER_ACCOUNT),
            plan: Some(BROKER_PLAN),
            ..base()
        },
        Case {
            name: "codex subscription account missing",
            config: Some(codex("absent-account")),
            broker: true,
            expected: ("unavailable", "account_missing", "select_account"),
            ..base()
        },
        Case {
            name: "codex subscription without broker",
            config: Some(codex(BROKER_ACCOUNT)),
            expected: (
                "unavailable",
                "account_broker_unavailable",
                "configure_account_broker",
            ),
            ..base()
        },
    ];

    for case in cases {
        let _server =
            ServeProcess::spawn(&fixture, case.config.as_deref(), case.api_key, case.broker);
        let response = fixture.request("GET", "/retitle/readiness");
        assert_eq!(
            response.status, 200,
            "case={} body={}",
            case.name, response.body
        );
        assert_outer_envelope(&response.body);
        let readiness = &response.body["data"]["retitle"];

        let keys = object_keys(readiness);
        let unexpected = keys
            .difference(&set(V2_ALLOWED_KEYS))
            .cloned()
            .collect::<Vec<_>>();
        assert!(
            unexpected.is_empty(),
            "case={}: v2 readiness emitted keys outside the allowlist: {unexpected:?}",
            case.name
        );
        let mut expected_keys = set(V2_REQUIRED_KEYS);
        expected_keys.extend(case.provider_kind.map(|_| "provider_kind".to_string()));
        expected_keys.extend(case.model_label.map(|_| "model_label".to_string()));
        expected_keys.extend(case.account.map(|_| "account".to_string()));
        expected_keys.extend(case.plan.map(|_| "plan".to_string()));
        assert_eq!(keys, expected_keys, "case={}", case.name);

        assert_eq!(
            readiness["schema_version"],
            "agent-session.session-retitle.readiness.v2"
        );
        assert_eq!(readiness["capability"], "agent-session.session-retitle.v2");
        let (status, reason, action) = case.expected;
        assert_eq!(
            (
                readiness["status"].as_str(),
                readiness["reason_code"].as_str(),
                readiness["next_action"].as_str(),
            ),
            (Some(status), Some(reason), Some(action)),
            "case={}",
            case.name
        );
        assert_member(readiness, "status", V2_STATUSES);
        assert_member(readiness, "reason_code", V2_REASONS);
        assert_member(readiness, "next_action", V2_ACTIONS);
        if readiness.get("provider_kind").is_some() {
            assert_member(readiness, "provider_kind", PROVIDER_KINDS);
        }
        assert_eq!(readiness["provider_kind"].as_str(), case.provider_kind);
        assert_eq!(readiness["model_label"].as_str(), case.model_label);
        assert_eq!(readiness["account"].as_str(), case.account);
        assert_eq!(readiness["plan"].as_str(), case.plan);

        let capabilities = &readiness["context_capabilities"];
        assert_eq!(
            object_keys(capabilities),
            set(V2_CONTEXT_CAPABILITY_KEYS),
            "case={}",
            case.name
        );
        for key in V2_CONTEXT_CAPABILITY_KEYS {
            assert_member(capabilities, key, V2_CONTEXT_CAPABILITY_VALUES);
        }

        assert_content_free(&fixture, case.name, &response.body);
    }
}

#[test]
fn v3_machine_readiness_emits_exactly_the_published_keys_and_rows() {
    let fixture = Fixture::new();
    let command =
        json!({"provider": "command", "argv": [fixture.provider_bin], "timeout_ms": 1000})
            .to_string();
    for (name, config, expected) in [
        ("provider available", Some(command), V3_MACHINE_ROWS[0]),
        ("provider not configured", None, V3_MACHINE_ROWS[1]),
    ] {
        let _server = ServeProcess::spawn(&fixture, config.as_deref(), false, false);
        let response = fixture.request("GET", "/retitle/v3/readiness");
        assert_eq!(response.status, 200, "case={name} body={}", response.body);
        assert_outer_envelope(&response.body);
        let readiness = &response.body["data"]["retitle"];
        assert_eq!(object_keys(readiness), set(V3_MACHINE_KEYS), "case={name}");
        assert_eq!(
            readiness["schema_version"],
            "agent-session.session-retitle.readiness.v3"
        );
        assert_eq!(readiness["capability"], "agent-session.session-retitle.v3");
        let row = (
            readiness["status"].as_str().expect("status"),
            readiness["reason_code"].as_str().expect("reason_code"),
            readiness["next_action"].as_str().expect("next_action"),
        );
        assert!(
            V3_MACHINE_ROWS.contains(&row),
            "case={name}: {row:?} is not a published machine readiness row"
        );
        assert_eq!(row, expected, "case={name}");
        assert_content_free(&fixture, name, &response.body);
    }
}

#[test]
fn v3_session_readiness_emits_only_allowlisted_content_free_fields() {
    let fixture = Fixture::new();
    let config = json!({"provider": "command", "argv": [fixture.provider_bin], "timeout_ms": 1000})
        .to_string();
    let _server = ServeProcess::spawn(&fixture, Some(&config), false, false);
    let session_path = format!("/sessions/{SESSION_ID}/retitle-v3/readiness");

    let initial = fixture.request("GET", &session_path);
    assert_session_readiness(&fixture, "uninitialized memory", &initial, false);
    assert_session_row(
        &initial.body,
        ("catching_up", "memory_not_initialized", "refresh_memory"),
    );

    let providerless = fixture.request(
        "GET",
        &format!("/sessions/{PROVIDERLESS_SESSION_ID}/retitle-v3/readiness"),
    );
    assert_session_readiness(&fixture, "no provider history", &providerless, false);
    assert_session_row(
        &providerless.body,
        (
            "unavailable",
            "provider_history_unavailable",
            "restore_provider_history",
        ),
    );

    // A memory-first manual retitle folds the transcript, so the next
    // observation carries the optional opaque cursor fence.
    let admitted = fixture.post(
        &format!("/sessions/{SESSION_ID}/retitle-v3"),
        &json!({
            "schema_version": "agent-session.session-retitle.request.v3",
            "trigger": "manual",
            "idempotency_key": "readiness-contract-prime",
            "expected": {
                "session_incarnation": INCARNATION,
                "title_revision": 0,
                "memory_revision": 0
            }
        }),
    );
    assert!(
        matches!(admitted.status, 200 | 202),
        "body={}",
        admitted.body
    );
    let operation_hash = admitted.body["data"]["retitle"]["operation_hash"]
        .as_str()
        .expect("operation hash")
        .to_string();
    fixture.poll_terminal(&operation_hash);

    let folded = fixture.request("GET", &session_path);
    assert_session_readiness(&fixture, "folded memory", &folded, true);
    assert_session_row(&folded.body, ("ready", "ready", "none"));
    assert_eq!(folded.body["data"]["retitle"]["usable_memory"], true);
    assert_eq!(folded.body["data"]["retitle"]["title_status"], "current");
}

fn assert_session_readiness(fixture: &Fixture, name: &str, response: &HttpResponse, fenced: bool) {
    assert_eq!(response.status, 200, "case={name} body={}", response.body);
    assert_outer_envelope(&response.body);
    let readiness = &response.body["data"]["retitle"];
    let keys = object_keys(readiness);
    let unexpected = keys
        .difference(&set(V3_SESSION_ALLOWED_KEYS))
        .cloned()
        .collect::<Vec<_>>();
    assert!(
        unexpected.is_empty(),
        "case={name}: v3 session readiness emitted keys outside the allowlist: {unexpected:?}"
    );
    let mut expected = set(V3_SESSION_ALLOWED_KEYS);
    if !fenced {
        expected.remove("cursor_fence_hash");
    }
    assert_eq!(keys, expected, "case={name}");

    assert_eq!(
        readiness["schema_version"],
        "agent-session.session-retitle.readiness.v3"
    );
    assert_eq!(readiness["capability"], "agent-session.session-retitle.v3");
    assert_member(readiness, "status", V3_SESSION_STATUSES);
    assert_member(readiness, "context_status", V3_SESSION_STATUSES);
    assert_member(readiness, "provider_status", V3_PROVIDER_STATUSES);
    assert_member(readiness, "title_status", V3_TITLE_STATUSES);
    assert_member(readiness, "reason_code", V3_SESSION_REASONS);
    assert_member(readiness, "next_action", V3_SESSION_ACTIONS);
    assert!(readiness["memory_revision"].is_u64(), "case={name}");
    assert!(readiness["usable_memory"].is_boolean(), "case={name}");
    assert!(readiness["pending_operation"].is_boolean(), "case={name}");
    if fenced {
        let hash = readiness["cursor_fence_hash"]
            .as_str()
            .expect("cursor fence hash");
        let digest = hash.strip_prefix("sha256:").expect("sha256 prefix");
        assert!(
            digest.len() == 64
                && digest
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
            "case={name}: cursor fence is not an opaque SHA-256 digest"
        );
    }
    assert_content_free(fixture, name, &response.body);
}

fn assert_session_row(body: &Value, expected: (&str, &str, &str)) {
    let readiness = &body["data"]["retitle"];
    assert_eq!(
        (
            readiness["status"].as_str(),
            readiness["reason_code"].as_str(),
            readiness["next_action"].as_str(),
        ),
        (Some(expected.0), Some(expected.1), Some(expected.2)),
        "body={body}"
    );
    assert_eq!(readiness["context_status"], readiness["status"]);
}

fn assert_outer_envelope(body: &Value) {
    assert_eq!(object_keys(body), set(&["data", "ok", "schema_version"]));
    assert_eq!(body["schema_version"], "cli.agent-session.serve.v1");
    assert_eq!(body["ok"], true);
    assert_eq!(object_keys(&body["data"]), set(&["machine", "retitle"]));
    assert_eq!(body["data"]["machine"], MACHINE);
}

fn assert_member(object: &Value, key: &str, allowed: &[&str]) {
    let value = object[key].as_str().unwrap_or_else(|| {
        panic!("{key} must be a string, got {}", object[key]);
    });
    assert!(
        allowed.contains(&value),
        "{key}={value:?} is outside the stable vocabulary {allowed:?}"
    );
}

/// Readiness is a content-free observation: no bearer, credential, fixture
/// path, transcript text, or free-form provider error may appear in any value.
/// The one exception is v2 `plan`, which is broker metadata passed through
/// unchanged, so only the bounds the broker boundary enforces are asserted.
fn assert_content_free(fixture: &Fixture, name: &str, body: &Value) {
    let rendered = body.to_string();
    let root = fixture.root.to_string_lossy();
    for (index, forbidden) in [
        TOKEN,
        API_KEY_ENV,
        API_KEY_CANARY,
        CONFIG_CANARY,
        MODEL_CANARY,
        PATH_CANARY,
        ASSISTANT_CANARY,
        BROKER_LABEL_CANARY,
        PROVIDER_SESSION_ID,
        INCARNATION,
        root.as_ref(),
    ]
    .into_iter()
    .enumerate()
    {
        assert!(
            !rendered.contains(forbidden),
            "case={name}: readiness leaked forbidden fixture value {index}"
        );
    }
    let mut readiness = body["data"]["retitle"].clone();
    if let Some(plan) = readiness
        .as_object_mut()
        .and_then(|fields| fields.remove("plan"))
    {
        let plan = plan.as_str().expect("plan is a string");
        assert!(
            plan.len() <= 128 && !plan.contains(['\n', '\r', '\0']),
            "case={name}: plan exceeds the broker metadata bounds"
        );
    }
    for value in string_values(&readiness) {
        assert!(
            value.len() <= 128
                && value.bytes().all(|byte| byte.is_ascii_alphanumeric()
                    || matches!(byte, b'.' | b'_' | b'-' | b':')),
            "case={name}: readiness value {value:?} is not a bounded content-free token"
        );
    }
}

fn string_values(value: &Value) -> Vec<&str> {
    match value {
        Value::String(text) => vec![text.as_str()],
        Value::Array(items) => items.iter().flat_map(string_values).collect(),
        Value::Object(map) => map.values().flat_map(string_values).collect(),
        _ => Vec::new(),
    }
}

fn object_keys(value: &Value) -> BTreeSet<String> {
    value
        .as_object()
        .unwrap_or_else(|| panic!("expected a JSON object, got {value}"))
        .keys()
        .cloned()
        .collect()
}

fn set(keys: &[&str]) -> BTreeSet<String> {
    keys.iter().map(|key| key.to_string()).collect()
}

struct HttpResponse {
    status: u16,
    body: Value,
}

struct Fixture {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    home: PathBuf,
    state_dir: PathBuf,
    codex_home: PathBuf,
    tmux_bin: PathBuf,
    provider_bin: PathBuf,
    missing_provider_bin: PathBuf,
    broker_bin: PathBuf,
    address: SocketAddr,
}

impl Fixture {
    fn new() -> Self {
        let tmp = tempfile::TempDir::new().expect("retitle readiness fixture");
        let root = tmp.path().to_path_buf();
        let home = root.join("home");
        let state_dir = root.join("state");
        let codex_home = root.join("codex-home");
        let private_bin = root.join(PATH_CANARY);
        let tmux_bin = root.join("tmux");
        let provider_bin = private_bin.join("title-provider");
        let missing_provider_bin = private_bin.join("missing-title-provider");
        let broker_bin = private_bin.join("account-broker");
        fs::create_dir_all(&home).expect("fixture home");
        fs::create_dir_all(&private_bin).expect("fixture provider directory");
        write_executable(&tmux_bin, "#!/bin/sh\nexit 0\n");
        write_executable(
            &provider_bin,
            "#!/bin/sh\nprintf '%s\\n' '{\"topic_action\":\"keep\",\"topic\":null,\"activity\":null,\"references\":[]}'\n",
        );
        // The broker label is public metadata that readiness must not project.
        let accounts = json!({
            "schema_version": "agent-session.codex-auth-broker.v1",
            "accounts": [{
                "account": BROKER_ACCOUNT,
                "label": BROKER_LABEL_CANARY,
                "plan": BROKER_PLAN
            }],
            "selection_strategies": []
        });
        write_executable(
            &broker_bin,
            &format!(
                "#!/bin/sh\ncase \"$1\" in\n  list) printf '%s\\n' '{accounts}' ;;\n  *) exit 64 ;;\nesac\n"
            ),
        );
        seed_session(&state_dir, SESSION_ID, true);
        seed_session(&state_dir, PROVIDERLESS_SESSION_ID, false);
        seed_codex_transcript(&codex_home);
        let listener = TcpListener::bind("127.0.0.1:0").expect("reserve loopback address");
        let address = listener.local_addr().expect("loopback address");
        drop(listener);
        Self {
            _tmp: tmp,
            root,
            home,
            state_dir,
            codex_home,
            tmux_bin,
            provider_bin,
            missing_provider_bin,
            broker_bin,
            address,
        }
    }

    fn request(&self, method: &str, path: &str) -> HttpResponse {
        request_json(self.address, method, path, None)
    }

    fn post(&self, path: &str, body: &Value) -> HttpResponse {
        request_json(self.address, "POST", path, Some(body))
    }

    fn poll_terminal(&self, operation_hash: &str) {
        let path = format!("/sessions/{SESSION_ID}/retitle-v3/operations/{operation_hash}");
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            let response = self.request("GET", &path);
            assert_eq!(response.status, 200, "body={}", response.body);
            if response.body["data"]["retitle"]["status"] == "terminal" {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "retitle v3 operation did not become terminal: body={}",
                response.body
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

struct ServeProcess {
    child: Child,
    stderr_path: PathBuf,
}

impl ServeProcess {
    fn spawn(fixture: &Fixture, config: Option<&str>, api_key: bool, broker: bool) -> Self {
        let stderr_path = fixture.root.join("serve.stderr");
        let stderr = File::create(&stderr_path).expect("create serve stderr fixture");
        let mut command = Command::new(bin::resolve("agent-session"));
        command
            .current_dir(&fixture.root)
            .args([
                "serve",
                "--bind",
                &fixture.address.to_string(),
                "--state-dir",
                fixture.state_dir.to_str().expect("UTF-8 state dir"),
                "--token",
                TOKEN,
                "--machine",
                MACHINE,
                "--tmux-bin",
                fixture.tmux_bin.to_str().expect("UTF-8 tmux fixture"),
            ])
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", &fixture.home)
            .env("CODEX_HOME", &fixture.codex_home)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::from(stderr));
        if let Some(config) = config {
            command.env("AGENT_SESSION_RETITLE_CONFIG", config);
        }
        if api_key {
            command.env(API_KEY_ENV, API_KEY_CANARY);
        }
        if broker {
            command.env(
                "AGENT_SESSION_CODEX_ACCOUNT_BROKER",
                json!([fixture.broker_bin]).to_string(),
            );
        }
        let mut server = Self {
            child: command.spawn().expect("spawn agent-session serve"),
            stderr_path,
        };
        let deadline = Instant::now() + Duration::from_secs(15);
        while TcpStream::connect(fixture.address).is_err() {
            if let Some(status) = server.child.try_wait().expect("poll serve child") {
                panic!(
                    "agent-session serve exited before listening: status={status}; stderr={}",
                    server.sanitized_stderr()
                );
            }
            assert!(
                Instant::now() < deadline,
                "agent-session serve did not listen; stderr={}",
                server.sanitized_stderr()
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        server
    }

    fn sanitized_stderr(&self) -> String {
        fs::read_to_string(&self.stderr_path)
            .unwrap_or_default()
            .lines()
            .take(8)
            .map(|line| {
                if line.contains('/') {
                    "<path-redacted>"
                } else {
                    line
                }
            })
            .collect::<Vec<_>>()
            .join(" | ")
    }
}

impl Drop for ServeProcess {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
    }
}

fn request_json(
    address: SocketAddr,
    method: &str,
    path: &str,
    body: Option<&Value>,
) -> HttpResponse {
    let mut stream = TcpStream::connect(address).expect("connect agent-session serve");
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .expect("HTTP read timeout");
    stream
        .set_write_timeout(Some(Duration::from_secs(10)))
        .expect("HTTP write timeout");
    let body = body.map(Value::to_string).unwrap_or_default();
    write!(
        stream,
        "{method} {path} HTTP/1.1\r\nHost: {address}\r\nAuthorization: Bearer {TOKEN}\r\n"
    )
    .expect("HTTP request head");
    if !body.is_empty() {
        write!(
            stream,
            "Content-Type: application/json\r\nContent-Length: {}\r\n",
            body.len()
        )
        .expect("HTTP content headers");
    }
    write!(stream, "Connection: close\r\n\r\n{body}").expect("HTTP body");
    stream.flush().expect("flush HTTP request");
    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .expect("read HTTP response");
    let header_end = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .expect("HTTP response header boundary");
    let headers = std::str::from_utf8(&response[..header_end]).expect("HTTP response headers");
    let status = headers
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|value| value.parse::<u16>().ok())
        .expect("HTTP response status");
    let body = serde_json::from_slice(&response[header_end + 4..]).expect("HTTP JSON response");
    HttpResponse { status, body }
}

fn seed_session(state_dir: &Path, id: &str, provider_history: bool) {
    let directory = state_dir.join("sessions").join(id);
    fs::create_dir_all(&directory).expect("managed session directory");
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))
        .expect("managed session directory mode");
    let tmux_session = format!("hs-{id}");
    let mut record = json!({
        "schema_version": "agent-session.session.v1",
        "id": id,
        "agent": "codex",
        "mode": "interactive",
        "title": null,
        "title_revision": 0,
        "cwd": format!("/synthetic/{PATH_CANARY}"),
        "tmux_session": tmux_session,
        "prompt_file": null,
        "log_file": null,
        "created_at": "2026-09-09T00:00:00Z",
        "updated_at": "2026-09-09T00:00:00Z",
        "runtime": {
            "kind": "tmux",
            "tmux_session": tmux_session,
            "generation": 1,
            "started_at": "2026-09-09T00:00:00Z",
            "launch_id": INCARNATION
        }
    });
    if provider_history {
        record["provider_resume"] = json!({
            "provider": "codex",
            "session_id": PROVIDER_SESSION_ID,
            "captured_at": "2026-09-09T00:00:03Z",
            "capture_method": "synthetic_fixture",
            "resume_args": []
        });
    }
    write_private_json(&directory.join("session.json"), &record);
}

fn seed_codex_transcript(codex_home: &Path) {
    let directory = codex_home.join("sessions/2026/09/09");
    fs::create_dir_all(&directory).expect("Codex history directory");
    let records = [
        json!({
            "timestamp": "2026-09-09T00:00:00Z",
            "type": "session_meta",
            "payload": {
                "id": PROVIDER_SESSION_ID,
                "cwd": format!("/synthetic/{PATH_CANARY}"),
                "source": "cli",
                "timestamp": "2026-09-09T00:00:00Z"
            }
        }),
        json!({
            "timestamp": "2026-09-09T00:00:01Z",
            "type": "response_item",
            "payload": {
                "type": "message",
                "role": "user",
                "content": [{
                    "type": "input_text",
                    "text": "Pin the retitle readiness contract"
                }],
                "internal_chat_message_metadata_passthrough": {
                    "turn_id": "synthetic-human-turn",
                    "content_item_kinds": ["user.text"]
                }
            }
        }),
        json!({
            "timestamp": "2026-09-09T00:00:02Z",
            "type": "response_item",
            "payload": {
                "type": "message",
                "role": "assistant",
                "content": [{
                    "type": "output_text",
                    "text": format!("{ASSISTANT_CANARY} {API_KEY_CANARY}")
                }]
            }
        }),
    ];
    let rendered = records
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    let transcript = directory.join("rollout-retitle-readiness-contract.jsonl");
    fs::write(&transcript, rendered).expect("Codex history fixture");
    fs::set_permissions(&transcript, fs::Permissions::from_mode(0o600))
        .expect("Codex history fixture mode");
}

fn write_private_json(path: &Path, value: &Value) {
    fs::write(
        path,
        serde_json::to_vec_pretty(value).expect("fixture JSON"),
    )
    .expect("write private JSON fixture");
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .expect("private JSON fixture mode");
}

fn write_executable(path: &Path, contents: &str) {
    fs::write(path, contents).expect("write executable fixture");
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).expect("executable fixture mode");
}
