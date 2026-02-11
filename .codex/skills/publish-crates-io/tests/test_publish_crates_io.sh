#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
skill_root="$(cd "${script_dir}/.." && pwd)"
entrypoint="${skill_root}/scripts/publish-crates-io.sh"

fail() {
  echo "error: $*" >&2
  exit 1
}

assert_contains() {
  local file="$1"
  local pattern="$2"
  if ! rg -q -- "$pattern" "$file"; then
    echo "error: expected pattern '$pattern' in $file" >&2
    sed -n '1,220p' "$file" >&2 || true
    exit 1
  fi
}

create_mock_cargo() {
  local dir="$1"
  cat > "${dir}/cargo" <<'MOCK'
#!/usr/bin/env bash
set -euo pipefail

scenario="${MOCK_CARGO_SCENARIO:-success}"
state_file="${MOCK_CARGO_STATE:-/tmp/mock-cargo-state}"

if [[ "${1:-}" == "metadata" ]]; then
  cat <<'JSON'
{
  "packages": [
    {
      "name": "nils-a",
      "version": "1.2.3",
      "publish": null,
      "dependencies": []
    },
    {
      "name": "nils-b",
      "version": "1.2.4",
      "publish": null,
      "dependencies": []
    }
  ]
}
JSON
  exit 0
fi

if [[ "${1:-}" != "publish" ]]; then
  echo "unexpected cargo command: $*" >&2
  exit 1
fi

crate=""
dry_run=0
args=("$@")
for ((i=0; i<${#args[@]}; i++)); do
  arg="${args[$i]}"
  if [[ "$arg" == "-p" ]]; then
    crate="${args[$((i+1))]}"
  elif [[ "$arg" == "--dry-run" ]]; then
    dry_run=1
  fi
done

if [[ -z "$crate" ]]; then
  echo "missing crate name" >&2
  exit 1
fi

if [[ "$dry_run" -eq 1 ]]; then
  echo "dry-run ok for $crate"
  exit 0
fi

case "$scenario" in
  success)
    echo "published $crate"
    exit 0
    ;;
  rate-limit-stop)
    if [[ "$crate" == "nils-a" ]]; then
      echo "error: too many requests; retry after 120 seconds" >&2
      exit 1
    fi
    echo "published $crate"
    exit 0
    ;;
  rate-limit-once)
    key="${state_file}.${crate}"
    attempts=0
    if [[ -f "$key" ]]; then
      attempts="$(cat "$key")"
    fi
    attempts=$((attempts + 1))
    echo "$attempts" > "$key"
    if [[ "$crate" == "nils-a" && "$attempts" -eq 1 ]]; then
      echo "error: rate limit reached, retry after 0 seconds" >&2
      exit 1
    fi
    echo "published $crate on attempt $attempts"
    exit 0
    ;;
  *)
    echo "unknown scenario: $scenario" >&2
    exit 1
    ;;
esac
MOCK
  chmod +x "${dir}/cargo"
}

create_mock_status_script() {
  local dir="$1"
  cat > "${dir}/crates-io-status.sh" <<'MOCK'
#!/usr/bin/env bash
set -euo pipefail

scenario="${MOCK_STATUS_SCENARIO:-ok}"
calls_file="${MOCK_STATUS_CALLS:-/tmp/mock-status-calls}"
json_out=""
text_out=""

echo "$*" >> "$calls_file"

while [[ $# -gt 0 ]]; do
  case "${1:-}" in
    --json-out)
      json_out="${2:-}"
      shift 2
      ;;
    --text-out)
      text_out="${2:-}"
      shift 2
      ;;
    *)
      shift
      ;;
  esac
done

if [[ -n "$json_out" ]]; then
  mkdir -p "$(dirname "$json_out")"
  cat > "$json_out" <<'JSON'
{"summary":{"published":2,"missing":0,"error":0}}
JSON
fi
if [[ -n "$text_out" ]]; then
  mkdir -p "$(dirname "$text_out")"
  cat > "$text_out" <<'TEXT'
# mock status report
TEXT
fi

if [[ "$scenario" == "fail" ]]; then
  exit 1
fi
exit 0
MOCK
  chmod +x "${dir}/crates-io-status.sh"
}

create_temp_repo() {
  local repo_dir="$1"
  git init "$repo_dir" >/dev/null
  git -C "$repo_dir" config user.email "test@example.com"
  git -C "$repo_dir" config user.name "Test User"
  echo "seed" > "${repo_dir}/README.md"
  git -C "$repo_dir" add README.md
  git -C "$repo_dir" commit -m "init" >/dev/null
}

test_rate_limit_stop_reports_next_time() {
  local tmp
  tmp="$(mktemp -d)"
  local repo="${tmp}/repo"
  local bin_dir="${tmp}/bin"
  mkdir -p "$repo" "$bin_dir"
  create_temp_repo "$repo"
  create_mock_cargo "$bin_dir"

  local report="${tmp}/report-stop.md"
  set +e
  PATH="${bin_dir}:$PATH" MOCK_CARGO_SCENARIO="rate-limit-stop" \
    "$entrypoint" --crates "nils-a nils-b" --no-skip-existing --report-file "$report" --allow-dirty --skip-status-check \
    >"${tmp}/stdout.log" 2>"${tmp}/stderr.log"
  local rc=$?
  set -e

  [[ "$rc" -eq 1 ]] || fail "expected exit code 1 for default rate-limit stop, got $rc"
  assert_contains "$report" "Next eligible publish time"
  assert_contains "$report" "\\| nils-a \\| 1.2.3 \\| failed \\|"
  assert_contains "$report" "\\| nils-b \\| 1.2.4 \\| pending \\|"
}

test_wait_retry_finishes_all() {
  local tmp
  tmp="$(mktemp -d)"
  local repo="${tmp}/repo"
  local bin_dir="${tmp}/bin"
  mkdir -p "$repo" "$bin_dir"
  create_temp_repo "$repo"
  create_mock_cargo "$bin_dir"

  local report="${tmp}/report-wait.md"
  PATH="${bin_dir}:$PATH" MOCK_CARGO_SCENARIO="rate-limit-once" MOCK_CARGO_STATE="${tmp}/state" \
    PUBLISH_CRATES_IO_SLEEP_BIN="true" \
    "$entrypoint" --crates "nils-a nils-b" --wait-retry --no-skip-existing --report-file "$report" --allow-dirty --skip-status-check \
    >"${tmp}/stdout.log" 2>"${tmp}/stderr.log"

  assert_contains "$report" "\\| nils-a \\| 1.2.3 \\| published \\|"
  assert_contains "$report" "\\| nils-b \\| 1.2.4 \\| published \\|"
  assert_contains "$report" 'Failed: `0`'
}

test_status_snapshot_integration() {
  local tmp
  tmp="$(mktemp -d)"
  local repo="${tmp}/repo"
  local bin_dir="${tmp}/bin"
  mkdir -p "$repo" "$bin_dir"
  create_temp_repo "$repo"
  create_mock_cargo "$bin_dir"
  create_mock_status_script "$bin_dir"

  local report="${tmp}/report-status.md"
  local status_calls="${tmp}/status-calls.log"
  PATH="${bin_dir}:$PATH" MOCK_CARGO_SCENARIO="success" MOCK_STATUS_CALLS="$status_calls" \
    PUBLISH_CRATES_IO_STATUS_SCRIPT="${bin_dir}/crates-io-status.sh" \
    "$entrypoint" --crates "nils-a nils-b" --no-skip-existing --report-file "$report" --allow-dirty \
    >"${tmp}/stdout.log" 2>"${tmp}/stderr.log"

  assert_contains "$status_calls" "--fail-on-missing"
  assert_contains "$report" 'Status snapshot: `ok`'
  assert_contains "$report" 'Status JSON:'
  assert_contains "$report" 'Status text:'
  [[ -f "${report%.md}.status.json" ]] || fail "missing status json output"
  [[ -f "${report%.md}.status.md" ]] || fail "missing status text output"
}

if [[ ! -f "${skill_root}/SKILL.md" ]]; then
  echo "error: missing SKILL.md" >&2
  exit 1
fi
if [[ ! -f "$entrypoint" ]]; then
  echo "error: missing scripts/publish-crates-io.sh" >&2
  exit 1
fi
if [[ ! -f "${skill_root}/references/PUBLISH_REPORT_TEMPLATE.md" ]]; then
  echo "error: missing references/PUBLISH_REPORT_TEMPLATE.md" >&2
  exit 1
fi

test_rate_limit_stop_reports_next_time
test_wait_retry_finishes_all
test_status_snapshot_integration

echo "ok: project skill smoke checks passed"
