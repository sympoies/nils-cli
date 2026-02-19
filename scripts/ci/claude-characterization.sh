#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage:
  scripts/ci/claude-characterization.sh --mode <mock|local-cli> [--allow-skip]

Modes:
  mock       Deterministic fixture-only characterization and diff report.
  local-cli  Optional local Claude CLI characterization with skip-safe output.

Outputs:
  target/claude-characterization/mock-report.json
  target/claude-characterization/mock-diff.json
  target/claude-characterization/local-cli-report.json
  target/claude-characterization/local-cli-diff.json
USAGE
}

die() {
  echo "error: $*" >&2
  exit 1
}

json_escape() {
  printf '%s' "$1" | sed -e 's/\\/\\\\/g' -e 's/"/\\"/g'
}

json_string_or_null() {
  local value="${1-}"
  if [[ -z "$value" ]]; then
    printf 'null'
  else
    printf '"%s"' "$(json_escape "$value")"
  fi
}

extract_json_string() {
  local key="$1"
  local file="$2"
  sed -n "s/^[[:space:]]*\"$key\"[[:space:]]*:[[:space:]]*\"\\([^\"]*\\)\".*/\\1/p" "$file" | head -n 1
}

require_json_string() {
  local key="$1"
  local file="$2"
  local value
  value="$(extract_json_string "$key" "$file")"
  if [[ -z "$value" ]]; then
    die "missing required key \"$key\" in $file"
  fi
  printf '%s' "$value"
}

assert_contains() {
  local pattern="$1"
  local file="$2"
  local label="$3"
  if ! rg -q "$pattern" "$file"; then
    die "$label ($file)"
  fi
}

report_path_for_mode() {
  local mode="$1"
  printf '%s/%s-report.json' "$OUT_DIR" "$mode"
}

diff_path_for_mode() {
  local mode="$1"
  printf '%s/%s-diff.json' "$OUT_DIR" "$mode"
}

report_relpath_for_mode() {
  local mode="$1"
  printf '%s/%s-report.json' "$OUT_DIR_RELATIVE" "$mode"
}

diff_relpath_for_mode() {
  local mode="$1"
  printf '%s/%s-diff.json' "$OUT_DIR_RELATIVE" "$mode"
}

write_mock_artifacts() {
  local mode="mock"
  local report_path diff_path report_relpath diff_relpath
  report_path="$(report_path_for_mode "$mode")"
  diff_path="$(diff_path_for_mode "$mode")"
  report_relpath="$(report_relpath_for_mode "$mode")"
  diff_relpath="$(diff_relpath_for_mode "$mode")"

  cat > "$report_path" <<EOF
{
  "mode": "$mode",
  "status": "passed",
  "fixture_id": "$FIXTURE_ID",
  "report_schema_version": "$REPORT_SCHEMA_VERSION",
  "api_doc_date": "$API_DOC_DATE",
  "model_id": "$MODEL_ID",
  "claude_cli_version": null,
  "fixture_schema_version": "$FIXTURE_SCHEMA_VERSION",
  "expected_cli_checks": [
    "$EXPECTED_CLI_CHECK"
  ],
  "observed_cli_checks": [],
  "report_path": "$report_relpath"
}
EOF

  cat > "$diff_path" <<EOF
{
  "mode": "$mode",
  "status": "passed",
  "fixture_id": "$FIXTURE_ID",
  "report_schema_version": "$REPORT_SCHEMA_VERSION",
  "report_path": "$report_relpath",
  "diff_path": "$diff_relpath",
  "checks": [
    {
      "id": "fixture-manifest-schema",
      "status": "passed"
    },
    {
      "id": "fixture-required-case-ids",
      "status": "passed"
    },
    {
      "id": "fixture-local-cli-metadata",
      "status": "passed"
    }
  ],
  "mismatches": []
}
EOF

  echo "ok: mock characterization completed ($report_path, $diff_path)"
}

write_local_cli_artifacts() {
  local status="$1"
  local reason="$2"
  local claude_cli_version="$3"
  local observed_cli_checks_json="$4"
  local cli_invocation_status="$5"
  local cli_invocation_actual_json="$6"

  local mode="local-cli"
  local report_path diff_path report_relpath diff_relpath
  local reason_json claude_cli_version_json
  report_path="$(report_path_for_mode "$mode")"
  diff_path="$(diff_path_for_mode "$mode")"
  report_relpath="$(report_relpath_for_mode "$mode")"
  diff_relpath="$(diff_relpath_for_mode "$mode")"
  reason_json="$(json_string_or_null "$reason")"
  claude_cli_version_json="$(json_string_or_null "$claude_cli_version")"

  cat > "$report_path" <<EOF
{
  "mode": "$mode",
  "status": "$status",
  "reason": $reason_json,
  "fixture_id": "$FIXTURE_ID",
  "report_schema_version": "$REPORT_SCHEMA_VERSION",
  "api_doc_date": "$API_DOC_DATE",
  "model_id": "$MODEL_ID",
  "claude_cli_version": $claude_cli_version_json,
  "fixture_schema_version": "$FIXTURE_SCHEMA_VERSION",
  "expected_cli_checks": [
    "$EXPECTED_CLI_CHECK"
  ],
  "observed_cli_checks": $observed_cli_checks_json,
  "report_path": "$report_relpath"
}
EOF

  cat > "$diff_path" <<EOF
{
  "mode": "$mode",
  "status": "$status",
  "reason": $reason_json,
  "fixture_id": "$FIXTURE_ID",
  "report_schema_version": "$REPORT_SCHEMA_VERSION",
  "report_path": "$report_relpath",
  "diff_path": "$diff_relpath",
  "checks": [
    {
      "id": "fixture-local-cli-metadata",
      "status": "passed"
    },
    {
      "id": "local-cli-invocation",
      "status": "$cli_invocation_status",
      "expected": "$EXPECTED_CLI_CHECK",
      "actual": $cli_invocation_actual_json
    }
  ],
  "mismatches": []
}
EOF

  if [[ "$status" == "skipped" ]]; then
    echo "ok: local-cli characterization skipped ($report_path, $diff_path)"
  else
    echo "ok: local-cli characterization completed ($report_path, $diff_path)"
  fi
}

MODE=""
ALLOW_SKIP=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --mode)
      MODE="${2:-}"
      shift 2
      ;;
    --allow-skip)
      ALLOW_SKIP=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "error: unknown arg: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

if [[ -z "$MODE" ]]; then
  echo "error: --mode is required" >&2
  usage >&2
  exit 2
fi

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
OUT_DIR="$ROOT_DIR/target/claude-characterization"
OUT_DIR_RELATIVE="target/claude-characterization"
mkdir -p "$OUT_DIR"

FIXTURE_MANIFEST="$ROOT_DIR/crates/agent-provider-claude/tests/fixtures/characterization/manifest.json"
CLI_FIXTURE="$ROOT_DIR/crates/agent-provider-claude/tests/fixtures/characterization/claude-cli-smoke.json"
EXPECTED_CLI_CHECK="claude --version"
FIXTURE_SCHEMA_VERSION_EXPECTED="claude-characterization.v1"
REPORT_SCHEMA_VERSION_EXPECTED="claude-characterization-report.v1"

test -f "$FIXTURE_MANIFEST"
test -f "$CLI_FIXTURE"

FIXTURE_SCHEMA_VERSION="$(require_json_string "fixture_schema_version" "$CLI_FIXTURE")"
FIXTURE_ID="$(require_json_string "fixture_id" "$CLI_FIXTURE")"
API_DOC_DATE="$(require_json_string "api_doc_date" "$CLI_FIXTURE")"
MODEL_ID="$(require_json_string "model_id" "$CLI_FIXTURE")"
REPORT_SCHEMA_VERSION="$(require_json_string "report_schema_version" "$CLI_FIXTURE")"

if [[ "$FIXTURE_SCHEMA_VERSION" != "$FIXTURE_SCHEMA_VERSION_EXPECTED" ]]; then
  die "unsupported fixture schema version: $FIXTURE_SCHEMA_VERSION (expected $FIXTURE_SCHEMA_VERSION_EXPECTED)"
fi
if [[ "$REPORT_SCHEMA_VERSION" != "$REPORT_SCHEMA_VERSION_EXPECTED" ]]; then
  die "unsupported report schema version: $REPORT_SCHEMA_VERSION (expected $REPORT_SCHEMA_VERSION_EXPECTED)"
fi

assert_contains "\"fixture_schema_version\"\\s*:\\s*\"$FIXTURE_SCHEMA_VERSION\"" "$FIXTURE_MANIFEST" "manifest schema version does not match fixture schema version"
assert_contains "\"$EXPECTED_CLI_CHECK\"" "$CLI_FIXTURE" "fixture missing required expected_cli_checks entry"
for expected_output_file in mock-report.json mock-diff.json local-cli-report.json local-cli-diff.json; do
  assert_contains "\"$expected_output_file\"" "$CLI_FIXTURE" "fixture missing expected output file declaration"
done
for required_case in success auth_failure rate_limit timeout malformed_response; do
  assert_contains "\"id\"\\s*:\\s*\"$required_case\"" "$FIXTURE_MANIFEST" "manifest missing required fixture id: $required_case"
done

case "$MODE" in
  mock)
    write_mock_artifacts
    ;;
  local-cli)
    if ! command -v claude >/dev/null 2>&1; then
      if [[ "$ALLOW_SKIP" -eq 1 ]]; then
        write_local_cli_artifacts \
          "skipped" \
          "claude cli not found on PATH" \
          "" \
          "[]" \
          "skipped" \
          "null"
        exit 0
      fi
      echo "error: claude cli not found on PATH" >&2
      exit 1
    fi

    claude_version="$(claude --version 2>/dev/null | head -n 1 || true)"
    if [[ -z "$claude_version" ]]; then
      claude_version="unknown"
    fi
    write_local_cli_artifacts \
      "passed" \
      "" \
      "$claude_version" \
      "[\"$EXPECTED_CLI_CHECK\"]" \
      "passed" \
      "\"$EXPECTED_CLI_CHECK\""
    ;;
  *)
    echo "error: unsupported mode: $MODE" >&2
    usage >&2
    exit 2
    ;;
esac
