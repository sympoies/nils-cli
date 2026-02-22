# codex/gemini CLI parity contract v1

## Purpose
This document is the canonical parity contract for `nils-codex-cli` and `nils-gemini-cli` after core-crate consolidation into CLI adapters plus `nils-common::provider_runtime`.

## Topology parity

Both binaries must expose the same top-level command topology:

- `agent`
- `auth`
- `diag`
- `config`
- `starship`
- `completion`

Shared help behavior:

- `--help`/`help` exit code remains `0`.
- Unknown groups/subcommands return deterministic usage errors (`64`) with lane-specific binary prefixes.

## Invalid-command parity

Non-canonical command invocations must preserve equal exit semantics between lanes:

- Unknown top-level groups
- Unknown subcommands under canonical groups

Parity requirement: for equivalent invocation shapes above, `codex-cli` and `gemini-cli` must return identical exit code classes.

## JSON contract parity

Structure parity is required while schema ids remain provider-specific.

- Auth command family:
  - Codex schema id prefix: `codex-cli.auth.v1`
  - Gemini schema id prefix: `gemini-cli.auth.v1`
- Diag command family:
  - Codex schema id prefix: `codex-cli.diag.rate-limits.v1`
  - Gemini schema id prefix: `gemini-cli.diag.rate-limits.v1`

Compatibility rules:

1. Required fields and envelope shape remain aligned across both lanes.
2. Provider labels and schema ids remain lane-specific.
3. No secret-bearing fields are emitted in error envelopes.

## Runtime adapter invariants

1. Provider-specific env key names, default model values, path precedence, and dangerous-exec command shape are configured via provider profiles.
2. Shared runtime primitives (`auth/json/jwt/error/path/config/exec` logic) stay in `nils-common::provider_runtime`.
3. Human output text and exit semantics stay stable for existing commands.

## Validation anchors

- `cargo test -p nils-codex-cli --test parity_oracle`
- `cargo test -p nils-gemini-cli --test parity_oracle`
- `cargo test -p nils-codex-cli --test runtime_auth_contract --test runtime_error_contract --test runtime_exec_contract --test runtime_paths_config_contract`
- `cargo test -p nils-gemini-cli --test runtime_auth_contract --test runtime_error_contract --test runtime_exec_contract --test runtime_paths_config_contract`
