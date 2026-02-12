# memo-cli Rollout and Rollback Guide

## Scope
This runbook covers first-time rollout of `memo-cli` for capture-first workflows and agent-assisted
enrichment.

## Rollout checklist
1. Install and verify binary:
   - `cargo run -p nils-memo-cli -- --help`
2. Initialize a local database and smoke capture:
   - `memo-cli add "rollout smoke capture"`
3. Validate list/search/report basics:
   - `memo-cli list --limit 5`
   - `memo-cli search "rollout"`
   - `memo-cli report week`
4. Validate machine flow with JSON:
   - `memo-cli fetch --json --limit 10`
   - `memo-cli apply --json --dry-run --stdin < enrichment-batch.json`

## Smoke test expectations
- `add` returns a new `item_id` without mutating prior entries.
- `fetch --json` returns deterministic ordering and valid envelope keys.
- `apply --dry-run` validates payloads without writing derivation rows.
- `report` shows non-negative totals and stable period/range fields.

## Monitoring checkpoints
- Command failure rate:
  - track non-zero exits for `add`, `fetch`, and `apply`.
- JSON contract health:
  - ensure `schema_version`, `command`, `ok`, and `result|results|error` are always present.
- Apply quality:
  - monitor `accepted/skipped/failed` counts for anomalies in `apply` responses.
- Search/report consistency:
  - spot-check that accepted derivations appear in `search` and `report` outputs.

## Rollback triggers
- Trigger A: repeated `invalid-apply-payload` or `apply-item-conflict` bursts after rollout.
- Trigger B: contract-breaking JSON output observed by automation consumers.
- Trigger C: unexpected drop in `fetch`/`report` correctness (missing recent captures).
- Trigger D: release-gate regressions (`nils-cli-checks.sh` or coverage gate repeatedly fail).

## Rollback actions
1. Pause write-back automation immediately:
   - stop `memo-cli apply` jobs and keep capture-only mode (`add`, `list`, `search`, `report`).
2. Continue capture durability:
   - keep `memo-cli add` enabled; do not rewrite existing raw rows.
3. Revert to last known-good implementation commit for `nils-memo-cli`.
4. Re-run validation gates before re-enabling automation:
   - `cargo test -p nils-memo-cli memo_flow`
   - `cargo test -p nils-memo-cli agent_roundtrip`
   - `./.codex/skills/nils-cli-checks/scripts/nils-cli-checks.sh`

## Roll-forward criteria
- `memo_flow` and `agent_roundtrip` tests green.
- JSON contract and no-secret-leak tests green.
- Required repo checks and coverage gate green.
- Agent workflow fallback path verified in `memo-cli-agent-workflow.md`.
