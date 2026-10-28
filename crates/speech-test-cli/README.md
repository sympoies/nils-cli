# speech-test-cli

`speech-test-cli` creates paid TTS fixtures only when `generate` is explicitly
run, records a human review, and replays hash-verified fixtures against a
configured ASR HTTP endpoint. Replay never generates audio. Provider credentials
are read from environment variables and are excluded from request fingerprints,
catalogs, run files, and output.

## Commands

```text
speech-test-cli generate --catalog <path> --plan
                        --cost-per-1000-characters <rate>
                        [--max-characters <count>] [--cache-dir <path>]
speech-test-cli generate --catalog <path> [--max-characters <count>]
                        [--cache-dir <path>] [--request-deadline-seconds <seconds>]
speech-test-cli review --catalog <path> --id <fixture-id>
                       --reference <spoken-text> --reviewer <role-or-name>
speech-test-cli replay --catalog <path> --endpoint <url>
                       [--auth-env <variable>] [--deadline-seconds <seconds>]
                       [--body raw|multipart] [--multipart-field <name>]
                       [--run-output <path>] [--normalize]
speech-test-cli score --run <saved-run.json>
speech-test-cli completion <bash|zsh>
```

Use `--format json` on `generate`, `review`, `replay`, and `score` for the
versioned workspace envelope. `completion` always prints raw shell source. Text
mode prints readable key/value summaries. Usage errors exit `64`, invalid data
or fixtures exit `65`, unavailable services exit `69`, and internal failures
exit `70`. JSON failures contain a stable `error.code` and one-line message.

## Catalog v1

Catalog files may be JSON or TOML; the extension `.toml` selects TOML, and
other extensions select JSON. The root has `schema_version =
"speech-test.catalog.v1"` and a `fixtures` array. Each fixture uses these
fields:

| Field | Meaning |
| --- | --- |
| `id`, `tags`, `target_terms` | Stable identity (unique; IDs are 1–64 ASCII letters, digits, hyphens, or underscores) and optional scoring labels |
| `intended_text` | Text submitted to TTS |
| `spoken_reference` | Human-reviewed transcript used for ASR scoring |
| `review` | `reviewer` and RFC 3339 `reviewed_at`; absent until reviewed |
| `license_source` | Source or license note for the audio |
| `generation` | `provider`, `model`, `voice`, and complete non-secret `settings` |
| `master_sha256`, `derived_sha256` | SHA-256 hashes of source and replay audio |
| `file` | Audio path relative to the catalog directory |
| `codec`, `sample_rate_hz`, `channels`, `duration_seconds` | Audio metadata |

The `generation.settings` object may contain `api_version`, `voice_settings`,
`speed`, `seed`, `language`, `normalization`, and `output_format`. Generation
currently supports `mp3_<sample_rate>_<bitrate>` and 16-bit mono
`pcm_<sample_rate>` output formats; other codecs are rejected before a provider
request so catalog metadata stays accurate. The full
request is fingerprinted as canonical JSON. Changing any request field creates
a cache miss. The API key is never part of the fingerprint. A valid cached
audio file and matching hash are reused; cache entries are written under
`.speech-test-cache` beside the catalog by default.

Generated audio uses content-addressed paths under `.speech-test-generated`.
Older generated files are retained when a catalog is updated so prior audio
remains available for restoring earlier fixture references.

## Generate, review, replay

1. Add catalog entries with intended text, provider/model/voice configuration,
   audio path, and source note. `generate --plan` displays request details,
   character counts, cache state, and estimated cost using the supplied
   `--cost-per-1000-characters` rate. The rate is required for a plan so the
   estimate does not assume a pricing tier. Planning makes no network calls.
2. Set `ELEVENLABS_API_KEY` in the process environment and run `generate` to
   create audio. `--max-characters` prevents a request set from exceeding the
   supplied budget. TTS requests are sent sequentially and never retried.
   `--request-deadline-seconds` controls each request timeout (default: 60).
   A timeout is reported as possibly charged. New audio clears any prior review.
3. Listen to the fixture and run `review` with the reviewed spoken reference.
   Review records the reviewer and current UTC timestamp.
4. Run `replay` with the configured ASR endpoint. Optionally name an
   environment variable with `--auth-env`; its value is sent as a bearer token.
   Requests are sent one at a time. Audio file presence, hash, and review are
   checked for the complete catalog before the first ASR request.
5. Replay saves a `speech-test.run.v1` file. `score --run` reads it offline and
   emits the same saved metrics without making a request.

Replay records per-fixture reference/hypothesis, CER substitution/deletion/
insertion counts, optional number/date/negation/target-term correctness counts,
latency, and real-time factor. It also reports aggregate latency p50/p95 and
mean RTF. CER scoring accepts at most 4,096 Unicode characters per transcript
and 4,000,000 edit-distance cell updates; larger inputs return a typed error
before dynamic-programming work starts. `--normalize` lowercases text and ignores
whitespace for CER.
Successful ASR response JSON must contain a string `text`; optional `x-backend`
and `x-role` response headers are preserved in the run file.

TTS audio responses are limited to 64 MiB. Larger responses are rejected; the
provider may have charged for a request that returned an oversized response.

## Dependencies and tests

The CLI uses `reqwest` with rustls for synchronous HTTP, `serde` for catalog
and run files, `sha2` for fixture and request fingerprints, and `jiff` for
review timestamps. Automated tests use generated tiny WAV data and loopback
HTTP stubs only; they never contact a TTS or ASR provider.
