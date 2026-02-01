# codex-cli fixtures

This document defines deterministic fixtures and edge-case coverage for codex-cli parity tests.

## Fixture layout (proposed)

```
fixtures/codex-cli/
  auth/
    auth-active.json
    auth-missing-refresh.json
    auth-invalid-json.json
  secrets/
    alpha.json
    beta.json
    alpha-duplicate.json
    gamma-missing-tokens.json
  cache/
    secrets/
      auth.json.timestamp
      alpha.json.timestamp
    starship-rate-limits/
      alpha.kv
      beta.kv
      auth_<hash>.kv
  http/
    oauth-token-200.json
    oauth-token-401.json
    wham-usage-200.json
    wham-usage-401.json
```

## Auth/secrets JSON templates

Base structure used by `auth-active.json` and secret files:

```json
{
  "tokens": {
    "access_token": "hdr.<payload>.sig",
    "refresh_token": "refresh_token_value",
    "id_token": "hdr.<payload>.sig",
    "account_id": "acct_001"
  },
  "last_refresh": "2025-01-20T12:34:56Z"
}
```

Deterministic JWT payloads (base64url; no padding):

- `payload_alpha`:
  - `eyJzdWIiOiJ1c2VyXzEyMyIsImVtYWlsIjoiYWxwaGFAZXhhbXBsZS5jb20iLCJodHRwczovL2FwaS5vcGVuYWkuY29tL2F1dGgiOnsiY2hhdGdwdF91c2VyX2lkIjoidXNlcl8xMjMiLCJlbWFpbCI6ImFscGhhQGV4YW1wbGUuY29tIn19`
- `payload_beta`:
  - `eyJzdWIiOiJ1c2VyXzQ1NiIsImVtYWlsIjoiYmV0YUBleGFtcGxlLmNvbSIsImh0dHBzOi8vYXBpLm9wZW5haS5jb20vYXV0aCI6eyJjaGF0Z3B0X3VzZXJfaWQiOiJ1c2VyXzQ1NiIsImVtYWlsIjoiYmV0YUBleGFtcGxlLmNvbSJ9fQ`

Example tokens (header can be any base64url string):

- `hdr = eyJhbGciOiJub25lIiwidHlwIjoiSldUIn0`
- `access_token = hdr.<payload_alpha>.sig`
- `id_token = hdr.<payload_alpha>.sig`

Profiles:

- `secrets/alpha.json`: payload_alpha, account_id `acct_001`.
- `secrets/beta.json`: payload_beta, account_id `acct_002`.
- `secrets/alpha-duplicate.json`: same payload/email as alpha to trigger ambiguity.
- `auth/auth-active.json`: identical to `secrets/alpha.json` (exact hash match).
- `auth/auth-missing-refresh.json`: no refresh_token (to trigger exit 2 in refresh).
- `auth/auth-invalid-json.json`: invalid JSON (syntax error) for error paths.

## Cache fixtures

### Secrets timestamp cache

- `cache/secrets/<filename>.timestamp` contains ISO8601 `last_refresh` (e.g. `2025-01-20T12:34:56Z`).

### Starship cache KV

KV format (one per line):

```
fetched_at=1700000000
non_weekly_label=5h
non_weekly_remaining=94
non_weekly_reset_epoch=1700003600
weekly_remaining=88
weekly_reset_epoch=1700600000
```

Fixtures:

- `cache/starship-rate-limits/alpha.kv` - valid cache for alpha.
- `cache/starship-rate-limits/beta.kv` - valid cache for beta.
- `cache/starship-rate-limits/auth_<hash>.kv` - valid cache for auth file hash key.
- Invalid cache variants (missing weekly or non-weekly fields) to trigger errors.

## HTTP stubs

### OAuth token success (200)

`http/oauth-token-200.json`:

```json
{
  "access_token": "new_access",
  "refresh_token": "new_refresh",
  "id_token": "new_id",
  "token_type": "Bearer",
  "expires_in": 3600
}
```

### OAuth token error (401)

`http/oauth-token-401.json`:

```json
{
  "error": "invalid_grant",
  "error_description": "Refresh token expired"
}
```

### wham/usage success (200)

`http/wham-usage-200.json`:

```json
{
  "rate_limit": {
    "primary_window": {
      "limit_window_seconds": 18000,
      "used_percent": 12,
      "reset_at": 1700003600
    },
    "secondary_window": {
      "limit_window_seconds": 604800,
      "used_percent": 25,
      "reset_at": 1700600000
    }
  }
}
```

### wham/usage unauthorized (401)

`http/wham-usage-401.json`:

```json
{
  "error": "Unauthorized"
}
```

## Edge-case matrix

| Scenario | Inputs | Expected behavior | Exit code |
|---|---|---|---|
| Missing `codex` binary | `agent prompt` with dangerous enabled | stderr `missing binary: codex` (if wrapper checks); no exec | 1 |
| Missing `git` | `agent commit` | stderr `codex-commit-with-scope: missing binary: git` | 1 |
| Not a git repo | `agent commit` | stderr `codex-commit-with-scope: not a git repository` | 1 |
| No staged changes | `agent commit` (no auto-stage) | stderr `no staged changes` | 1 |
| Invalid `auth use` arg | `auth use ../x` | stderr `invalid secret name` | 64 |
| Ambiguous profile | `auth use alpha` with `alpha.json` and `alpha-duplicate.json` | stderr includes `identifier matches multiple secrets` + candidates | 2 |
| Missing secret | `auth use missing` | stderr `secret not found` | 1 |
| Missing refresh token | `auth refresh` on `auth-missing-refresh.json` | stderr `failed to read refresh token` | 2 |
| Refresh 401 then success | `auth refresh` with first 401 then 200 | refresh+retry, success message, timestamps updated | 0 |
| Refresh non-200 | `auth refresh` with 401/500 | stderr with error summary (if present) | 3 |
| Rate limits 401 refresh retry | `diag rate-limits` | refresh tokens, retry once, success | 0 |
| Rate limits 401 no-refresh | `diag rate-limits --no-refresh-auth` | no retry, stderr non-200 | 3 |
| `--cached` without cache | `diag rate-limits --cached` | stderr `cache not found` | 1 |
| `--cached` invalid cache | missing weekly/non-weekly data | stderr `invalid cache` | 1 |
| `--json` + `--cached` | invalid combo | usage error | 64 |
| `--all` + `--json` | invalid combo | usage error | 64 |
| `--all` empty secret dir | no secrets | stderr `no secrets found` | 1 |
| `--async` jobs invalid | `--jobs 0` or non-numeric | default to 5 | 0 |
| Async debug | `--async --debug` | prints captured per-account stderr after table | 0 or 1 |
| Starship stale output | cached but expired | prints cached output + `CODEX_STARSHIP_STALE_SUFFIX` | 0 |
| Starship disabled | `CODEX_STARSHIP_ENABLED=false` | prints nothing | 0 |
| `NO_COLOR` set | rate-limits table | no ANSI color output | 0 |

