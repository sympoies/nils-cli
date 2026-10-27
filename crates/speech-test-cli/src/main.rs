use clap::{CommandFactory, Parser, Subcommand, ValueEnum};
use clap_complete::{Shell, generate as generate_completion};
use nils_common::cli_contract::{
    Envelope, EnvelopeError, OutputFormat, emit_parse_error, exit, schema_version_for,
};
use nils_speech_test_cli::{
    FixtureScore, ResponseService, SpeechError, TtsRequest, canonical_fingerprint,
    check_writable_directory, correctness_checks, edit_counts, percentile, read_bounded,
    read_catalog, resolve_catalog_file, sha256, sha256_file, validate_catalog_update_target,
    validate_credential_url, validate_directory_path, verified_audio_path, write_atomic,
    write_catalog,
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Instant;

const BINARY: &str = "speech-test-cli";
const MAX_TTS_AUDIO_BYTES: usize = 64 * 1024 * 1024;
const MAX_ASR_JSON_BYTES: usize = 2 * 1024 * 1024;

#[derive(Parser)]
#[command(name = BINARY, version, long_version = nils_build_info::long_version(env!("CARGO_PKG_VERSION")), about = "Generate reviewed speech fixtures and replay them against ASR endpoints")]
struct Cli {
    #[arg(long, global = true, value_enum)]
    format: Option<OutputFormat>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// List TTS requests and estimated character cost; --plan never calls the provider.
    Generate {
        /// Path to the fixture catalog file.
        #[arg(long)]
        catalog: PathBuf,
        #[arg(long, requires = "cost_per_1000_characters")]
        plan: bool,
        #[arg(long)]
        max_characters: Option<usize>,
        #[arg(long)]
        cost_per_1000_characters: Option<f64>,
        #[arg(long)]
        cache_dir: Option<PathBuf>,
        #[arg(long, default_value = "60")]
        request_deadline_seconds: u64,
    },
    /// Record a human review and spoken reference for a generated fixture.
    Review {
        /// Path to the fixture catalog file.
        #[arg(long)]
        catalog: PathBuf,
        #[arg(long)]
        id: String,
        #[arg(long)]
        reference: String,
        #[arg(long)]
        reviewer: String,
    },
    /// Replay verified, reviewed fixture audio against one ASR HTTP endpoint.
    Replay {
        /// Path to the fixture catalog file.
        #[arg(long)]
        catalog: PathBuf,
        #[arg(long)]
        endpoint: String,
        #[arg(long, default_value = "30")]
        deadline_seconds: u64,
        #[arg(long)]
        auth_env: Option<String>,
        #[arg(long, value_parser = ["raw", "multipart"], default_value = "raw")]
        body: String,
        #[arg(long, default_value = "audio")]
        multipart_field: String,
        #[arg(long)]
        run_output: Option<PathBuf>,
        #[arg(long)]
        normalize: bool,
    },
    /// Re-emit scores from a saved run without network access.
    Score {
        #[arg(long)]
        run: PathBuf,
    },
    /// Print generated shell completions.
    Completion {
        #[arg(value_enum)]
        shell: CompletionShell,
    },
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum CompletionShell {
    Bash,
    Zsh,
}

impl From<CompletionShell> for Shell {
    fn from(value: CompletionShell) -> Self {
        match value {
            CompletionShell::Bash => Shell::Bash,
            CompletionShell::Zsh => Shell::Zsh,
        }
    }
}

#[derive(Serialize)]
struct PlanItem {
    id: String,
    request: TtsRequest,
    characters: usize,
    fingerprint: String,
    estimated_cost_usd: Option<f64>,
    cache_hit: bool,
}

struct GenerationPlan {
    id: String,
    request: TtsRequest,
    audio_format: AudioFormat,
    fingerprint: String,
    cached: Option<CacheAudio>,
    preserved_file: Option<(String, u64)>,
    output_reservation: Option<tempfile::TempPath>,
}

struct ReplayOptions<'a> {
    endpoint: &'a str,
    deadline: u64,
    auth_env: Option<&'a str>,
    body: &'a str,
    field: &'a str,
    run_output: Option<PathBuf>,
    normalize: bool,
}

#[derive(Clone, Copy)]
struct AudioFormat {
    codec: &'static str,
    extension: &'static str,
    sample_rate_hz: u32,
    bitrate_kbps: Option<u32>,
}

#[derive(Serialize, Deserialize)]
struct SavedRun {
    schema_version: String,
    scores: Vec<FixtureScore>,
    latency_p50_ms: u64,
    latency_p95_ms: u64,
    real_time_factor: f64,
}

fn error_code(err: &SpeechError) -> (&'static str, i32) {
    match err {
        SpeechError::CatalogRead(_) => ("catalog-read-failed", exit::DATA),
        SpeechError::RunRead(_) => ("run-read-failed", exit::DATA),
        SpeechError::FileMissing(_) => ("fixture-missing", exit::DATA),
        SpeechError::CatalogInvalid(_) => ("catalog-invalid", exit::DATA),
        SpeechError::FixtureMissing(_) => ("fixture-not-found", exit::DATA),
        SpeechError::RunInvalid(_) => ("run-invalid", exit::DATA),
        SpeechError::Configuration(_) => ("configuration-error", exit::UNAVAILABLE),
        SpeechError::InsecureCredentialTransport => ("insecure-credential-transport", exit::USAGE),
        SpeechError::ResponseTooLarge(_) => ("response-too-large", exit::UNAVAILABLE),
        SpeechError::TtsResponseTooLarge => ("tts-response-too-large", exit::UNAVAILABLE),
        SpeechError::ScoreInputTooLarge => ("score-input-too-large", exit::DATA),
        SpeechError::CacheInvalid => ("cache-invalid", exit::DATA),
        SpeechError::HashMismatch(_) => ("fixture-hash-mismatch", exit::DATA),
        SpeechError::Unreviewed(_) => ("fixture-unreviewed", exit::DATA),
        SpeechError::Budget { .. } => ("character-budget-exceeded", exit::DATA),
        SpeechError::HttpStatus(code) if *code == 401 => ("asr-unauthorized", exit::UNAVAILABLE),
        SpeechError::HttpStatus(code) if *code == 429 => ("asr-rate-limited", exit::UNAVAILABLE),
        SpeechError::TtsStatus(code) if *code == 401 => ("tts-unauthorized", exit::UNAVAILABLE),
        SpeechError::TtsStatus(code) if *code == 429 => ("tts-rate-limited", exit::UNAVAILABLE),
        SpeechError::TtsStatus(_) => ("tts-http-status", exit::UNAVAILABLE),
        SpeechError::TtsTimeout => ("tts-timeout-possibly-charged", exit::UNAVAILABLE),
        SpeechError::AsrTimeout => ("asr-timeout", exit::UNAVAILABLE),
        SpeechError::HttpStatus(_) => ("http-status", exit::UNAVAILABLE),
        SpeechError::MalformedJson => ("malformed-json", exit::DATA),
        SpeechError::Http(_) => ("http-error", exit::UNAVAILABLE),
        _ => ("operation-failed", exit::SOFTWARE),
    }
}

fn emit_error(format: OutputFormat, command: &str, err: &SpeechError) -> i32 {
    let (code, status) = error_code(err);
    let message = err.to_string().replace(['\n', '\r'], " ");
    if format == OutputFormat::Json {
        let envelope = Envelope::<serde_json::Value>::failure(
            schema_version_for(BINARY, command, 1),
            EnvelopeError::new(code, message),
        );
        println!(
            "{}",
            serde_json::to_string(&envelope).unwrap_or_else(|_| "{}".into())
        );
    } else {
        eprintln!("error: {message}");
    }
    status
}

fn emit<T: Serialize>(format: OutputFormat, command: &str, data: &T) -> i32 {
    if format == OutputFormat::Json {
        let envelope = Envelope::success(schema_version_for(BINARY, command, 1), data);
        match serde_json::to_string(&envelope) {
            Ok(text) => println!("{text}"),
            Err(_) => return exit::SOFTWARE,
        }
    } else {
        let value = match serde_json::to_value(data) {
            Ok(value) => value,
            Err(_) => return exit::SOFTWARE,
        };
        println!("{command}:");
        if let Some(fields) = value.as_object() {
            for (name, item) in fields {
                if let Some(items) = item.as_array() {
                    println!("  {name}:");
                    for entry in items {
                        println!("    - {}", entry);
                    }
                } else {
                    println!("  {name}: {item}");
                }
            }
        } else {
            println!("  {value}");
        }
    }
    exit::SUCCESS
}

fn parse_output_format(value: &str) -> Result<AudioFormat, SpeechError> {
    let parts = value.split('_').collect::<Vec<_>>();
    match parts.as_slice() {
        ["mp3", rate, bitrate] => {
            let sample_rate_hz = rate
                .parse::<u32>()
                .ok()
                .filter(|value| *value > 0)
                .ok_or_else(|| SpeechError::CatalogInvalid("invalid MP3 sample rate".into()))?;
            let bitrate_kbps = bitrate
                .parse::<u32>()
                .ok()
                .filter(|value| *value > 0)
                .ok_or_else(|| SpeechError::CatalogInvalid("invalid MP3 bitrate".into()))?;
            Ok(AudioFormat {
                codec: "mp3",
                extension: "mp3",
                sample_rate_hz,
                bitrate_kbps: Some(bitrate_kbps),
            })
        }
        ["pcm", rate] => {
            let sample_rate_hz = rate
                .parse::<u32>()
                .ok()
                .filter(|value| *value > 0)
                .ok_or_else(|| SpeechError::CatalogInvalid("invalid PCM sample rate".into()))?;
            Ok(AudioFormat {
                codec: "pcm_s16le",
                extension: "pcm",
                sample_rate_hz,
                bitrate_kbps: None,
            })
        }
        _ => Err(SpeechError::CatalogInvalid(
            "output_format must be mp3_<sample_rate>_<bitrate> or pcm_<sample_rate>".into(),
        )),
    }
}

fn tts_request(fixture: &nils_speech_test_cli::Fixture) -> TtsRequest {
    let settings = &fixture.generation.settings;
    TtsRequest {
        provider: fixture.generation.provider.clone(),
        api_version: settings["api_version"].as_str().unwrap_or("v1").into(),
        model: fixture.generation.model.clone(),
        voice: fixture.generation.voice.clone(),
        text: fixture.intended_text.clone(),
        voice_settings: settings
            .get("voice_settings")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({})),
        speed: settings["speed"].as_f64(),
        seed: settings["seed"].as_u64(),
        language: settings["language"].as_str().map(str::to_string),
        normalization: settings["normalization"].as_str().map(str::to_string),
        output_format: settings["output_format"]
            .as_str()
            .unwrap_or("mp3_44100_128")
            .into(),
    }
}

struct CacheAudio {
    path: PathBuf,
    sha256: String,
}

fn cached_audio_info(cache_dir: &Path, fingerprint: &str) -> Option<CacheAudio> {
    let cache_file = cache_dir.join(format!("{fingerprint}.json"));
    let metadata = std::fs::symlink_metadata(&cache_file).ok()?;
    if !metadata.file_type().is_file() || metadata.len() > 4096 {
        return None;
    }
    let entry: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&cache_file).ok()?).ok()?;
    if entry["fingerprint"].as_str()? != fingerprint {
        return None;
    }
    let expected_hash = entry["sha256"].as_str()?.to_string();
    let audio_path = cache_dir.join(format!("{fingerprint}.audio"));
    let audio_metadata = std::fs::symlink_metadata(&audio_path).ok()?;
    if !audio_metadata.file_type().is_file() || audio_metadata.len() > MAX_TTS_AUDIO_BYTES as u64 {
        return None;
    }
    if sha256_file(&audio_path).ok()?.as_str() != expected_hash {
        return None;
    }
    Some(CacheAudio {
        path: audio_path,
        sha256: expected_hash,
    })
}

fn read_cached_audio(cache: &CacheAudio) -> Result<Vec<u8>, SpeechError> {
    let audio = std::fs::read(&cache.path).map_err(|_| SpeechError::CacheInvalid)?;
    if audio.len() > MAX_TTS_AUDIO_BYTES || sha256(&audio) != cache.sha256 {
        return Err(SpeechError::CacheInvalid);
    }
    Ok(audio)
}
fn validate_cache_targets(
    cache_dir: &Path,
    requests: &[(String, TtsRequest, AudioFormat)],
) -> Result<(), SpeechError> {
    for (_, request, _) in requests {
        let fingerprint = canonical_fingerprint(request)?;
        for path in [
            cache_dir.join(format!("{fingerprint}.json")),
            cache_dir.join(format!("{fingerprint}.audio")),
        ] {
            if let Ok(metadata) = std::fs::symlink_metadata(&path)
                && !metadata.file_type().is_file()
            {
                return Err(SpeechError::CatalogInvalid(
                    "cache entries must be regular files, not symlinks or directories".into(),
                ));
            }
        }
    }
    Ok(())
}

fn cleanup_created_files(paths: &[PathBuf]) {
    for path in paths {
        let _ = std::fs::remove_file(path);
    }
}

fn generate(
    catalog_path: &Path,
    plan: bool,
    max_characters: Option<usize>,
    rate: Option<f64>,
    cache_override: Option<PathBuf>,
    request_deadline_seconds: u64,
    format: OutputFormat,
) -> i32 {
    let result = (|| {
        validate_catalog_update_target(catalog_path)?;
        let mut catalog = read_catalog(catalog_path)?;
        let mut requests = Vec::new();
        for fixture in &catalog.fixtures {
            let request = tts_request(fixture);
            if request.provider != "elevenlabs" {
                return Err(SpeechError::CatalogInvalid(format!(
                    "unsupported TTS provider for fixture {}",
                    fixture.id
                )));
            }
            if !request.voice_settings.is_object() {
                return Err(SpeechError::CatalogInvalid(
                    "voice_settings must be an object".into(),
                ));
            }
            if request.api_version.is_empty()
                || !request
                    .api_version
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || ch == '-')
            {
                return Err(SpeechError::CatalogInvalid("invalid API version".into()));
            }
            if request.seed.is_some_and(|seed| seed > u32::MAX as u64) {
                return Err(SpeechError::CatalogInvalid(
                    "seed exceeds the provider limit".into(),
                ));
            }
            let audio_format = parse_output_format(&request.output_format)?;
            requests.push((fixture.id.clone(), request, audio_format));
        }
        if rate.is_some_and(|value| !value.is_finite() || value < 0.0) {
            return Err(SpeechError::CatalogInvalid(
                "cost rate must be a finite non-negative number".into(),
            ));
        }
        let characters = requests
            .iter()
            .map(|(_, request, _)| request.text.chars().count())
            .sum::<usize>();
        if let Some(maximum) = max_characters
            && characters > maximum
        {
            return Err(SpeechError::Budget {
                actual: characters,
                maximum,
            });
        }
        let cache_dir = cache_override.unwrap_or_else(|| {
            catalog_path
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .join(".speech-test-cache")
        });
        validate_directory_path(&cache_dir)?;
        if plan {
            let mut plans = Vec::new();
            for (fixture_id, request, _) in &requests {
                let fingerprint = canonical_fingerprint(request)?;
                let cache_hit = cached_audio_info(&cache_dir, &fingerprint).is_some();
                let estimated_cost_usd =
                    rate.map(|value| request.text.chars().count() as f64 * value / 1000.0);
                plans.push(PlanItem {
                    id: fixture_id.clone(),
                    request: request.clone(),
                    characters: request.text.chars().count(),
                    fingerprint,
                    estimated_cost_usd,
                    cache_hit,
                });
            }
            return Ok(serde_json::json!({
                "requests": plans,
                "characters": characters,
                "estimated_cost_usd": rate.map(|value| characters as f64 * value / 1000.0)
            }));
        }

        let catalog_dir = catalog_path.parent().unwrap_or_else(|| Path::new("."));
        let generated_dir = catalog_dir.join(".speech-test-generated");
        check_writable_directory(&cache_dir)?;
        check_writable_directory(catalog_dir)?;
        check_writable_directory(&generated_dir)?;
        validate_cache_targets(&cache_dir, &requests)?;

        // Resolve cache and existing fixture state, and reserve every required output before TTS.
        let mut generation_plans = Vec::new();
        for (id, request, audio_format) in requests {
            let fingerprint = canonical_fingerprint(&request)?;
            let cached = cached_audio_info(&cache_dir, &fingerprint);
            let fixture = catalog
                .fixtures
                .iter()
                .find(|fixture| fixture.id == id)
                .expect("preflight catalog fixture");
            let current_path = resolve_catalog_file(catalog_path, &fixture.file)?;
            let preserved_file = cached.as_ref().and_then(|cache| {
                let content_hash = sha256_file(&current_path).ok()?;
                if content_hash != cache.sha256 {
                    return None;
                }
                let length = std::fs::metadata(&current_path).ok()?.len();
                Some((fixture.file.clone(), length))
            });
            let output_reservation = if preserved_file.is_none() {
                let prefix = format!("{id}-");
                let suffix = format!(".{}", audio_format.extension);
                Some(
                    tempfile::Builder::new()
                        .prefix(&prefix)
                        .suffix(&suffix)
                        .tempfile_in(&generated_dir)
                        .map(tempfile::NamedTempFile::into_temp_path)
                        .map_err(|error| SpeechError::Other(error.to_string()))?,
                )
            } else {
                None
            };
            generation_plans.push(GenerationPlan {
                id,
                request,
                audio_format,
                fingerprint,
                cached,
                preserved_file,
                output_reservation,
            });
        }

        let mut key: Option<String> = None;
        let base = std::env::var("SPEECH_TEST_ELEVENLABS_BASE_URL")
            .unwrap_or_else(|_| "https://api.elevenlabs.io".into());
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(request_deadline_seconds))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|error| SpeechError::Http(error.without_url().to_string()))?;
        let mut generated = 0usize;
        let mut pending_outputs: Vec<(tempfile::TempPath, PathBuf, String)> = Vec::new();
        for mut item in generation_plans {
            let (hash, audio_len, generated_file) = if let Some((path, length)) =
                item.preserved_file
            {
                let hash = item
                    .cached
                    .as_ref()
                    .expect("preserved fixture came from cache")
                    .sha256
                    .clone();
                (hash, length, path)
            } else {
                let cached = item
                    .cached
                    .take()
                    .or_else(|| cached_audio_info(&cache_dir, &item.fingerprint));
                let audio = match cached.as_ref() {
                    Some(cache) => read_cached_audio(cache)?,
                    None => {
                        if key.is_none() {
                            key = Some(std::env::var("ELEVENLABS_API_KEY").map_err(|_| {
                                SpeechError::Configuration(
                                    "ELEVENLABS_API_KEY is required for a TTS cache miss".into(),
                                )
                            })?);
                        }
                        let api_key = key.as_deref().expect("key was just set");
                        let api_root = validate_credential_url(&base)?;
                        if api_root.query().is_some() || api_root.fragment().is_some() {
                            return Err(SpeechError::Configuration(
                                "TTS API base URL must not contain a query or fragment".into(),
                            ));
                        }
                        let mut url = api_root;
                        {
                            let mut segments = url.path_segments_mut().map_err(|_| {
                                SpeechError::Configuration("invalid TTS endpoint base URL".into())
                            })?;
                            segments
                                .pop_if_empty()
                                .push(&item.request.api_version)
                                .push("text-to-speech")
                                .push(&item.request.voice);
                        }
                        let mut voice_settings = item.request.voice_settings.clone();
                        if let Some(speed) = item.request.speed {
                            voice_settings
                                .as_object_mut()
                                .expect("voice_settings validated before request")
                                .insert("speed".into(), serde_json::json!(speed));
                        }
                        let response = client
                            .post(url)
                            .query(&[("output_format", item.request.output_format.as_str())])
                            .header("xi-api-key", api_key)
                            .json(&serde_json::json!({
                                "text": item.request.text,
                                "model_id": item.request.model,
                                "voice_settings": voice_settings,
                                "seed": item.request.seed,
                                "language_code": item.request.language,
                                "apply_text_normalization": item.request.normalization
                            }))
                            .send()
                            .map_err(|error| {
                                if error.is_timeout() {
                                    SpeechError::TtsTimeout
                                } else {
                                    SpeechError::Http(error.without_url().to_string())
                                }
                            })?;
                        if !response.status().is_success() {
                            return Err(SpeechError::TtsStatus(response.status().as_u16()));
                        }
                        if response
                            .content_length()
                            .is_some_and(|length| length > MAX_TTS_AUDIO_BYTES as u64)
                        {
                            return Err(SpeechError::TtsResponseTooLarge);
                        }
                        let audio = match read_bounded(
                            response,
                            MAX_TTS_AUDIO_BYTES,
                            ResponseService::TtsAudio,
                        ) {
                            Err(SpeechError::ResponseTooLarge(_)) => {
                                return Err(SpeechError::TtsResponseTooLarge);
                            }
                            result => result?,
                        };
                        write_atomic(
                            &cache_dir.join(format!("{}.audio", item.fingerprint)),
                            &audio,
                        )?;
                        write_atomic(
                            &cache_dir.join(format!("{}.json", item.fingerprint)),
                            &serde_json::to_vec(&serde_json::json!({
                                "fingerprint": item.fingerprint,
                                "sha256": sha256(&audio)
                            }))
                            .map_err(|error| SpeechError::Other(error.to_string()))?,
                        )?;
                        audio
                    }
                };
                let hash = sha256(&audio);
                let output_reservation = item
                    .output_reservation
                    .take()
                    .expect("non-preserved fixture reserved an output path");
                write_atomic(&output_reservation, &audio)?;
                let extension = item.audio_format.extension;
                let target = generated_dir.join(format!(
                    "{}-{}-{}.{}",
                    item.id, item.fingerprint, hash, extension
                ));
                let generated_file = Path::new(".speech-test-generated")
                    .join(target.file_name().expect("generated filename"))
                    .to_string_lossy()
                    .into_owned();
                pending_outputs.push((output_reservation, target, hash.clone()));
                (hash, audio.len() as u64, generated_file)
            };

            let fixture = catalog
                .fixtures
                .iter_mut()
                .find(|fixture| fixture.id == item.id)
                .expect("preflight catalog fixture");
            let changed_audio = fixture.derived_sha256 != hash
                || fixture.codec != item.audio_format.codec
                || fixture.sample_rate_hz != item.audio_format.sample_rate_hz
                || fixture.channels != 1;
            if changed_audio {
                fixture.review = None;
                fixture.spoken_reference = None;
            }
            fixture.file = generated_file;
            fixture.master_sha256 = hash.clone();
            fixture.derived_sha256 = hash;
            fixture.codec = item.audio_format.codec.into();
            fixture.channels = 1;
            fixture.sample_rate_hz = item.audio_format.sample_rate_hz;
            fixture.duration_seconds = if let Some(bitrate) = item.audio_format.bitrate_kbps {
                audio_len as f64 * 8.0 / (bitrate as f64 * 1000.0)
            } else {
                audio_len as f64 / (item.audio_format.sample_rate_hz as f64 * 2.0)
            };
            generated += 1;
        }

        let mut persisted_outputs = Vec::new();
        for (reservation, target, expected_hash) in pending_outputs {
            match reservation.persist_noclobber(&target) {
                Ok(_) => persisted_outputs.push(target),
                Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
                    let metadata = std::fs::symlink_metadata(&target)
                        .map_err(|error| SpeechError::Other(error.to_string()))?;
                    if !metadata.file_type().is_file() || sha256_file(&target)? != expected_hash {
                        cleanup_created_files(&persisted_outputs);
                        return Err(SpeechError::Other(
                            "generated output path exists with different content".into(),
                        ));
                    }
                }
                Err(error) => {
                    cleanup_created_files(&persisted_outputs);
                    return Err(SpeechError::Other(error.error.to_string()));
                }
            }
        }
        if let Err(error) = write_catalog(catalog_path, &catalog) {
            cleanup_created_files(&persisted_outputs);
            return Err(error);
        }
        Ok(serde_json::json!({
            "generated": generated,
            "characters": characters,
            "cache_dir": cache_dir.display().to_string()
        }))
    })();
    match result {
        Ok(data) => emit(format, "generate", &data),
        Err(error) => emit_error(format, "generate", &error),
    }
}
fn review(
    catalog_path: &Path,
    id: &str,
    reference: &str,
    reviewer: &str,
    format: OutputFormat,
) -> i32 {
    let result = (|| {
        if reference.trim().is_empty() || reviewer.trim().is_empty() {
            return Err(SpeechError::CatalogInvalid(
                "reviewer and spoken reference must not be empty".into(),
            ));
        }
        let mut catalog = read_catalog(catalog_path)?;
        let fixture = catalog
            .fixtures
            .iter_mut()
            .find(|f| f.id == id)
            .ok_or_else(|| SpeechError::FixtureMissing(id.into()))?;
        let path = resolve_catalog_file(catalog_path, &fixture.file)?;
        let audio = std::fs::read(&path)
            .map_err(|_| SpeechError::FileMissing(path.display().to_string()))?;
        if sha256(&audio) != fixture.derived_sha256 {
            return Err(SpeechError::HashMismatch(path.display().to_string()));
        }
        fixture.spoken_reference = Some(reference.into());
        fixture.review = Some(nils_speech_test_cli::Review {
            reviewer: reviewer.into(),
            reviewed_at: jiff::Timestamp::now().to_string(),
        });
        write_catalog(catalog_path, &catalog)?;
        Ok(serde_json::json!({"id":id,"reviewed":true}))
    })();
    match result {
        Ok(data) => emit(format, "review", &data),
        Err(e) => emit_error(format, "review", &e),
    }
}

fn replay(catalog_path: &Path, options: ReplayOptions<'_>, format: OutputFormat) -> i32 {
    let ReplayOptions {
        endpoint,
        deadline,
        auth_env,
        body,
        field,
        run_output,
        normalize,
    } = options;
    let result = (|| {
        let catalog = read_catalog(catalog_path)?;
        let mut verified = Vec::new();
        for fixture in &catalog.fixtures {
            let path = verified_audio_path(catalog_path, fixture)?;
            verified.push((fixture, path));
        }
        let token = auth_env
            .map(|name| {
                std::env::var(name).map_err(|_| {
                    SpeechError::Configuration(format!("environment variable {name} is not set"))
                })
            })
            .transpose()?;
        let parsed_endpoint = reqwest::Url::parse(endpoint).map_err(|error| {
            SpeechError::Configuration(format!("invalid ASR endpoint: {error}"))
        })?;
        if !matches!(parsed_endpoint.scheme(), "http" | "https")
            || !parsed_endpoint.username().is_empty()
            || parsed_endpoint.password().is_some()
        {
            return Err(SpeechError::Configuration(
                "ASR endpoint must use HTTP or HTTPS without URL credentials".into(),
            ));
        }
        let endpoint = if token.is_some() {
            validate_credential_url(endpoint)?
        } else {
            parsed_endpoint
        };
        let output = run_output.unwrap_or_else(|| {
            catalog_path
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .join("speech-test-run.json")
        });
        if std::fs::symlink_metadata(&output).is_ok_and(|metadata| metadata.file_type().is_dir()) {
            return Err(SpeechError::CatalogInvalid(
                "run output path must name a file".into(),
            ));
        }
        let output_parent = output.parent().unwrap_or_else(|| Path::new("."));
        check_writable_directory(output_parent)?;
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(deadline))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|error| SpeechError::Http(error.to_string()))?;
        let mut scores = Vec::new();
        let mut latencies = Vec::new();
        let mut rtf = 0.0;
        for (fixture, path) in verified {
            let audio = std::fs::read(&path)
                .map_err(|_| SpeechError::FileMissing(path.display().to_string()))?;
            if sha256(&audio) != fixture.derived_sha256 {
                return Err(SpeechError::HashMismatch(path.display().to_string()));
            }
            let start = Instant::now();
            let mut request = client.post(endpoint.clone());
            if let Some(token) = &token {
                request = request.bearer_auth(token);
            }
            let response = if body == "multipart" {
                let part = reqwest::blocking::multipart::Part::bytes(audio).file_name(
                    path.file_name()
                        .and_then(|value| value.to_str())
                        .unwrap_or("fixture.audio")
                        .to_string(),
                );
                request
                    .multipart(
                        reqwest::blocking::multipart::Form::new().part(field.to_string(), part),
                    )
                    .send()
            } else {
                request
                    .header("content-type", "application/octet-stream")
                    .body(audio)
                    .send()
            }
            .map_err(|error| {
                if error.is_timeout() {
                    SpeechError::AsrTimeout
                } else {
                    SpeechError::Http(error.without_url().to_string())
                }
            })?;
            if !response.status().is_success() {
                return Err(SpeechError::HttpStatus(response.status().as_u16()));
            }
            let backend = response
                .headers()
                .get("x-backend")
                .and_then(|value| value.to_str().ok())
                .map(str::to_string);
            let role = response
                .headers()
                .get("x-role")
                .and_then(|value| value.to_str().ok())
                .map(str::to_string);
            if response
                .content_length()
                .is_some_and(|length| length > MAX_ASR_JSON_BYTES as u64)
            {
                return Err(SpeechError::ResponseTooLarge("ASR JSON"));
            }
            let response_bytes =
                read_bounded(response, MAX_ASR_JSON_BYTES, ResponseService::AsrJson)?;
            let value: serde_json::Value =
                serde_json::from_slice(&response_bytes).map_err(|_| SpeechError::MalformedJson)?;
            let hypothesis = value
                .get("text")
                .and_then(serde_json::Value::as_str)
                .ok_or(SpeechError::MalformedJson)?
                .to_string();
            let latency = start.elapsed().as_millis() as u64;
            latencies.push(latency);
            let seconds = fixture.duration_seconds.max(0.000_001);
            let ratio = latency as f64 / (seconds * 1000.0);
            rtf += ratio;
            let reference = fixture.spoken_reference.clone().unwrap_or_default();
            scores.push(FixtureScore {
                fixture_id: fixture.id.clone(),
                edits: edit_counts(&reference, &hypothesis, normalize)?,
                correctness: correctness_checks(&reference, &hypothesis, &fixture.target_terms),
                reference,
                hypothesis,
                latency_ms: latency,
                real_time_factor: ratio,
                backend,
                role,
            });
        }
        let p50 = percentile(&mut latencies.clone(), 0.50);
        let p95 = percentile(&mut latencies, 0.95);
        let saved = SavedRun {
            schema_version: "speech-test.run.v1".into(),
            scores,
            latency_p50_ms: p50,
            latency_p95_ms: p95,
            real_time_factor: rtf / catalog.fixtures.len().max(1) as f64,
        };
        write_atomic(
            &output,
            &serde_json::to_vec_pretty(&saved)
                .map_err(|error| SpeechError::Other(error.to_string()))?,
        )?;
        Ok((saved, output))
    })();
    match result {
        Ok((data, path)) => emit(
            format,
            "replay",
            &serde_json::json!({
                "run": path,
                "scores": data.scores,
                "latency_p50_ms": data.latency_p50_ms,
                "latency_p95_ms": data.latency_p95_ms,
                "real_time_factor": data.real_time_factor
            }),
        ),
        Err(error) => emit_error(format, "replay", &error),
    }
}
fn score(path: &Path, format: OutputFormat) -> i32 {
    let result: Result<SavedRun, SpeechError> = match std::fs::read(path) {
        Ok(bytes) => {
            serde_json::from_slice::<SavedRun>(&bytes).map_err(|_| SpeechError::MalformedJson)
        }
        Err(error) => Err(SpeechError::RunRead(error.to_string())),
    };
    match result {
        Ok(run) if run.schema_version == "speech-test.run.v1" => emit(format, "score", &run),
        Ok(_) => emit_error(
            format,
            "score",
            &SpeechError::RunInvalid("unsupported schema_version".into()),
        ),
        Err(e) => emit_error(format, "score", &e),
    }
}

fn detect_format_from_argv() -> OutputFormat {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--format"
            && args
                .next()
                .is_some_and(|value| value.eq_ignore_ascii_case("json"))
        {
            return OutputFormat::Json;
        }
        if arg == "--format=json" {
            return OutputFormat::Json;
        }
    }
    OutputFormat::Text
}

fn main() {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(err) => {
            use clap::error::ErrorKind;
            if matches!(
                err.kind(),
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
            ) {
                err.exit();
            }
            let message = err
                .to_string()
                .lines()
                .next()
                .unwrap_or("invalid arguments")
                .replace("error:", "")
                .trim()
                .to_string();
            std::process::exit(emit_parse_error(
                BINARY,
                detect_format_from_argv(),
                "parse-error",
                &message,
            ));
        }
    };
    let format = cli.format.unwrap_or_default();
    let status = match cli.command {
        Command::Generate {
            catalog,
            plan,
            max_characters,
            cost_per_1000_characters,
            cache_dir,
            request_deadline_seconds,
        } => generate(
            &catalog,
            plan,
            max_characters,
            cost_per_1000_characters,
            cache_dir,
            request_deadline_seconds,
            format,
        ),
        Command::Review {
            catalog,
            id,
            reference,
            reviewer,
        } => review(&catalog, &id, &reference, &reviewer, format),
        Command::Replay {
            catalog,
            endpoint,
            deadline_seconds,
            auth_env,
            body,
            multipart_field,
            run_output,
            normalize,
        } => replay(
            &catalog,
            ReplayOptions {
                endpoint: &endpoint,
                deadline: deadline_seconds,
                auth_env: auth_env.as_deref(),
                body: &body,
                field: &multipart_field,
                run_output,
                normalize,
            },
            format,
        ),
        Command::Score { run } => score(&run, format),
        Command::Completion { shell } => {
            let mut command = Cli::command();
            let shell: Shell = shell.into();
            let mut generated = Vec::new();
            generate_completion(shell, &mut command, BINARY.to_string(), &mut generated);
            let generated = String::from_utf8_lossy(&generated);
            let normalized = if matches!(shell, Shell::Bash) {
                generated.replace("__subcmd__", "__")
            } else {
                generated.into_owned()
            };
            print!("{normalized}");
            exit::SUCCESS
        }
    };
    std::process::exit(status);
}
