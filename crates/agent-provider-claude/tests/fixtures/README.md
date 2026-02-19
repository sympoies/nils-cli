# agent-provider-claude fixtures

This folder contains deterministic fixtures for `agent-provider-claude` contract tests.

## Layout

- `api/`: raw API payload fixtures used by mock contract tests.
- `characterization/`: black-box characterization inputs and expected metadata.

## Fixture policy

- Fixture IDs are immutable once published.
- Additive fixture changes are allowed within the same schema version.
- Breaking fixture shape changes require a new `fixture_schema_version`.

## Required scenario IDs

`characterization/manifest.json` must include:

- `success`
- `auth_failure`
- `rate_limit`
- `timeout`
- `malformed_response`

## Secret hygiene

- Do not store real API keys, bearer tokens, or cookies.
- Use placeholders such as `test-key` and synthetic request IDs.

## Fixture update checklist

- Treat every fixture update as deterministic-contract maintenance; do not mutate existing IDs.
- Follow redaction guidance from
  `crates/agent-provider-claude/docs/runbooks/verification-oracles.md` before staging fixture
  payloads.
- Run a secret scan before commit:
  - `rg -n "(?i)(api[_-]?key|authorization:|bearer\\s+[a-z0-9._-]+|cookie:|sk-ant-)" crates/agent-provider-claude/tests/fixtures`
- If any secret leakage is found, stop and replace values with deterministic placeholders.
