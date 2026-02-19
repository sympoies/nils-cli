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
