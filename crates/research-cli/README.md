# research-cli

One research engine for scripts, agents and a local HTTP service. Select OpenAI
`gpt-6-luna` or Anthropic `claude-haiku-5-5` without changing the caller schema.
Plain public URL reading uses HTTP directly and requires no provider key.

```sh
research-cli --format json search 'example domain specification' --provider openai --domain iana.org --limit 3
research-cli read-url https://example.com --max-bytes 65536 --max-chars 4096
research-cli --format json brief 'What is known about example domains?' --provider anthropic --url https://example.com --field unknowns
research-cli serve --bind 127.0.0.1:8787
research-cli completion zsh
```

Commands: `search`, `read-url`, `brief`, `serve`, `completion`. Root `-V/--version`
is available. Text is the default; `--format json` emits one versioned envelope.
Facts cite sources; the assessment is separately labelled model inference.
A failed request retains available partial data. Limits are explicit flags;
search and brief default to one provider request, one use per configured tool,
1024 model output tokens, 30 seconds, 256 KiB per response and 16 Ki characters
of returned prose. Brief with URLs allows Anthropic search and fetch separately;
OpenAI uses its aggregate `max_tool_calls` limit. There are no automatic retries.
The requested token limit is a provider generation parameter, not a hard
aggregate billing cap across hosted tool iterations. An Anthropic-reported
aggregate output overrun returns an explicit partial failure with raw usage
preserved; already billed tokens cannot be retroactively capped.

Provider keys come only from `OPENAI_API_KEY` / `ANTHROPIC_API_KEY`. No external
binary is needed. Rust dependencies include reqwest with rustls, Tokio, axum,
clap and jiff. HTTPS trusts the platform certificate store. Environment proxies
are disabled so public-destination checks cannot be bypassed by proxy routing.
Read-url returns bounded UTF-8 text (including raw HTML), not browser rendering;
PDFs and binary media are unsupported. URL credentials and non-public destinations
are rejected, including after redirects. Content is evidence, never instructions.

The local HTTP service exposes POST `/v1/search`, `/v1/read-url`, `/v1/brief`.
It uses the exact same engine and result/error envelopes as the CLI. Bind beyond
loopback only within an operator-managed service boundary; authentication and
private service provisioning are outside this crate.

See the [documentation index](docs/README.md) for the full request/output
contract, error/exit codes, offline tests and bounded optional live smoke.
