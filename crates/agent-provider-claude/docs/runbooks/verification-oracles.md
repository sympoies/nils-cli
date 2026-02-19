# Claude verification oracles

## Goal

Validate Claude adapter correctness without requiring proprietary source code access.

## Oracle priority

1. **Primary oracle**: official API contracts and SDK behavior from Anthropic official sources.
2. **Secondary oracle**: black-box characterization from local Claude CLI behavior.
3. **Tertiary oracle**: deterministic mock fixtures used by CI.

If oracles disagree, primary oracle wins; differences must be documented.

## Mismatch severity and release gate rules

Release gate category map:

| Category | Trigger | Release gate |
| --- | --- | --- |
| `api-contract-mismatch` | Adapter behavior conflicts with official Anthropic API docs/SDK behavior. | **Closed (blocker)** |
| `fixture-adapter-mismatch` | Deterministic fixture-backed mock test expectation and adapter behavior diverge. | **Closed (blocker)** |
| `cli-characterization-drift` | Local Claude CLI differs from adapter while adapter still matches primary API contract. | **Open (non-blocking)** |

`cli-characterization-drift` must still be documented in parity docs and characterization reports.

Release gate decision order:

1. Evaluate **Primary oracle** evidence first.
2. Evaluate **Tertiary oracle** fixture-backed CI results second.
3. Use **Secondary oracle** characterization deltas for drift reporting when 1 and 2 are green.

## Required evidence fields

Characterization artifacts must include:

- `fixture_id`
- `report_schema_version`
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
3. Review generated report and diff outputs:
   - `target/claude-characterization/mock-report.json`
   - `target/claude-characterization/mock-diff.json`
   - `target/claude-characterization/local-cli-report.json`
   - `target/claude-characterization/local-cli-diff.json`

Local CLI mode is skip-safe when `claude` is unavailable and `--allow-skip` is set; in this
case both local-cli artifacts are still generated with `status=skipped` and a non-empty `reason`.

## Escalation

When a blocker mismatch is detected:

1. stop release promotion for Claude adapter changes
2. capture failing payload/fixture IDs
3. update contract docs or adapter implementation
4. re-run mock profile before re-opening release gate

## Fixture update redaction guidance

When a fixture update is proposed:

1. Keep fixture IDs unchanged; only additive payload changes are allowed within the same schema.
2. Apply redaction guidance before commit: replace credentials, tokens, cookies, org IDs, and
   request IDs with deterministic placeholders.
3. Run a secret scan over fixture paths and staged docs:
   - `rg -n "(?i)(api[_-]?key|authorization:|bearer\\s+[a-z0-9._-]+|cookie:|sk-ant-)" crates/agent-provider-claude/tests/fixtures crates/agent-provider-claude/docs/runbooks`
4. If secret leakage is detected, block the fixture update, scrub history as needed, and rotate
   any potentially exposed credential material.
