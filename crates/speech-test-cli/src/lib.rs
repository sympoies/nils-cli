use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::IpAddr;
use std::path::{Component, Path, PathBuf};

const MAX_FIXTURE_ID_BYTES: usize = 64;
pub const MAX_SCORE_CHARACTERS: usize = 4096;
const MAX_SCORE_CELL_UPDATES: usize = 4_000_000;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Catalog {
    pub schema_version: String,
    pub fixtures: Vec<Fixture>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Fixture {
    pub id: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub target_terms: Vec<String>,
    pub intended_text: String,
    #[serde(default)]
    pub spoken_reference: Option<String>,
    #[serde(default)]
    pub review: Option<Review>,
    pub license_source: String,
    pub generation: Generation,
    pub master_sha256: String,
    pub derived_sha256: String,
    pub file: String,
    pub codec: String,
    pub sample_rate_hz: u32,
    pub channels: u16,
    pub duration_seconds: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Review {
    pub reviewer: String,
    pub reviewed_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Generation {
    pub provider: String,
    pub model: String,
    pub voice: String,
    #[serde(default)]
    pub settings: serde_json::Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TtsRequest {
    pub provider: String,
    pub api_version: String,
    pub model: String,
    pub voice: String,
    pub text: String,
    pub voice_settings: serde_json::Value,
    pub speed: Option<f64>,
    pub seed: Option<u64>,
    pub language: Option<String>,
    pub normalization: Option<String>,
    pub output_format: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EditCounts {
    pub substitutions: usize,
    pub deletions: usize,
    pub insertions: usize,
    pub characters: usize,
    pub cer: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FixtureScore {
    pub fixture_id: String,
    pub reference: String,
    pub hypothesis: String,
    pub edits: EditCounts,
    pub correctness: CorrectnessChecks,
    pub latency_ms: u64,
    pub real_time_factor: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backend: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CheckCount {
    pub correct: usize,
    pub total: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CorrectnessChecks {
    pub number_tokens: CheckCount,
    pub date_tokens: CheckCount,
    pub negations: CheckCount,
    pub target_terms: CheckCount,
}

#[derive(Debug, thiserror::Error)]
pub enum SpeechError {
    #[error("cannot read catalog: {0}")]
    CatalogRead(String),
    #[error("cannot read saved run: {0}")]
    RunRead(String),
    #[error("invalid catalog: {0}")]
    CatalogInvalid(String),
    #[error("fixture {0} was not found")]
    FixtureMissing(String),
    #[error("fixture file is missing: {0}")]
    FileMissing(String),
    #[error("fixture hash does not match: {0}")]
    HashMismatch(String),
    #[error("fixture reference has not been reviewed: {0}")]
    Unreviewed(String),
    #[error("character budget exceeded: {actual} characters exceeds {maximum}")]
    Budget { actual: usize, maximum: usize },
    #[error("HTTP request failed: {0}")]
    Http(String),
    #[error("TTS request timed out; the provider may have charged for it. No retry was attempted")]
    TtsTimeout,
    #[error(
        "TTS audio response exceeded the 64 MiB supported limit; the provider may have charged for it. No retry was attempted"
    )]
    TtsResponseTooLarge,
    #[error("ASR request exceeded its configured deadline")]
    AsrTimeout,
    #[error("service returned HTTP {0}")]
    HttpStatus(u16),
    #[error("TTS provider returned HTTP {0}")]
    TtsStatus(u16),
    #[error("service returned malformed JSON")]
    MalformedJson,
    #[error("operation failed: {0}")]
    Other(String),
    #[error("invalid saved run: {0}")]
    RunInvalid(String),
    #[error("configuration error: {0}")]
    Configuration(String),
    #[error("credential-bearing requests require HTTPS outside loopback")]
    InsecureCredentialTransport,
    #[error("{0} response exceeded the configured size limit")]
    ResponseTooLarge(&'static str),
    #[error("cache entry changed or is invalid")]
    CacheInvalid,
    #[error("transcript is too large for bounded CER scoring")]
    ScoreInputTooLarge,
}

#[derive(Clone, Copy, Debug)]
pub enum ResponseService {
    TtsAudio,
    AsrJson,
}

impl ResponseService {
    fn label(self) -> &'static str {
        match self {
            Self::TtsAudio => "TTS audio",
            Self::AsrJson => "ASR JSON",
        }
    }

    fn timeout_error(self) -> SpeechError {
        match self {
            Self::TtsAudio => SpeechError::TtsTimeout,
            Self::AsrJson => SpeechError::AsrTimeout,
        }
    }
}

pub fn read_catalog(path: &Path) -> Result<Catalog, SpeechError> {
    let bytes = std::fs::read(path).map_err(|e| SpeechError::CatalogRead(e.to_string()))?;
    let text =
        std::str::from_utf8(&bytes).map_err(|e| SpeechError::CatalogInvalid(e.to_string()))?;
    let catalog: Catalog = if path.extension().is_some_and(|e| e == "toml") {
        toml::from_str(text).map_err(|e| SpeechError::CatalogInvalid(e.to_string()))?
    } else {
        serde_json::from_str(text).map_err(|e| SpeechError::CatalogInvalid(e.to_string()))?
    };
    if catalog.schema_version != "speech-test.catalog.v1" {
        return Err(SpeechError::CatalogInvalid(
            "unsupported schema_version".into(),
        ));
    }
    let mut ids = std::collections::HashSet::new();
    for fixture in &catalog.fixtures {
        if fixture.id.is_empty()
            || fixture.id.len() > MAX_FIXTURE_ID_BYTES
            || !fixture
                .id
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
            || !ids.insert(fixture.id.as_str())
        {
            return Err(SpeechError::CatalogInvalid(
                "fixture IDs must be unique, at most 64 characters, and use letters, digits, hyphens or underscores".into(),
            ));
        }
    }
    Ok(catalog)
}

pub fn write_catalog(path: &Path, catalog: &Catalog) -> Result<(), SpeechError> {
    validate_catalog_update_target(path)?;
    let encoded = if path.extension().is_some_and(|e| e == "toml") {
        toml::to_string_pretty(catalog).map_err(|e| SpeechError::Other(e.to_string()))?
    } else {
        serde_json::to_string_pretty(catalog).map_err(|e| SpeechError::Other(e.to_string()))?
    };
    write_atomic(path, encoded.as_bytes())
}

pub fn validate_catalog_update_target(path: &Path) -> Result<(), SpeechError> {
    if std::fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        return Err(SpeechError::CatalogInvalid(
            "catalog file must not be a symlink when updating it".into(),
        ));
    }
    Ok(())
}

pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), SpeechError> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let mut staged =
        tempfile::NamedTempFile::new_in(parent).map_err(|e| SpeechError::Other(e.to_string()))?;
    staged
        .write_all(bytes)
        .and_then(|_| staged.as_file().sync_all())
        .map_err(|e| SpeechError::Other(e.to_string()))?;
    staged
        .persist(path)
        .map_err(|e| SpeechError::Other(e.error.to_string()))?;
    Ok(())
}

pub fn check_writable_directory(path: &Path) -> Result<(), SpeechError> {
    validate_directory_path(path)?;
    std::fs::create_dir_all(path).map_err(|e| SpeechError::Other(e.to_string()))?;
    validate_directory_path(path)?;
    let probe =
        tempfile::NamedTempFile::new_in(path).map_err(|e| SpeechError::Other(e.to_string()))?;
    drop(probe);
    Ok(())
}

pub fn validate_directory_path(path: &Path) -> Result<(), SpeechError> {
    if std::fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        return Err(SpeechError::CatalogInvalid(
            "output directory must not contain symlinks".into(),
        ));
    }
    if path.exists() && !path.is_dir() {
        return Err(SpeechError::CatalogInvalid(
            "output directory is not a directory".into(),
        ));
    }
    Ok(())
}

pub fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

pub fn canonical_fingerprint(request: &TtsRequest) -> Result<String, SpeechError> {
    let value = serde_json::to_value(request).map_err(|e| SpeechError::Other(e.to_string()))?;
    let canonical = canonicalize_json(value);
    let bytes = serde_json::to_vec(&canonical).map_err(|e| SpeechError::Other(e.to_string()))?;
    Ok(sha256(&bytes))
}

fn canonicalize_json(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(object) => {
            let sorted: BTreeMap<_, _> = object.into_iter().collect();
            let mut canonical = serde_json::Map::new();
            for (key, value) in sorted {
                canonical.insert(key, canonicalize_json(value));
            }
            serde_json::Value::Object(canonical)
        }
        serde_json::Value::Array(values) => {
            serde_json::Value::Array(values.into_iter().map(canonicalize_json).collect())
        }
        other => other,
    }
}

pub fn resolve_catalog_file(catalog_path: &Path, file: &str) -> Result<PathBuf, SpeechError> {
    let relative = Path::new(file);
    if relative.is_absolute()
        || relative
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(SpeechError::CatalogInvalid(
            "fixture file must be a relative path inside the catalog directory".into(),
        ));
    }
    let base = catalog_path.parent().unwrap_or_else(|| Path::new("."));
    let mut path = base.to_path_buf();
    for component in relative.components() {
        path.push(component);
        if std::fs::symlink_metadata(&path).is_ok_and(|metadata| metadata.file_type().is_symlink())
        {
            return Err(SpeechError::CatalogInvalid(
                "fixture paths must not contain symlinks".into(),
            ));
        }
    }
    Ok(path)
}

pub fn verified_audio_path(catalog_path: &Path, fixture: &Fixture) -> Result<PathBuf, SpeechError> {
    if fixture.review.is_none()
        || fixture
            .spoken_reference
            .as_deref()
            .unwrap_or("")
            .trim()
            .is_empty()
    {
        return Err(SpeechError::Unreviewed(fixture.id.clone()));
    }
    let path = resolve_catalog_file(catalog_path, &fixture.file)?;
    if !std::fs::symlink_metadata(&path).is_ok_and(|metadata| metadata.file_type().is_file()) {
        return Err(SpeechError::FileMissing(path.display().to_string()));
    }
    let actual = sha256_file(&path)?;
    if actual != fixture.derived_sha256 {
        return Err(SpeechError::HashMismatch(path.display().to_string()));
    }
    Ok(path)
}

pub fn sha256_file(path: &Path) -> Result<String, SpeechError> {
    let mut file = std::fs::File::open(path)
        .map_err(|_| SpeechError::FileMissing(path.display().to_string()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|e| SpeechError::Other(e.to_string()))?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

pub fn read_bounded<R: Read>(
    reader: R,
    maximum: usize,
    service: ResponseService,
) -> Result<Vec<u8>, SpeechError> {
    let mut bytes = Vec::with_capacity(maximum.min(64 * 1024));
    reader
        .take(maximum as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| {
            let is_timeout = error.kind() == std::io::ErrorKind::TimedOut
                || error
                    .get_ref()
                    .and_then(|source| source.downcast_ref::<reqwest::Error>())
                    .is_some_and(reqwest::Error::is_timeout);
            if is_timeout {
                service.timeout_error()
            } else {
                SpeechError::Http(error.to_string())
            }
        })?;
    if bytes.len() > maximum {
        return Err(SpeechError::ResponseTooLarge(service.label()));
    }
    Ok(bytes)
}

pub fn validate_credential_url(raw: &str) -> Result<reqwest::Url, SpeechError> {
    let url = reqwest::Url::parse(raw)
        .map_err(|e| SpeechError::Configuration(format!("invalid endpoint URL: {e}")))?;
    if !url.username().is_empty() || url.password().is_some() {
        return Err(SpeechError::Configuration(
            "endpoint URLs must not contain credentials".into(),
        ));
    }
    if url.scheme() == "https" {
        return Ok(url);
    }
    let is_loopback = url.host_str().is_some_and(|host| {
        host.eq_ignore_ascii_case("localhost")
            || host
                .parse::<IpAddr>()
                .is_ok_and(|address| address.is_loopback())
    });
    if url.scheme() == "http" && is_loopback {
        return Ok(url);
    }
    Err(SpeechError::InsecureCredentialTransport)
}

pub fn edit_counts(
    reference: &str,
    hypothesis: &str,
    normalize: bool,
) -> Result<EditCounts, SpeechError> {
    if reference.chars().count() > MAX_SCORE_CHARACTERS
        || hypothesis.chars().count() > MAX_SCORE_CHARACTERS
    {
        return Err(SpeechError::ScoreInputTooLarge);
    }
    let prep = |s: &str| -> Vec<char> {
        let value = if normalize {
            s.to_lowercase()
        } else {
            s.to_string()
        };
        value
            .chars()
            .filter(|c| !normalize || !c.is_whitespace())
            .collect()
    };
    let a = prep(reference);
    let b = prep(hypothesis);
    if a.len() > MAX_SCORE_CHARACTERS
        || b.len() > MAX_SCORE_CHARACTERS
        || a.len().saturating_mul(b.len()) > MAX_SCORE_CELL_UPDATES
    {
        return Err(SpeechError::ScoreInputTooLarge);
    }
    let mut previous = (0..=b.len()).map(|j| (j, 0, 0, j)).collect::<Vec<_>>();
    for (i, char_a) in a.iter().enumerate() {
        let mut current = vec![(0usize, 0usize, 0usize, 0usize); b.len() + 1];
        current[0] = (i + 1, 0, i + 1, 0);
        for (offset, char_b) in b.iter().enumerate() {
            let j = offset + 1;
            let (cost, s, d, ins) = previous[j];
            let mut best = (cost + 1, s, d + 1, ins);
            let (cost, s, d, ins) = current[j - 1];
            let insertion = (cost + 1, s, d, ins + 1);
            if insertion.0 < best.0 {
                best = insertion;
            }
            let (cost, s, d, ins) = previous[j - 1];
            let substitution = if char_a == char_b {
                (cost, s, d, ins)
            } else {
                (cost + 1, s + 1, d, ins)
            };
            if substitution.0 < best.0 {
                best = substitution;
            }
            current[j] = best;
        }
        previous = current;
    }
    let (errors, substitutions, deletions, insertions) = previous[b.len()];
    Ok(EditCounts {
        substitutions,
        deletions,
        insertions,
        characters: a.len(),
        cer: if a.is_empty() {
            if b.is_empty() { 0.0 } else { 1.0 }
        } else {
            errors as f64 / a.len() as f64
        },
    })
}

fn words(text: &str) -> Vec<String> {
    text.split_whitespace()
        .map(|word| {
            word.trim_matches(|c: char| {
                matches!(
                    c,
                    ',' | '.' | '!' | '?' | ':' | ';' | '(' | ')' | '[' | ']' | '"' | '\''
                )
            })
            .to_lowercase()
        })
        .filter(|word| !word.is_empty())
        .collect()
}

fn count_matches(expected: &[String], actual: &[String]) -> CheckCount {
    let mut remaining = actual.to_vec();
    let mut correct = 0;
    for value in expected {
        if let Some(index) = remaining.iter().position(|candidate| candidate == value) {
            correct += 1;
            remaining.remove(index);
        }
    }
    CheckCount {
        correct,
        total: expected.len(),
    }
}

pub fn correctness_checks(
    reference: &str,
    hypothesis: &str,
    target_terms: &[String],
) -> CorrectnessChecks {
    let reference_words = words(reference);
    let hypothesis_words = words(hypothesis);
    let numbers = |tokens: &[String]| {
        tokens
            .iter()
            .filter(|token| token.chars().any(|c| c.is_ascii_digit()))
            .cloned()
            .collect::<Vec<_>>()
    };
    let dates = |tokens: &[String]| {
        tokens
            .iter()
            .filter(|token| {
                token.matches('-').count() + token.matches('/').count() >= 2
                    && token.chars().any(|c| c.is_ascii_digit())
            })
            .cloned()
            .collect::<Vec<_>>()
    };
    let negative = |tokens: &[String]| {
        tokens
            .iter()
            .filter(|token| {
                matches!(
                    token.as_str(),
                    "no" | "not" | "never" | "without" | "cannot"
                ) || token.ends_with("n't")
            })
            .cloned()
            .collect::<Vec<_>>()
    };
    let terms = target_terms
        .iter()
        .map(|term| term.to_lowercase())
        .collect::<Vec<_>>();
    let mut reference_terms = Vec::new();
    let mut hypothesis_terms = Vec::new();
    for term in &terms {
        let count = |text: &str| text.to_lowercase().match_indices(term).count();
        reference_terms.extend(vec![term.clone(); count(reference)]);
        hypothesis_terms.extend(vec![term.clone(); count(hypothesis)]);
    }
    CorrectnessChecks {
        number_tokens: count_matches(&numbers(&reference_words), &numbers(&hypothesis_words)),
        date_tokens: count_matches(&dates(&reference_words), &dates(&hypothesis_words)),
        negations: count_matches(&negative(&reference_words), &negative(&hypothesis_words)),
        target_terms: count_matches(&reference_terms, &hypothesis_terms),
    }
}

pub fn percentile(values: &mut [u64], percentile: f64) -> u64 {
    if values.is_empty() {
        return 0;
    }
    values.sort_unstable();
    let index = ((values.len() - 1) as f64 * percentile).ceil() as usize;
    values[index.min(values.len() - 1)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn computes_known_character_error_breakdown() {
        let score = edit_counts("cat", "cut!", false).unwrap();
        assert_eq!(score.substitutions, 1);
        assert_eq!(score.deletions, 0);
        assert_eq!(score.insertions, 1);
        assert_eq!(score.characters, 3);
        assert_eq!(score.cer, 2.0 / 3.0);
    }

    #[test]
    fn edit_distance_scoring_rejects_inputs_over_its_cell_budget() {
        let text = "x".repeat(2_001);
        assert!(matches!(
            edit_counts(&text, &text, false),
            Err(SpeechError::ScoreInputTooLarge)
        ));
    }

    #[test]
    fn fingerprint_changes_with_any_request_field_but_not_credentials() {
        let request = TtsRequest {
            provider: "elevenlabs".into(),
            api_version: "v1".into(),
            model: "m".into(),
            voice: "v".into(),
            text: "hello".into(),
            voice_settings: serde_json::json!({"stability":0.5}),
            speed: Some(1.0),
            seed: Some(1),
            language: Some("en".into()),
            normalization: Some("auto".into()),
            output_format: "mp3_44100_128".into(),
        };
        let baseline = canonical_fingerprint(&request).unwrap();
        let mut changed = Vec::new();
        let mut next = request.clone();
        next.provider = "another-provider".into();
        changed.push(next);
        let mut next = request.clone();
        next.api_version = "v2".into();
        changed.push(next);
        let mut next = request.clone();
        next.model = "other-model".into();
        changed.push(next);
        let mut next = request.clone();
        next.voice = "other-voice".into();
        changed.push(next);
        let mut next = request.clone();
        next.text = "other text".into();
        changed.push(next);
        let mut next = request.clone();
        next.voice_settings["stability"] = serde_json::json!(0.7);
        changed.push(next);
        let mut next = request.clone();
        next.speed = Some(1.1);
        changed.push(next);
        let mut next = request.clone();
        next.seed = Some(2);
        changed.push(next);
        let mut next = request.clone();
        next.language = Some("fr".into());
        changed.push(next);
        let mut next = request.clone();
        next.normalization = Some("off".into());
        changed.push(next);
        let mut next = request.clone();
        next.output_format = "pcm_16000".into();
        changed.push(next);
        for mutation in changed {
            assert_ne!(baseline, canonical_fingerprint(&mutation).unwrap());
        }
    }

    #[test]
    fn fingerprint_sorts_nested_object_keys() {
        let mut left = serde_json::Map::new();
        left.insert("stability".into(), serde_json::json!(0.5));
        left.insert("similarity_boost".into(), serde_json::json!(0.75));
        let mut right = serde_json::Map::new();
        right.insert("similarity_boost".into(), serde_json::json!(0.75));
        right.insert("stability".into(), serde_json::json!(0.5));
        let make = |voice_settings| TtsRequest {
            provider: "elevenlabs".into(),
            api_version: "v1".into(),
            model: "m".into(),
            voice: "v".into(),
            text: "hello".into(),
            voice_settings: serde_json::Value::Object(voice_settings),
            speed: None,
            seed: None,
            language: None,
            normalization: None,
            output_format: "mp3_44100_128".into(),
        };
        assert_eq!(
            canonical_fingerprint(&make(left)).unwrap(),
            canonical_fingerprint(&make(right)).unwrap()
        );
    }

    #[test]
    fn scores_numbers_dates_negations_and_configured_terms() {
        let checks = correctness_checks(
            "Call 911 on 2026-10-27, never omit Alpha.",
            "Call 911 on 2026-10-27, omit alpha.",
            &["Alpha".into()],
        );
        assert_eq!(checks.number_tokens.correct, 2);
        assert_eq!(checks.date_tokens.correct, 1);
        assert_eq!(checks.negations.correct, 0);
        assert_eq!(checks.negations.total, 1);
        assert_eq!(checks.target_terms.correct, 1);
    }

    #[test]
    fn recognizes_negation_contractions() {
        let checks = correctness_checks(
            "It didn't fail and shouldn't continue.",
            "It failed and continued.",
            &[],
        );
        assert_eq!(checks.negations.total, 2);
        assert_eq!(checks.negations.correct, 0);
    }

    #[test]
    fn bounds_http_response_reads_and_rejects_plaintext_credentials() {
        assert!(matches!(
            read_bounded(std::io::Cursor::new(b"1234"), 3, ResponseService::AsrJson,),
            Err(SpeechError::ResponseTooLarge("ASR JSON"))
        ));
        assert!(validate_credential_url("http://example.invalid").is_err());
        assert!(validate_credential_url("http://127.0.0.1:8080").is_ok());
        assert!(validate_credential_url("http://localhost:8080").is_ok());
        assert!(validate_credential_url("https://example.invalid").is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn atomic_file_writes_replace_symlinks_without_following_targets() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let outside = temp.path().join("outside.txt");
        let link = temp.path().join("output.bin");
        std::fs::write(&outside, b"preserve this file").unwrap();
        symlink(&outside, &link).unwrap();
        write_atomic(&link, b"generated fixture bytes").unwrap();
        assert_eq!(std::fs::read(&outside).unwrap(), b"preserve this file");
        assert_eq!(std::fs::read(&link).unwrap(), b"generated fixture bytes");
        assert!(
            !std::fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
    }

    #[test]
    fn toml_catalog_round_trips_and_fixture_paths_stay_relative() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("catalog.toml");
        let catalog = Catalog {
            schema_version: "speech-test.catalog.v1".into(),
            fixtures: vec![Fixture {
                id: "tone-a".into(),
                tags: vec!["synthetic".into()],
                target_terms: vec![],
                intended_text: "Fictional phrase.".into(),
                spoken_reference: None,
                review: None,
                license_source: "self-made".into(),
                generation: Generation {
                    provider: "elevenlabs".into(),
                    model: "configured-model".into(),
                    voice: "configured-voice".into(),
                    settings: serde_json::json!({"speed": 1.0}),
                },
                master_sha256: "00".into(),
                derived_sha256: "00".into(),
                file: "tone-a.mp3".into(),
                codec: "mp3".into(),
                sample_rate_hz: 44100,
                channels: 1,
                duration_seconds: 0.1,
            }],
        };
        write_catalog(&path, &catalog).unwrap();
        let decoded = read_catalog(&path).unwrap();
        assert_eq!(decoded.fixtures[0].generation.settings["speed"], 1.0);
        assert!(resolve_catalog_file(&path, "../outside.wav").is_err());
    }
}
