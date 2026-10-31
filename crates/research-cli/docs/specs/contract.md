# Research contract

Canonical crate-local specification. Refs #3161.

## Observable acceptance

- CLI search/read-url/brief and HTTP POST /v1/search, /v1/read-url, /v1/brief call the same engine and produce the same operation envelope
  and errors.
- A request uses `schema_version: research.request.v1`, `operation: search|read-url|brief`, `query`, `provider: openai|anthropic`,
  optional `language`, `domains`, `recency_days`, `limit`, `urls`, `fields`, and `limits`. Providers never change caller fields. The CLI
  maps flags directly to this request.
- Search and brief require a query. Read-url requires exactly one URL and performs no model request. Brief first reads supplied public
  URLs, then performs at most one provider API request. Keys are environment-only.
- Limits bound the whole deadline, bytes per response, aggregate prose output characters, redirects, model output tokens, and server tool
  uses. URL destinations must be public at every redirect; DNS answers are checked and pinned to the connection, and environment proxies
  are disabled.
- Success envelopes use `cli.research-cli.<operation>.v1`, `ok`, `data`, and optional envelope `warnings`. Failures retain available
  partial `data` with the same schema plus `error: {code,message,details?}`. Parse errors use `cli.research-cli.error.v1`. No automatic
  retries or model/provider substitution.
- Data always contains query, provider/model (null for plain read), retrieved_at, sources, facts, summary, assessment, unknowns,
  conflicts, usage, warnings, truncated, and optional content. Sources have id/url/final_url/title/published_at; unknown metadata is null,
  excerpts only exist when returned. Search URLs are not claimed as final redirect destinations. Page age is update metadata, not a
  publication date.
- Facts contain claim and nonempty source_ids referencing returned sources. Unsupported facts are discarded with a warning. Assessment is
  always separate and labelled `model_inference`, with generated, key_points, suggested_directions, preliminary_judgment and confidence
  low/medium/high. Without model assessment, generated is false and judgment is null. Text prints Facts and Assessment (model inference)
  separately.
- Usage retains the provider usage object plus nullable search_calls/input_tokens/output_tokens/cost_usd_estimate and price_source.
  Unknown is never a guessed zero. Estimates are informational, never a billing or credit guarantee. Haiku 5.5 Standard/global estimates
  use the 2026-11-01 public rate snapshot only when token/search/cache components are known; nonzero cache writes remain unknown because
  the TTL-specific rate is unavailable. Aggregated input over 100,000 tokens also leaves cost unknown because each server tool iteration
  can have its own prompt-length tier. Responses has no guaranteed raw search-call count, so its search_calls and total cost stay null.
- Empty search is an honest empty source/fact list. Provider failure, timeout, incomplete output, interrupted server turns and limits
  remain failures or explicitly partial results, never invented content. Tools are configured from validated caller input only; fetched
  content cannot change configuration or execute local tools.

## CLI and HTTP

`research-cli --format json search QUERY --provider openai` selects OpenAI gpt-6-luna Responses web_search. Anthropic selects
claude-haiku-5-5 Messages with the basic web_search_20250305 and web_fetch_20250910 contracts, without code execution. Both are current
supported external tool versions selected for basic search/fetch.

Common flags: --language, --domain (repeat), --recency-days, --limit, --timeout-seconds, --max-bytes, --max-chars, --max-redirects,
--max-output-tokens, --max-tool-uses. Brief also accepts --url and --field (repeat). Fields are requested synthesis topics, never
arbitrary JSON schema or tools. Language and recency are model instructions; adapters warn they are best-effort because neither selected
tool exposes a strict recency/language filter. Domains and returned source counts are enforced locally.

`research-cli serve --bind 127.0.0.1:8787` binds locally by default. The HTTP body is the request above; its operation must match the
route. Malformed/oversized JSON uses a structured error; the HTTP body limit is 64 KiB. HTTP 400 = invalid request, 429 = provider rate
limit, 504 = deadline, 502 = upstream failure, 200 = successful or warned partial result. CLI exits: 0 success, 64 syntax, 65 invalid
request, 69 missing key/unsupported provider capability, 1 runtime/upstream failure. Completion exports bash/zsh from clap; no aliases or
runtime completion modes.

## Provider synthesis

The system instruction requests application JSON with summary, facts containing claim/source_urls, assessment, unknowns and conflicts.
This is an application output format, not an invented provider field. Source metadata is parsed only from documented native
source/citation/tool blocks. Invalid or cut-off synthesis retains source metadata/raw usage and warns that synthesis/assessment is
unavailable. No second paid repair request is made.

## Optional live smoke

With an approved key already injected, run once per provider:

```sh
research-cli --format json search 'Find the IANA example domains page' --provider openai --domain iana.org --limit 1 --max-tool-uses 1 --max-output-tokens 1024 --timeout-seconds 30
research-cli --format json search 'Find the IANA example domains page' --provider anthropic --domain iana.org --limit 1 --max-tool-uses 1 --max-output-tokens 1024 --timeout-seconds 30
```

Missing OPENAI_API_KEY or ANTHROPIC_API_KEY means SKIP that provider. A timeout or uncertain billing ends that live path. No retry,
fallback or account mutation. Offline fixtures are synthetic documented-wire examples, not claims of live access.

Official contracts checked during implementation: [OpenAI model](https://developers.openai.com/api/docs/models/gpt-6-luna), [Responses web
search](https://developers.openai.com/api/docs/guides/tools-web-search), [OpenAI pricing](https://developers.openai.com/api/docs/pricing),
[Haiku](https://platform.claude.com/docs/en/models/haiku-5-5/overview), [Messages
search](https://platform.claude.com/docs/en/agents-and-tools/tool-use/web-search-tool), [Messages
fetch](https://platform.claude.com/docs/en/agents-and-tools/tool-use/web-fetch-tool). Account access is unverified without a live key.
