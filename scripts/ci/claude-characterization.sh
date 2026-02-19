#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage:
  scripts/ci/claude-characterization.sh --mode <mock|local-cli> [--allow-skip]

Modes:
  mock       Validate fixture-based characterization metadata.
  local-cli  Collect local Claude CLI metadata (optional in CI).
EOF
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
mkdir -p "$OUT_DIR"

FIXTURE_MANIFEST="$ROOT_DIR/crates/agent-provider-claude/tests/fixtures/characterization/manifest.json"
CLI_FIXTURE="$ROOT_DIR/crates/agent-provider-claude/tests/fixtures/characterization/claude-cli-smoke.json"
API_DOC_DATE="2026-02-19"
MODEL_ID="claude-sonnet-4-5-20250929"
FIXTURE_SCHEMA_VERSION="claude-characterization.v1"

case "$MODE" in
  mock)
    test -f "$FIXTURE_MANIFEST"
    test -f "$CLI_FIXTURE"
    for required in success auth_failure rate_limit timeout malformed_response; do
      rg -q "\"id\"\\s*:\\s*\"$required\"" "$FIXTURE_MANIFEST"
    done
    cat > "$OUT_DIR/mock-report.json" <<EOF
{
  "mode": "mock",
  "status": "passed",
  "api_doc_date": "$API_DOC_DATE",
  "model_id": "$MODEL_ID",
  "fixture_schema_version": "$FIXTURE_SCHEMA_VERSION"
}
EOF
    echo "ok: mock characterization completed ($OUT_DIR/mock-report.json)"
    ;;
  local-cli)
    REPORT_PATH="$OUT_DIR/local-cli-report.json"
    if ! command -v claude >/dev/null 2>&1; then
      if [[ "$ALLOW_SKIP" -eq 1 ]]; then
        cat > "$REPORT_PATH" <<EOF
{
  "mode": "local-cli",
  "status": "skipped",
  "reason": "claude cli not found on PATH",
  "api_doc_date": "$API_DOC_DATE",
  "model_id": "$MODEL_ID",
  "claude_cli_version": null,
  "fixture_schema_version": "$FIXTURE_SCHEMA_VERSION"
}
EOF
        echo "ok: local-cli characterization skipped ($REPORT_PATH)"
        exit 0
      fi
      echo "error: claude cli not found on PATH" >&2
      exit 1
    fi

    CLAUDE_VERSION="$(claude --version 2>/dev/null | head -n 1 || true)"
    if [[ -z "$CLAUDE_VERSION" ]]; then
      CLAUDE_VERSION="unknown"
    fi
    cat > "$REPORT_PATH" <<EOF
{
  "mode": "local-cli",
  "status": "passed",
  "api_doc_date": "$API_DOC_DATE",
  "model_id": "$MODEL_ID",
  "claude_cli_version": "$CLAUDE_VERSION",
  "fixture_schema_version": "$FIXTURE_SCHEMA_VERSION"
}
EOF
    echo "ok: local-cli characterization completed ($REPORT_PATH)"
    ;;
  *)
    echo "error: unsupported mode: $MODE" >&2
    usage >&2
    exit 2
    ;;
esac
