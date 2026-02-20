# Gemini verification oracles

## Goal

Validate `agent-provider-gemini` using deterministic contract evidence first, with optional live
drift checks as a secondary safeguard.

## Oracle hierarchy

1. Primary oracle: `provider-adapter.v1` contract + crate-level contract tests.
2. Secondary oracle: deterministic mock/fixture data in `tests/fixtures/`.
3. Tertiary oracle: optional live Gemini runtime characterization.

If oracle outputs disagree, primary oracle wins and the mismatch must be documented before merge.

## Profiles

### Mock oracle profile (CI required)

- Source: contract tests + deterministic fixture inputs.
- Network: forbidden.
- Required for merge and release.

### Live oracle profile (optional, local)

- Source: configured Gemini runtime surface.
- Purpose: detect runtime drift vs fixture assumptions.
- Live-only differences are tracked but do not override contract oracle outcomes.

## Release blocker policy

- Contract oracle mismatch: release blocker.
- Mock/fixture oracle mismatch: release blocker.
- Dependency-boundary mismatch (`gemini_cli` import or missing `gemini-core` edge): release blocker.
- Live-only mismatch with contract-consistent behavior: non-blocking, document in change notes.

## Required evidence fields

Each verification artifact should include:

- `fixture_schema_version`
- `runtime_profile` (`mock` or `live`)
- `runtime_surface`
- `adapter_version`
- `oracle_result` (`pass` or `fail`)

## Verification workflow

1. Run adapter contract and dependency-boundary tests.
2. Confirm execute/auth-state failure paths map to stable category/code taxonomy.
3. Confirm unsupported/unavailable cases use explicit error envelopes (no silent fallback).
4. Optionally run live profile checks and compare against mock oracle expectations.
5. If behavior changes are intentional, update contract spec + fixture policy + release notes in one change.
