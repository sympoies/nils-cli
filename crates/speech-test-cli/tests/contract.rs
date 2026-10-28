use nils_speech_test_cli::{
    MAX_SCORE_CHARACTERS, TtsRequest, canonical_fingerprint, read_catalog, sha256,
};
use nils_test_support::bin;
use nils_test_support::cmd::{self, CmdOptions, CmdOutput};
use nils_test_support::http::{HttpResponse, LoopbackServer};
use sha2::Digest;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::Duration;
use tempfile::TempDir;

#[test]
fn catalog_reader_rejects_duplicate_and_unsafe_fixture_ids() {
    let temp = TempDir::new().unwrap();
    let fixture = |id: String| {
        serde_json::json!({
            "id": id,
            "intended_text": "x",
            "license_source": "self-made",
            "generation": {"provider":"manual", "model":"test", "voice":"none"},
            "master_sha256": "a",
            "derived_sha256": "a",
            "file": "x.wav",
            "codec": "wav",
            "sample_rate_hz": 16000,
            "channels": 1,
            "duration_seconds": 0.01
        })
    };
    for fixtures in [
        vec![fixture("same".into()), fixture("same".into())],
        vec![fixture("../escape".into())],
        vec![fixture("x".repeat(65))],
    ] {
        let path = temp.path().join("catalog.json");
        fs::write(
            &path,
            serde_json::to_vec(&serde_json::json!({
                "schema_version": "speech-test.catalog.v1",
                "fixtures": fixtures
            }))
            .unwrap(),
        )
        .unwrap();
        assert!(read_catalog(&path).is_err());
    }
    let path = temp.path().join("catalog.json");
    fs::write(
        &path,
        serde_json::to_vec(&serde_json::json!({
            "schema_version": "speech-test.catalog.v1",
            "fixtures": [fixture("x".repeat(64))]
        }))
        .unwrap(),
    )
    .unwrap();
    assert!(read_catalog(&path).is_ok());
}

fn run(args: &[&str], options: &CmdOptions) -> CmdOutput {
    cmd::run_with(&bin::resolve("speech-test-cli"), args, options)
}

fn reviewed_catalog(root: &TempDir, audio: &str) -> PathBuf {
    let bytes = tiny_wav();
    fs::write(root.path().join("sample.wav"), &bytes).unwrap();
    let hash = sha2::Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let catalog = serde_json::json!({
        "schema_version": "speech-test.catalog.v1",
        "fixtures": [{
            "id": "sample-tone",
            "tags": ["synthetic"],
            "intended_text": "A fictional test phrase.",
            "spoken_reference": "A fictional test phrase.",
            "review": {"reviewer": "fixture-reviewer", "reviewed_at": "2026-10-01T00:00:00Z"},
            "license_source": "self-made fixture",
            "generation": {"provider": "manual", "model": "synthetic", "voice": "none", "settings": {}},
            "master_sha256": hash,
            "derived_sha256": hash,
            "file": audio,
            "codec": "wav",
            "sample_rate_hz": 16000,
            "channels": 1,
            "duration_seconds": 0.01
        }]
    });
    let path = root.path().join("catalog.json");
    fs::write(&path, serde_json::to_vec(&catalog).unwrap()).unwrap();
    path
}

fn configure_tts(catalog: &PathBuf, settings: serde_json::Value) {
    let mut value: serde_json::Value = serde_json::from_slice(&fs::read(catalog).unwrap()).unwrap();
    value["fixtures"][0]["generation"] = serde_json::json!({
        "provider":"elevenlabs", "model":"model-fixture", "voice":"voice-fixture", "settings":settings
    });
    fs::write(catalog, serde_json::to_vec(&value).unwrap()).unwrap();
}

fn tiny_wav() -> Vec<u8> {
    let samples = (0..160u32)
        .map(|i| ((i as f64 * std::f64::consts::TAU * 440.0 / 16_000.0).sin() * 5000.0) as i16)
        .collect::<Vec<_>>();
    let data_len = (samples.len() * 2) as u32;
    let mut bytes = Vec::with_capacity(44 + data_len as usize);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&16_000u32.to_le_bytes());
    bytes.extend_from_slice(&32_000u32.to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_len.to_le_bytes());
    for sample in samples {
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    bytes
}

fn start_stalled_body_server() -> (String, Arc<AtomicUsize>, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let requests = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&requests);
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut request = [0u8; 8192];
        let _ = stream.read(&mut request);
        seen.fetch_add(1, Ordering::SeqCst);
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\n")
            .unwrap();
        thread::sleep(Duration::from_millis(1300));
        let _ = stream.write_all(b"body");
    });
    (base, requests, server)
}

#[test]
fn replay_rejects_missing_audio_before_asr_and_emits_json_error() {
    let temp = TempDir::new().unwrap();
    let catalog = reviewed_catalog(&temp, "missing.wav");
    let args = [
        "--format",
        "json",
        "replay",
        "--catalog",
        catalog.to_str().unwrap(),
        "--endpoint",
        "http://127.0.0.1:9/asr",
    ];
    let output = run(&args, &CmdOptions::default());
    assert_eq!(output.code, 65);
    let value: serde_json::Value = serde_json::from_str(&output.stdout_text()).unwrap();
    assert_eq!(value["ok"], false);
    assert_eq!(value["error"]["code"], "fixture-missing");
}

#[test]
fn replay_redacts_query_credentials_from_transport_errors() {
    let temp = TempDir::new().unwrap();
    let catalog = reviewed_catalog(&temp, "sample.wav");
    let server = LoopbackServer::new().unwrap();
    let endpoint = format!("{}/asr?credential=synthetic-marker", server.url());
    drop(server);
    let output = run(
        &[
            "--format",
            "json",
            "replay",
            "--catalog",
            catalog.to_str().unwrap(),
            "--endpoint",
            &endpoint,
        ],
        &CmdOptions::default(),
    );
    assert_ne!(output.code, 0);
    assert!(!output.stdout_text().contains("synthetic-marker"));
    assert!(!output.stderr_text().contains("synthetic-marker"));
}

#[test]
fn replay_rejects_oversized_transcripts_before_cer_scoring() {
    let temp = TempDir::new().unwrap();
    let catalog = reviewed_catalog(&temp, "sample.wav");
    let server = LoopbackServer::new().unwrap();
    let transcript = "x".repeat(MAX_SCORE_CHARACTERS + 1);
    server.add_route(
        "POST",
        "/asr",
        HttpResponse::new(200, serde_json::json!({"text": transcript}).to_string()),
    );
    let endpoint = format!("{}/asr", server.url());
    let output = run(
        &[
            "--format",
            "json",
            "replay",
            "--catalog",
            catalog.to_str().unwrap(),
            "--endpoint",
            &endpoint,
        ],
        &CmdOptions::default(),
    );
    assert_eq!(output.code, 65);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&output.stdout_text()).unwrap()["error"]["code"],
        "score-input-too-large"
    );
    assert_eq!(server.take_requests().len(), 1);
}

#[test]
fn replay_rejects_bad_hash_before_asr() {
    let temp = TempDir::new().unwrap();
    let catalog = reviewed_catalog(&temp, "sample.wav");
    let mut value: serde_json::Value =
        serde_json::from_slice(&fs::read(&catalog).unwrap()).unwrap();
    value["fixtures"][0]["derived_sha256"] = serde_json::json!("00");
    fs::write(&catalog, serde_json::to_vec(&value).unwrap()).unwrap();
    let args = [
        "replay",
        "--catalog",
        catalog.to_str().unwrap(),
        "--endpoint",
        "http://127.0.0.1:9/asr",
    ];
    let output = run(&args, &CmdOptions::default());
    assert_eq!(output.code, 65);
    assert!(output.stderr_text().contains("fixture hash"));
}

#[test]
fn replay_rejects_unreviewed_reference_before_asr() {
    let temp = TempDir::new().unwrap();
    let catalog = reviewed_catalog(&temp, "sample.wav");
    let mut value: serde_json::Value =
        serde_json::from_slice(&fs::read(&catalog).unwrap()).unwrap();
    value["fixtures"][0]["review"] = serde_json::Value::Null;
    value["fixtures"][0]["spoken_reference"] = serde_json::Value::Null;
    fs::write(&catalog, serde_json::to_vec(&value).unwrap()).unwrap();
    let server = LoopbackServer::new().unwrap();
    server.add_route(
        "POST",
        "/asr",
        HttpResponse::new(200, r#"{"text":"ignored"}"#),
    );
    let endpoint = format!("{}/asr", server.url());
    let output = run(
        &[
            "--format",
            "json",
            "replay",
            "--catalog",
            catalog.to_str().unwrap(),
            "--endpoint",
            &endpoint,
        ],
        &CmdOptions::default(),
    );
    assert_eq!(output.code, 65);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&output.stdout_text()).unwrap()["error"]["code"],
        "fixture-unreviewed"
    );
    assert!(server.take_requests().is_empty());
}

#[test]
fn replay_preflights_every_fixture_before_sending_any_audio() {
    let temp = TempDir::new().unwrap();
    let catalog = reviewed_catalog(&temp, "sample.wav");
    fs::write(temp.path().join("second.wav"), tiny_wav()).unwrap();
    let mut value: serde_json::Value =
        serde_json::from_slice(&fs::read(&catalog).unwrap()).unwrap();
    let mut second = value["fixtures"][0].clone();
    second["id"] = serde_json::json!("second-tone");
    second["file"] = serde_json::json!("second.wav");
    second["review"] = serde_json::Value::Null;
    second["spoken_reference"] = serde_json::Value::Null;
    value["fixtures"].as_array_mut().unwrap().push(second);
    fs::write(&catalog, serde_json::to_vec(&value).unwrap()).unwrap();

    let server = LoopbackServer::new().unwrap();
    server.add_route(
        "POST",
        "/asr",
        HttpResponse::new(200, r#"{"text":"ignored"}"#),
    );
    let endpoint = format!("{}/asr", server.url());
    let output = run(
        &[
            "--format",
            "json",
            "replay",
            "--catalog",
            catalog.to_str().unwrap(),
            "--endpoint",
            &endpoint,
        ],
        &CmdOptions::default(),
    );
    assert_eq!(output.code, 65);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&output.stdout_text()).unwrap()["error"]["code"],
        "fixture-unreviewed"
    );
    assert!(server.take_requests().is_empty());
}

#[test]
fn review_persists_metadata_and_refuses_tampered_audio_without_mutation() {
    let temp = TempDir::new().unwrap();
    let catalog = reviewed_catalog(&temp, "sample.wav");
    let mut value: serde_json::Value =
        serde_json::from_slice(&fs::read(&catalog).unwrap()).unwrap();
    value["fixtures"][0]["review"] = serde_json::Value::Null;
    value["fixtures"][0]["spoken_reference"] = serde_json::Value::Null;
    fs::write(&catalog, serde_json::to_vec(&value).unwrap()).unwrap();
    let output = run(
        &[
            "review",
            "--catalog",
            catalog.to_str().unwrap(),
            "--id",
            "sample-tone",
            "--reference",
            "A reviewed fictional phrase.",
            "--reviewer",
            "fixture-reviewer",
        ],
        &CmdOptions::default(),
    );
    assert_eq!(output.code, 0);
    let reviewed: serde_json::Value = serde_json::from_slice(&fs::read(&catalog).unwrap()).unwrap();
    assert_eq!(
        reviewed["fixtures"][0]["spoken_reference"],
        "A reviewed fictional phrase."
    );
    assert_eq!(
        reviewed["fixtures"][0]["review"]["reviewer"],
        "fixture-reviewer"
    );
    assert!(
        reviewed["fixtures"][0]["review"]["reviewed_at"]
            .as_str()
            .is_some()
    );

    let mut tampered: serde_json::Value =
        serde_json::from_slice(&fs::read(&catalog).unwrap()).unwrap();
    tampered["fixtures"][0]["derived_sha256"] = serde_json::json!("00");
    tampered["fixtures"][0]["review"] = serde_json::Value::Null;
    tampered["fixtures"][0]["spoken_reference"] = serde_json::Value::Null;
    fs::write(&catalog, serde_json::to_vec(&tampered).unwrap()).unwrap();
    let before = fs::read(&catalog).unwrap();
    let output = run(
        &[
            "review",
            "--catalog",
            catalog.to_str().unwrap(),
            "--id",
            "sample-tone",
            "--reference",
            "A reviewed fictional phrase.",
            "--reviewer",
            "fixture-reviewer",
        ],
        &CmdOptions::default(),
    );
    assert_eq!(output.code, 65);
    assert_eq!(fs::read(&catalog).unwrap(), before);
}

#[test]
fn invalid_catalog_review_and_run_inputs_use_data_error_contract() {
    let temp = TempDir::new().unwrap();
    let catalog = reviewed_catalog(&temp, "sample.wav");
    let mut unsupported: serde_json::Value =
        serde_json::from_slice(&fs::read(&catalog).unwrap()).unwrap();
    unsupported["schema_version"] = serde_json::json!("speech-test.catalog.v0");
    fs::write(&catalog, serde_json::to_vec(&unsupported).unwrap()).unwrap();
    let output = run(
        &[
            "--format",
            "json",
            "generate",
            "--plan",
            "--cost-per-1000-characters",
            "0.12",
            "--catalog",
            catalog.to_str().unwrap(),
        ],
        &CmdOptions::default(),
    );
    assert_eq!(output.code, 65);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&output.stdout_text()).unwrap()["error"]["code"],
        "catalog-invalid"
    );

    let catalog = reviewed_catalog(&temp, "sample.wav");
    let output = run(
        &[
            "--format",
            "json",
            "review",
            "--catalog",
            catalog.to_str().unwrap(),
            "--id",
            "absent",
            "--reference",
            "text",
            "--reviewer",
            "fixture-reviewer",
        ],
        &CmdOptions::default(),
    );
    assert_eq!(output.code, 65);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&output.stdout_text()).unwrap()["error"]["code"],
        "fixture-not-found"
    );

    let saved_run = temp.path().join("unsupported-run.json");
    fs::write(&saved_run, r#"{"schema_version":"speech-test.run.v0","scores":[],"latency_p50_ms":0,"latency_p95_ms":0,"real_time_factor":0.0}"#).unwrap();
    let output = run(
        &[
            "--format",
            "json",
            "score",
            "--run",
            saved_run.to_str().unwrap(),
        ],
        &CmdOptions::default(),
    );
    assert_eq!(output.code, 65);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&output.stdout_text()).unwrap()["error"]["code"],
        "run-invalid"
    );

    let before = fs::read(&catalog).unwrap();
    let output = run(
        &[
            "--format",
            "json",
            "review",
            "--catalog",
            catalog.to_str().unwrap(),
            "--id",
            "sample-tone",
            "--reference",
            "   ",
            "--reviewer",
            "fixture-reviewer",
        ],
        &CmdOptions::default(),
    );
    assert_eq!(output.code, 65);
    assert_eq!(fs::read(&catalog).unwrap(), before);
}

#[test]
fn generate_plan_refuses_character_budget_without_network_access() {
    let temp = TempDir::new().unwrap();
    let catalog = reviewed_catalog(&temp, "sample.wav");
    configure_tts(&catalog, serde_json::json!({}));
    let args = [
        "generate",
        "--catalog",
        catalog.to_str().unwrap(),
        "--max-characters",
        "3",
        "--format",
        "json",
    ];
    let output = run(&args, &CmdOptions::default());
    assert_eq!(output.code, 65);
    let value: serde_json::Value = serde_json::from_str(&output.stdout_text()).unwrap();
    assert_eq!(value["error"]["code"], "character-budget-exceeded");
}

#[test]
fn cli_has_version_and_json_parse_errors_follow_output_contract() {
    assert_eq!(run(&["--version"], &CmdOptions::default()).code, 0);
    let output = run(
        &["--format", "json", "--unknown-option"],
        &CmdOptions::default(),
    );
    assert_eq!(output.code, 64);
    let value: serde_json::Value = serde_json::from_str(&output.stdout_text()).unwrap();
    assert_eq!(value["schema_version"], "cli.speech-test-cli.error.v1");
    assert_eq!(value["ok"], false);
    assert_eq!(value["error"]["code"], "parse-error");
}

#[test]
fn completion_export_contains_cli_commands_and_format_flag() {
    let output = run(&["completion", "bash"], &CmdOptions::default());
    assert_eq!(output.code, 0);
    let script = output.stdout_text();
    assert!(script.contains("generate"));
    assert!(script.contains("replay"));
    assert!(script.contains("--format"));
}

#[test]
fn plan_json_is_versioned_and_does_not_expose_credentials() {
    let temp = TempDir::new().unwrap();
    let catalog = reviewed_catalog(&temp, "sample.wav");
    configure_tts(&catalog, serde_json::json!({}));
    let tts_server = LoopbackServer::new().unwrap();
    let tts_base = tts_server.url();
    let args = [
        "--format",
        "json",
        "generate",
        "--plan",
        "--catalog",
        catalog.to_str().unwrap(),
        "--cost-per-1000-characters",
        "0.12",
    ];
    let options = CmdOptions::default()
        .with_env("ELEVENLABS_API_KEY", "fixture-secret")
        .with_env("SPEECH_TEST_ELEVENLABS_BASE_URL", &tts_base);
    let output = run(&args, &options);
    assert_eq!(output.code, 0);
    let stdout = output.stdout_text();
    let value: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(value["schema_version"], "cli.speech-test-cli.generate.v1");
    assert!(value["data"]["estimated_cost_usd"].as_f64().unwrap() > 0.0);
    assert!(!stdout.contains("fixture-secret"));
    assert!(
        tts_server.take_requests().is_empty(),
        "--plan must never call TTS"
    );
}

#[test]
fn replay_sends_verified_audio_once_and_saves_headers_and_offline_scores() {
    let temp = TempDir::new().unwrap();
    let catalog = reviewed_catalog(&temp, "sample.wav");
    let server = LoopbackServer::new().unwrap();
    let tts_server = LoopbackServer::new().unwrap();
    tts_server.add_route(
        "POST",
        "/v1/text-to-speech/voice-fixture",
        HttpResponse::new(200, "unexpected tts call"),
    );
    server.add_route(
        "POST",
        "/asr",
        HttpResponse::new(200, r#"{"text":"A fictional test phrase."}"#)
            .with_header("x-backend", "stub-asr")
            .with_header("x-role", "test"),
    );
    let output_path = temp.path().join("saved-run.json");
    let args = [
        "--format",
        "json",
        "replay",
        "--catalog",
        catalog.to_str().unwrap(),
        "--endpoint",
        &format!("{}/asr", server.url()),
        "--auth-env",
        "ASR_TEST_TOKEN",
        "--run-output",
        output_path.to_str().unwrap(),
    ];
    let tts_base = tts_server.url();
    let options = CmdOptions::default()
        .with_env("ASR_TEST_TOKEN", "stub-token")
        .with_env("ELEVENLABS_API_KEY", "stub-secret")
        .with_env("SPEECH_TEST_ELEVENLABS_BASE_URL", &tts_base);
    let output = run(&args, &options);
    assert_eq!(output.code, 0, "stderr={}", output.stderr_text());
    let requests = server.take_requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].body, tiny_wav());
    assert_eq!(
        requests[0].header_value("authorization").as_deref(),
        Some("Bearer stub-token")
    );
    assert!(
        tts_server.take_requests().is_empty(),
        "replay must never call TTS"
    );
    let saved: serde_json::Value =
        serde_json::from_slice(&fs::read(&output_path).unwrap()).unwrap();
    assert_eq!(saved["scores"][0]["backend"], "stub-asr");
    assert_eq!(saved["scores"][0]["role"], "test");
    let score = run(
        &[
            "--format",
            "json",
            "score",
            "--run",
            output_path.to_str().unwrap(),
        ],
        &CmdOptions::default(),
    );
    assert_eq!(score.code, 0);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&score.stdout_text()).unwrap()["data"],
        saved
    );
}

#[test]
fn generation_cache_hits_identical_requests_and_misses_changed_settings() {
    let temp = TempDir::new().unwrap();
    let catalog = reviewed_catalog(&temp, "sample.wav");
    let mut value: serde_json::Value =
        serde_json::from_slice(&fs::read(&catalog).unwrap()).unwrap();
    value["fixtures"][0]["generation"] = serde_json::json!({"provider":"elevenlabs","model":"model-fixture","voice":"voice-fixture","settings":{"api_version":"v1","speed":1.2,"seed":42,"language":"en","normalization":"auto","voice_settings":{"stability":0.5},"output_format":"mp3_44100_128"}});
    fs::write(&catalog, serde_json::to_vec(&value).unwrap()).unwrap();
    let server = LoopbackServer::new().unwrap();
    server.add_route(
        "POST",
        "/v1/text-to-speech/voice-fixture",
        HttpResponse::new(200, "tiny-fixture-audio"),
    );
    let cache = temp.path().join("cache");
    let args = [
        "generate",
        "--catalog",
        catalog.to_str().unwrap(),
        "--cache-dir",
        cache.to_str().unwrap(),
    ];
    let tts_base = server.url();
    let options = CmdOptions::default()
        .with_env("ELEVENLABS_API_KEY", "stub-secret")
        .with_env("SPEECH_TEST_ELEVENLABS_BASE_URL", &tts_base);
    assert_eq!(run(&args, &options).code, 0);
    let requests = server.take_requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].path, "/v1/text-to-speech/voice-fixture");
    assert_eq!(
        requests[0].query.as_deref(),
        Some("output_format=mp3_44100_128")
    );
    let body: serde_json::Value = serde_json::from_str(&requests[0].body_text()).unwrap();
    assert_eq!(body["text"], "A fictional test phrase.");
    assert_eq!(body["model_id"], "model-fixture");
    assert_eq!(body["voice_settings"]["stability"], 0.5);
    assert_eq!(body["voice_settings"]["speed"], 1.2);
    assert_eq!(body["seed"], 42);
    assert_eq!(body["language_code"], "en");
    assert_eq!(body["apply_text_normalization"], "auto");
    let first_catalog = fs::read(&catalog).unwrap();
    let generated_dir = temp.path().join(".speech-test-generated");
    let generated_files = || {
        fs::read_dir(&generated_dir)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>()
    };
    let first_files = generated_files();
    assert_eq!(first_files.len(), 1);
    let no_key_options = CmdOptions::default()
        .with_env("SPEECH_TEST_ELEVENLABS_BASE_URL", &tts_base)
        .with_env_remove("ELEVENLABS_API_KEY");
    assert_eq!(run(&args, &no_key_options).code, 0);
    assert_eq!(fs::read(&catalog).unwrap(), first_catalog);
    assert_eq!(generated_files(), first_files);
    assert!(
        server.take_requests().is_empty(),
        "identical requests must use cached audio"
    );
    server.add_route(
        "POST",
        "/v1/text-to-speech/voice-fixture",
        HttpResponse::new(200, "different-fixture-audio"),
    );
    let mut value: serde_json::Value =
        serde_json::from_slice(&fs::read(&catalog).unwrap()).unwrap();
    value["fixtures"][0]["generation"]["settings"]["speed"] = serde_json::json!(1.1);
    fs::write(&catalog, serde_json::to_vec(&value).unwrap()).unwrap();
    assert_eq!(run(&args, &options).code, 0);
    let requests = server.take_requests();
    assert_eq!(
        requests.len(),
        1,
        "a changed request field must miss the cache"
    );
    assert_eq!(
        requests[0].header_value("xi-api-key").as_deref(),
        Some("stub-secret")
    );
    let updated: serde_json::Value = serde_json::from_slice(&fs::read(&catalog).unwrap()).unwrap();
    assert!(
        updated["fixtures"][0]["review"].is_null(),
        "new audio requires a fresh review"
    );
}

#[test]
fn generation_batches_identical_requests_once() {
    let temp = TempDir::new().unwrap();
    let catalog = reviewed_catalog(&temp, "sample.wav");
    configure_tts(
        &catalog,
        serde_json::json!({"speed":1.2,"voice_settings":{"stability":0.5}}),
    );
    fs::write(temp.path().join("second.wav"), tiny_wav()).unwrap();
    let mut value: serde_json::Value =
        serde_json::from_slice(&fs::read(&catalog).unwrap()).unwrap();
    let mut second = value["fixtures"][0].clone();
    second["id"] = serde_json::json!("second-tone");
    second["file"] = serde_json::json!("second.wav");
    value["fixtures"].as_array_mut().unwrap().push(second);
    fs::write(&catalog, serde_json::to_vec(&value).unwrap()).unwrap();

    let server = LoopbackServer::new().unwrap();
    server.add_route(
        "POST",
        "/v1/text-to-speech/voice-fixture",
        HttpResponse::new(200, "one-audio-payload"),
    );
    let base = server.url();
    let output = run(
        &[
            "--format",
            "json",
            "generate",
            "--catalog",
            catalog.to_str().unwrap(),
        ],
        &CmdOptions::default()
            .with_env("ELEVENLABS_API_KEY", "stub-secret")
            .with_env("SPEECH_TEST_ELEVENLABS_BASE_URL", &base),
    );
    assert_eq!(output.code, 0, "stderr={}", output.stderr_text());
    let requests = server.take_requests();
    assert_eq!(requests.len(), 1, "one TTS call per request fingerprint");
    assert_eq!(
        requests[0].header_value("xi-api-key").as_deref(),
        Some("stub-secret")
    );
    let updated: serde_json::Value = serde_json::from_slice(&fs::read(&catalog).unwrap()).unwrap();
    let fixtures = updated["fixtures"].as_array().unwrap();
    assert_eq!(fixtures.len(), 2);
    let first = fs::read(temp.path().join(fixtures[0]["file"].as_str().unwrap())).unwrap();
    let second = fs::read(temp.path().join(fixtures[1]["file"].as_str().unwrap())).unwrap();
    assert_eq!(first, second);
}

#[test]
fn tts_request_payload_matches_catalog_settings_and_redirects_do_not_forward_keys() {
    let temp = TempDir::new().unwrap();
    let catalog = reviewed_catalog(&temp, "sample.wav");
    let mut value: serde_json::Value =
        serde_json::from_slice(&fs::read(&catalog).unwrap()).unwrap();
    value["fixtures"][0]["generation"] = serde_json::json!({"provider":"elevenlabs","model":"model-fixture","voice":"voice-fixture","settings":{
        "api_version":"v1", "speed":1.2, "seed":42, "language":"en", "normalization":"auto",
        "voice_settings":{"stability":0.5}, "output_format":"mp3_44100_128"
    }});
    fs::write(&catalog, serde_json::to_vec(&value).unwrap()).unwrap();
    let server = LoopbackServer::new().unwrap();
    server.add_route(
        "POST",
        "/v1/text-to-speech/voice-fixture",
        HttpResponse::new(200, "synthetic-audio"),
    );
    let base = server.url();
    let output = run(
        &[
            "--format",
            "json",
            "generate",
            "--catalog",
            catalog.to_str().unwrap(),
        ],
        &CmdOptions::default()
            .with_env("ELEVENLABS_API_KEY", "stub-secret")
            .with_env("SPEECH_TEST_ELEVENLABS_BASE_URL", &base),
    );
    assert_eq!(output.code, 0, "stderr={}", output.stderr_text());
    let requests = server.take_requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].path, "/v1/text-to-speech/voice-fixture");
    assert_eq!(
        requests[0].query.as_deref(),
        Some("output_format=mp3_44100_128")
    );
    assert_eq!(
        requests[0].header_value("xi-api-key").as_deref(),
        Some("stub-secret")
    );
    let body: serde_json::Value = serde_json::from_str(&requests[0].body_text()).unwrap();
    assert_eq!(body["text"], "A fictional test phrase.");
    assert_eq!(body["model_id"], "model-fixture");
    assert_eq!(body["voice_settings"]["stability"], 0.5);
    assert_eq!(body["voice_settings"]["speed"], 1.2);
    assert_eq!(body["seed"], 42);
    assert_eq!(body["language_code"], "en");
    assert_eq!(body["apply_text_normalization"], "auto");

    let redirect_temp = TempDir::new().unwrap();
    let redirect_catalog = reviewed_catalog(&redirect_temp, "sample.wav");
    configure_tts(&redirect_catalog, serde_json::json!({}));
    let first = LoopbackServer::new().unwrap();
    let second = LoopbackServer::new().unwrap();
    let destination = format!("{}/capture", second.url());
    first.add_route(
        "POST",
        "/v1/text-to-speech/voice-fixture",
        HttpResponse::new(307, "").with_header("location", &destination),
    );
    second.add_route(
        "POST",
        "/capture",
        HttpResponse::new(200, "should not receive the key"),
    );
    let base = first.url();
    let redirected = run(
        &["generate", "--catalog", redirect_catalog.to_str().unwrap()],
        &CmdOptions::default()
            .with_env("ELEVENLABS_API_KEY", "stub-secret")
            .with_env("SPEECH_TEST_ELEVENLABS_BASE_URL", &base),
    );
    assert_eq!(redirected.code, 69);
    assert_eq!(first.take_requests().len(), 1);
    assert!(
        second.take_requests().is_empty(),
        "TTS redirects must not forward credentials"
    );
}

#[test]
fn generation_validates_fixture_ids_before_paid_requests() {
    let temp = TempDir::new().unwrap();
    let catalog = reviewed_catalog(&temp, "sample.wav");
    configure_tts(&catalog, serde_json::json!({}));
    let mut value: serde_json::Value =
        serde_json::from_slice(&fs::read(&catalog).unwrap()).unwrap();
    value["fixtures"][0]["id"] = serde_json::json!("bad id");
    fs::write(&catalog, serde_json::to_vec(&value).unwrap()).unwrap();
    let server = LoopbackServer::new().unwrap();
    let base = server.url();
    let output = run(
        &[
            "--format",
            "json",
            "generate",
            "--catalog",
            catalog.to_str().unwrap(),
        ],
        &CmdOptions::default()
            .with_env("ELEVENLABS_API_KEY", "stub-secret")
            .with_env("SPEECH_TEST_ELEVENLABS_BASE_URL", &base),
    );
    assert_eq!(output.code, 65);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&output.stdout_text()).unwrap()["error"]["code"],
        "catalog-invalid"
    );
    assert!(server.take_requests().is_empty());
    assert!(!temp.path().join(".speech-test-cache").exists());
}

#[test]
fn failed_multi_fixture_generation_preserves_previous_audio_review_and_catalog() {
    let temp = TempDir::new().unwrap();
    let catalog = reviewed_catalog(&temp, "sample.wav");
    configure_tts(&catalog, serde_json::json!({}));
    let server = LoopbackServer::new().unwrap();
    server.add_route(
        "POST",
        "/v1/text-to-speech/voice-fixture",
        HttpResponse::new(200, "first-audio"),
    );
    let base = server.url();
    let first = run(
        &["generate", "--catalog", catalog.to_str().unwrap()],
        &CmdOptions::default()
            .with_env("ELEVENLABS_API_KEY", "stub-secret")
            .with_env("SPEECH_TEST_ELEVENLABS_BASE_URL", &base),
    );
    assert_eq!(first.code, 0);
    server.take_requests();
    let mut value: serde_json::Value =
        serde_json::from_slice(&fs::read(&catalog).unwrap()).unwrap();
    value["fixtures"][0]["review"] =
        serde_json::json!({"reviewer":"fixture-reviewer","reviewed_at":"2026-10-01T00:00:00Z"});
    value["fixtures"][0]["spoken_reference"] = serde_json::json!("A reviewed fictional phrase.");
    let mut second = value["fixtures"][0].clone();
    second["id"] = serde_json::json!("second-tone");
    second["generation"]["voice"] = serde_json::json!("voice-second");
    second["review"] = serde_json::Value::Null;
    second["spoken_reference"] = serde_json::Value::Null;
    value["fixtures"].as_array_mut().unwrap().push(second);
    value["fixtures"][0]["intended_text"] = serde_json::json!("A changed fictional phrase.");
    fs::write(&catalog, serde_json::to_vec(&value).unwrap()).unwrap();
    server.add_route(
        "POST",
        "/v1/text-to-speech/voice-fixture",
        HttpResponse::new(200, "replacement-audio"),
    );
    server.add_route(
        "POST",
        "/v1/text-to-speech/voice-second",
        HttpResponse::new(503, "unavailable"),
    );
    let before = fs::read(&catalog).unwrap();
    let old_audio_path = temp
        .path()
        .join(value["fixtures"][0]["file"].as_str().unwrap());
    let old_audio = fs::read(&old_audio_path).unwrap();
    let failed = run(
        &["generate", "--catalog", catalog.to_str().unwrap()],
        &CmdOptions::default()
            .with_env("ELEVENLABS_API_KEY", "stub-secret")
            .with_env("SPEECH_TEST_ELEVENLABS_BASE_URL", &base),
    );
    assert_eq!(failed.code, 69);
    assert_eq!(fs::read(&catalog).unwrap(), before);
    assert_eq!(fs::read(&old_audio_path).unwrap(), old_audio);
    let retained: serde_json::Value = serde_json::from_slice(&fs::read(&catalog).unwrap()).unwrap();
    assert!(retained["fixtures"][0]["review"].is_object());
    assert_eq!(
        sha2::Sha256::digest(&old_audio)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>(),
        retained["fixtures"][0]["derived_sha256"]
    );
    assert_eq!(server.take_requests().len(), 2);
}

#[cfg(unix)]
#[test]
fn generation_rejects_symlinked_cache_before_provider_request() {
    use std::os::unix::fs::symlink;

    let temp = TempDir::new().unwrap();
    let catalog = reviewed_catalog(&temp, "sample.wav");
    configure_tts(&catalog, serde_json::json!({}));
    let request = TtsRequest {
        provider: "elevenlabs".into(),
        api_version: "v1".into(),
        model: "model-fixture".into(),
        voice: "voice-fixture".into(),
        text: "A fictional test phrase.".into(),
        voice_settings: serde_json::json!({}),
        speed: None,
        seed: None,
        language: None,
        normalization: None,
        output_format: "mp3_44100_128".into(),
    };
    let fingerprint = canonical_fingerprint(&request).unwrap();
    let poisoned = b"poisoned cache target";
    let outside = temp.path().join("outside.txt");
    fs::write(&outside, poisoned).unwrap();
    let cache = temp.path().join("cache");
    fs::create_dir_all(&cache).unwrap();
    fs::write(
        cache.join(format!("{fingerprint}.json")),
        serde_json::to_vec(&serde_json::json!({
            "fingerprint": fingerprint,
            "sha256": sha256(poisoned)
        }))
        .unwrap(),
    )
    .unwrap();
    symlink(&outside, cache.join(format!("{fingerprint}.audio"))).unwrap();

    let server = LoopbackServer::new().unwrap();
    let base = server.url();
    let output = run(
        &[
            "--format",
            "json",
            "generate",
            "--catalog",
            catalog.to_str().unwrap(),
            "--cache-dir",
            cache.to_str().unwrap(),
        ],
        &CmdOptions::default()
            .with_env("ELEVENLABS_API_KEY", "stub-secret")
            .with_env("SPEECH_TEST_ELEVENLABS_BASE_URL", &base),
    );
    assert_eq!(output.code, 65);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&output.stdout_text()).unwrap()["error"]["code"],
        "catalog-invalid"
    );
    assert_eq!(fs::read(&outside).unwrap(), poisoned);
    assert!(
        fs::symlink_metadata(cache.join(format!("{fingerprint}.audio")))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert!(server.take_requests().is_empty());
}

#[cfg(unix)]
#[test]
fn generation_rejects_symlinked_catalog_before_provider_request() {
    use std::os::unix::fs::symlink;

    let temp = TempDir::new().unwrap();
    let catalog = reviewed_catalog(&temp, "sample.wav");
    configure_tts(&catalog, serde_json::json!({}));
    let target_catalog = temp.path().join("catalog-target.json");
    fs::rename(&catalog, &target_catalog).unwrap();
    symlink(&target_catalog, &catalog).unwrap();
    let before = fs::read(&target_catalog).unwrap();

    let server = LoopbackServer::new().unwrap();
    server.add_route(
        "POST",
        "/v1/text-to-speech/voice-fixture",
        HttpResponse::new(200, "synthetic-audio"),
    );
    let base = server.url();
    let output = run(
        &[
            "--format",
            "json",
            "generate",
            "--catalog",
            catalog.to_str().unwrap(),
        ],
        &CmdOptions::default()
            .with_env("ELEVENLABS_API_KEY", "stub-secret")
            .with_env("SPEECH_TEST_ELEVENLABS_BASE_URL", &base),
    );
    assert_eq!(output.code, 65);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&output.stdout_text()).unwrap()["error"]["code"],
        "catalog-invalid"
    );
    assert!(server.take_requests().is_empty());
    assert_eq!(fs::read(&target_catalog).unwrap(), before);
}

#[test]
fn replay_checks_run_output_writability_before_asr_requests() {
    let temp = TempDir::new().unwrap();
    let catalog = reviewed_catalog(&temp, "sample.wav");
    let blocking_file = temp.path().join("not-a-directory");
    fs::write(&blocking_file, b"self-made").unwrap();
    let output_path = blocking_file.join("run.json");
    let server = LoopbackServer::new().unwrap();
    server.add_route(
        "POST",
        "/asr",
        HttpResponse::new(200, r#"{"text":"ignored"}"#),
    );
    let endpoint = format!("{}/asr", server.url());
    let output = run(
        &[
            "replay",
            "--catalog",
            catalog.to_str().unwrap(),
            "--endpoint",
            &endpoint,
            "--run-output",
            output_path.to_str().unwrap(),
        ],
        &CmdOptions::default(),
    );
    assert_ne!(output.code, 0);
    assert!(server.take_requests().is_empty());
}

#[test]
fn tts_timeout_warns_possibly_charged_and_is_never_retried() {
    let temp = TempDir::new().unwrap();
    let catalog = reviewed_catalog(&temp, "sample.wav");
    configure_tts(&catalog, serde_json::json!({}));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let requests = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&requests);
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        seen.fetch_add(1, Ordering::SeqCst);
        thread::sleep(Duration::from_millis(1300));
        let _ = stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\naudio");
    });
    let cache = temp.path().join("cache");
    let output = run(
        &[
            "--format",
            "json",
            "generate",
            "--catalog",
            catalog.to_str().unwrap(),
            "--cache-dir",
            cache.to_str().unwrap(),
            "--request-deadline-seconds",
            "1",
        ],
        &CmdOptions::default()
            .with_env("ELEVENLABS_API_KEY", "stub-secret")
            .with_env("SPEECH_TEST_ELEVENLABS_BASE_URL", &base),
    );
    assert_eq!(output.code, 69);
    let value: serde_json::Value = serde_json::from_str(&output.stdout_text()).unwrap();
    assert_eq!(value["error"]["code"], "tts-timeout-possibly-charged");
    assert!(
        value["error"]["message"]
            .as_str()
            .unwrap()
            .contains("may have charged")
    );
    server.join().unwrap();
    assert_eq!(requests.load(Ordering::SeqCst), 1);
}

#[test]
fn tts_body_timeout_is_possibly_charged_and_not_retried() {
    let temp = TempDir::new().unwrap();
    let catalog = reviewed_catalog(&temp, "sample.wav");
    configure_tts(&catalog, serde_json::json!({}));
    let (base, requests, server) = start_stalled_body_server();
    let output = run(
        &[
            "--format",
            "json",
            "generate",
            "--catalog",
            catalog.to_str().unwrap(),
            "--request-deadline-seconds",
            "1",
        ],
        &CmdOptions::default()
            .with_env("ELEVENLABS_API_KEY", "stub-secret")
            .with_env("SPEECH_TEST_ELEVENLABS_BASE_URL", &base),
    );
    assert_eq!(output.code, 69);
    let value: serde_json::Value = serde_json::from_str(&output.stdout_text()).unwrap();
    assert_eq!(value["error"]["code"], "tts-timeout-possibly-charged");
    assert!(
        value["error"]["message"]
            .as_str()
            .unwrap()
            .contains("may have charged")
    );
    server.join().unwrap();
    assert_eq!(requests.load(Ordering::SeqCst), 1);
}

#[test]
fn asr_body_timeout_keeps_timeout_classification_and_is_not_retried() {
    let temp = TempDir::new().unwrap();
    let catalog = reviewed_catalog(&temp, "sample.wav");
    let (base, requests, server) = start_stalled_body_server();
    let output = run(
        &[
            "--format",
            "json",
            "replay",
            "--catalog",
            catalog.to_str().unwrap(),
            "--endpoint",
            &format!("{base}/asr"),
            "--deadline-seconds",
            "1",
        ],
        &CmdOptions::default(),
    );
    assert_eq!(output.code, 69);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&output.stdout_text()).unwrap()["error"]["code"],
        "asr-timeout"
    );
    server.join().unwrap();
    assert_eq!(requests.load(Ordering::SeqCst), 1);
}

#[test]
fn replay_maps_asr_401_429_and_malformed_json_without_retry() {
    for (status, body, expected_code, exit_code) in [
        (401, "unauthorized", "asr-unauthorized", 69),
        (429, "limited", "asr-rate-limited", 69),
        (200, "not-json", "malformed-json", 65),
    ] {
        let temp = TempDir::new().unwrap();
        let catalog = reviewed_catalog(&temp, "sample.wav");
        let server = LoopbackServer::new().unwrap();
        server.add_route("POST", "/asr", HttpResponse::new(status, body));
        let endpoint = format!("{}/asr", server.url());
        let output = run(
            &[
                "--format",
                "json",
                "replay",
                "--catalog",
                catalog.to_str().unwrap(),
                "--endpoint",
                &endpoint,
            ],
            &CmdOptions::default(),
        );
        assert_eq!(output.code, exit_code);
        let value: serde_json::Value = serde_json::from_str(&output.stdout_text()).unwrap();
        assert_eq!(value["error"]["code"], expected_code);
        assert_eq!(
            server.take_requests().len(),
            1,
            "requests are never retried"
        );
    }
}

#[test]
fn replay_deadline_times_out_without_retry() {
    let temp = TempDir::new().unwrap();
    let catalog = reviewed_catalog(&temp, "sample.wav");
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}/asr", listener.local_addr().unwrap());
    let requests = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&requests);
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        seen.fetch_add(1, Ordering::SeqCst);
        thread::sleep(Duration::from_millis(1300));
        let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 11\r\nConnection: close\r\n\r\n{\"text\":\"x\"}");
    });
    let output = run(
        &[
            "--format",
            "json",
            "replay",
            "--catalog",
            catalog.to_str().unwrap(),
            "--endpoint",
            &endpoint,
            "--deadline-seconds",
            "1",
        ],
        &CmdOptions::default(),
    );
    assert_eq!(output.code, 69);
    let value: serde_json::Value = serde_json::from_str(&output.stdout_text()).unwrap();
    assert_eq!(value["error"]["code"], "asr-timeout");
    server.join().unwrap();
    assert_eq!(requests.load(Ordering::SeqCst), 1);
}
