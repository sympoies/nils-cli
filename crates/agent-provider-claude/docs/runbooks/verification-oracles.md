# Claude verification oracles

## Goal

Validate Claude adapter correctness without requiring proprietary source code access.

## Oracle priority

1. **Primary oracle**: official API contracts and SDK behavior from Anthropic official sources.
2. **Secondary oracle**: black-box characterization from local Claude CLI behavior.
3. **Tertiary oracle**: deterministic mock fixtures used by CI.

If oracles disagree, primary oracle wins; differences must be documented.

## Mismatch severity and release policy

- API contract mismatch: **release blocker**
- fixture-vs-adapter mismatch: **release blocker**
- CLI-only mismatch with API-consistent adapter: **non-blocking**, but must be documented in
  parity docs and characterization report.

## Required evidence fields

Characterization artifacts must include:

- `api_doc_date`
- `model_id`
- `claude_cli_version`
- `fixture_schema_version`

## Profiles

### Mock profile (required in CI)

- deterministic
- no network credentials required
- runs fixture-backed tests:
  - `cargo test -p nils-agent-provider-claude --test mock_contract`

### Live profile (optional, non-blocking for default CI)

- requires explicit opt-in:
  - `CLAUDE_LIVE_TEST=1 cargo test -p nils-agent-provider-claude --test live_smoke -- --ignored`
- intended for periodic drift detection

## Characterization workflow

1. Run mock characterization:
   - `bash scripts/ci/claude-characterization.sh --mode mock`
2. Run local CLI characterization (optional):
   - `bash scripts/ci/claude-characterization.sh --mode local-cli --allow-skip`
3. Review generated report:
   - `target/claude-characterization/local-cli-report.json`

## Escalation

When a blocker mismatch is detected:

1. stop release promotion for Claude adapter changes
2. capture failing payload/fixture IDs
3. update contract docs or adapter implementation
4. re-run mock profile before re-opening release gate
