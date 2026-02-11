# codex-cli JSON Consumers Runbook

## Scope
This runbook is for frontend/service callers that consume `codex-cli` machine output for:
- `diag rate-limits` (single/all/async)
- `auth login|use|save|refresh|auto-refresh|current|sync`

Contract source of truth:
- `docs/specs/codex-cli-diag-auth-json-contract-v1.md`
- `docs/specs/cli-service-json-contract-guideline-v1.md`

## Migration Checklist
1. Keep default text mode for humans; for automation always pass explicit JSON mode:
   - preferred: `--format json`
   - compatibility: `--json` on supported surfaces
2. Parse JSON `stdout` only; do not parse prose `stderr`.
3. Validate stable envelope keys first: `schema_version`, `command`, `ok`.
4. Parse `result` for single-entity responses and `results` for collection responses.
5. Route by `command` (`diag rate-limits`, `auth login`, `auth use`, `auth save`, `auth refresh`,
   `auth auto-refresh`, `auth current`, `auth sync`) and ignore unknown additive fields.
6. Enforce schema routing:
   - `diag rate-limits` => `schema_version=codex-cli.diag.rate-limits.v1`
   - `auth *` => `schema_version=codex-cli.auth.v1`

## Integration Notes
- `ok=false` means command-level failure; read top-level `error.code`.
- `ok=true` can still include partial failures in collection/per-target flows:
  - `diag rate-limits --all|--async`: inspect `results[*].status`
  - `auth auto-refresh`: inspect `result.targets[*].status`
- `auth login` exposes three stable method values in `result.method`:
  - `chatgpt-browser` (default `auth login`)
  - `chatgpt-device-code` (`auth login --device-code`)
  - `api-key` (`auth login --api-key`)
- `auth save` overwrite handling:
  - success path: check `result.overwritten` (`false` = new file, `true` = replaced existing file)
  - confirmation-required path: `ok=false`, `error.code=overwrite-confirmation-required`
- Treat `raw_usage` and other informational metadata as optional and unstable for strict parsing.

Example commands:
```bash
codex-cli diag rate-limits --format json alpha.json
codex-cli diag rate-limits --all --format json
codex-cli auth login --format json
codex-cli auth login --format json --device-code
codex-cli auth login --format json --api-key
codex-cli auth save --format json team-alpha.json
codex-cli auth save --format json --yes team-alpha.json
codex-cli auth auto-refresh --format json
codex-cli auth current --format json
```

## Do / Don't

Do:
- Pin logic to stable fields documented in the v1 contract.
- Handle both exit code and JSON envelope together.
- Keep idempotent retry behavior for transient failures.
- Store the last successful normalized summary for UI fallback.

Don't:
- Do not scrape human text output or `stderr` for machine logic.
- Do not assume informational fields (`raw_usage`, optional metadata) always exist.
- Do not treat partial failure as total success without checking per-item/per-target status.
- Do not rely on field ordering.

## Retry and Fallback Guidance

| Scenario | Signal | Guidance |
|---|---|---|
| Invalid CLI usage | exit `64` and/or `error.code=invalid-arguments` | Do not retry; fix call arguments/flags. |
| Save overwrite confirmation required | `ok=false` with `error.code=overwrite-confirmation-required` | Ask for explicit confirmation, then rerun with `auth save --format json --yes <secret.json>`. |
| Command-level transient failure | `ok=false` with timeout/network/auth endpoint code | Retry with bounded exponential backoff. |
| Partial failure in collection mode | `ok=true` with some `status=error` | Accept succeeded items; retry only failed targets. |
| Diag partial failure | failed `results[*].target_file` entries | Retry each failed target with `diag rate-limits --format json <secret.json>`. |
| Auth auto-refresh partial failure | `result.targets[*].status=failed` | Retry failed targets with `auth refresh --format json <secret.json>`. |
| No fresh remote data available | repeated transient failures | Fallback to your service-side cached last-success snapshot; do not fallback to parsing text mode. |

## Partial Failure Playbook
1. Parse successful items first and publish partial data to callers/UI.
2. Collect failed target list from `results[*].error` or `result.targets[*].reason`.
3. Retry failed targets only (single-target commands), capped by retry budget.
4. Return an aggregate status that keeps successful items and surfaces unresolved failures.
