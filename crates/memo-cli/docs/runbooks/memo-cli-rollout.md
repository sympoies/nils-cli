# memo-cli Rollout and Rollback Guide

## Scope
This runbook covers first-time rollout of `memo-cli` for capture-first workflows and agent-assisted
enrichment.
Storage initialization now uses one consolidated schema snapshot (`schema_v1.sql`). Older DB files
from pre-consolidation builds are not guaranteed to auto-upgrade.

## Rollout checklist
1. Install and verify binary:
   - `cargo run -p nils-memo-cli -- --help`
2. For environments with old `memo-cli` DB files, recreate DB before rollout:
   - remove or move the old DB file, then re-run `memo-cli add ...` to initialize a new schema.
3. Initialize a local database and smoke capture:
   - `memo-cli add "rollout smoke capture"`
   - `memo-cli add --at 2026-02-12T10:00:00Z "rollout explicit timestamp capture"`
4. Validate list/search/report basics:
   - `memo-cli list --limit 5`
   - `memo-cli search "rollout"`
   - `memo-cli search "rollout" --field raw,tags`
   - `memo-cli update itm_00000001 "rollout updated capture"`
   - `memo-cli delete itm_00000002 --hard`
   - `memo-cli report week`
   - `memo-cli report week --tz Asia/Taipei`
   - `memo-cli report month --from 2026-02-01T00:00:00Z --to 2026-02-29T23:59:59Z --json`
5. Validate machine flow with JSON:
   - `memo-cli fetch --json --limit 10`
   - `memo-cli apply --json --dry-run --stdin < enrichment-batch.json`

## Smoke test expectations
- `add` returns a new `item_id`.
- hard delete + re-add yields a strictly newer `item_id` (no id reuse).
- explicit sequence check: add `itm_00000001` -> `delete --hard itm_00000001` -> next add is `itm_00000002`+.
- `update` clears downstream derivations/workflow rows and returns item to pending.
- `delete --hard` removes the item from list/search/fetch/report surfaces.
- `fetch --json` returns deterministic ordering and valid envelope keys.
- `apply --dry-run` validates payloads without writing derivation rows.
- `report` shows non-negative totals and stable period/range fields.
- `list`/`search`/`fetch` JSON may include additive metadata fields:
  `content_type` and `validation_status`.
- `report --json` may include additive aggregates:
  `top_content_types` and `validation_status_totals`.

## Monitoring checkpoints
- Command failure rate:
  - track non-zero exits for `add`, `fetch`, and `apply`.
- JSON contract health:
  - ensure `schema_version`, `command`, `ok`, and `result|results|error` are always present.
- Apply quality:
  - monitor `accepted/skipped/failed` counts for anomalies in `apply` responses.
  - monitor metadata distribution (`content_type`, `validation_status`) for
    unexpected spikes.
- Search/report consistency:
  - spot-check that accepted derivations appear in `search` and `report` outputs.

## Rollback triggers
- Trigger A: repeated `invalid-apply-payload` or `apply-item-conflict` bursts after rollout.
- Trigger B: contract-breaking JSON output observed by automation consumers.
- Trigger C: unexpected drop in `fetch`/`report` correctness (missing recent captures).
- Trigger D: release-gate regressions (`nils-cli-checks.sh` or coverage gate repeatedly fail).
- Trigger E: timezone/custom-range regressions around `--tz` or `--from/--to`.

## Rollback actions
1. Pause write-back automation immediately:
   - stop `memo-cli apply` jobs and keep capture-only mode (`add`, `list`, `search`, `report`).
2. Continue capture durability:
   - keep `memo-cli add` enabled; pause `update`/`delete` until issue is triaged.
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
