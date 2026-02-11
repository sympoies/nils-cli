#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage:
  <ENTRYPOINT> [options]

Options:
  --crate NAME               Add one crate (repeatable).
  --crates "A B,C"           Add multiple crates (space/comma separated).
  --all                      Publish all crates from the list file.
  --list-file PATH           Crate order file for --all/default.
  --publish                  Run dry-run + publish (default).
  --dry-run-only             Only run cargo publish --dry-run.
  --wait-retry               On rate-limit errors, wait and retry until completion.
  --max-retries N            Max publish retries per crate in --wait-retry mode (0 = unlimited).
  --default-retry-seconds N  Fallback wait seconds when retry time cannot be parsed (default: 300).
  --registry NAME            Optional cargo registry name (blank = crates.io).
  --skip-existing            Skip already-published versions on crates.io (default for publish mode).
  --no-skip-existing         Do not skip existing crate versions.
  --allow-dirty              Allow dirty worktree in publish mode.
  --skip-status-check        Do not run post-run crates.io status snapshot.
  --status-script PATH       Override status snapshot script path.
  --status-json-file PATH    Override status snapshot JSON output path.
  --status-text-file PATH    Override status snapshot text output path.
  --report-file PATH         Markdown report output path.
  -h, --help                 Show help.

Examples:
  <ENTRYPOINT> --crate nils-codex-cli
  <ENTRYPOINT> --crates "nils-common nils-term" --wait-retry
  <ENTRYPOINT> --all --wait-retry --report-file "$CODEX_HOME/out/publish-report.md"
USAGE
}

die() {
  echo "error: $*" >&2
  exit 1
}

note() {
  echo "info: $*" >&2
}

warn() {
  echo "warning: $*" >&2
}

contains() {
  local needle="$1"
  shift
  local item
  for item in "$@"; do
    if [[ "$item" == "$needle" ]]; then
      return 0
    fi
  done
  return 1
}

append_crates_from_words() {
  local raw="$1"
  local item
  raw="${raw//,/ }"
  for item in $raw; do
    [[ -n "$item" ]] || continue
    selected_crates+=("$item")
  done
}

append_crates_from_file() {
  local path="$1"
  [[ -f "$path" ]] || die "crate list file not found: $path"
  local line trimmed
  while IFS= read -r line || [[ -n "$line" ]]; do
    trimmed="$(printf '%s' "$line" | sed -E 's/[[:space:]]*#.*$//; s/^[[:space:]]+//; s/[[:space:]]+$//')"
    [[ -n "$trimmed" ]] || continue
    selected_crates+=("$trimmed")
  done < "$path"
}

sanitize_field() {
  printf '%s' "$1" | tr '\n\t' '  '
}

now_utc() {
  date -u +"%Y-%m-%dT%H:%M:%SZ"
}

add_seconds_utc() {
  "$python_bin" - "$1" <<'PY'
from __future__ import annotations

from datetime import datetime, timedelta, timezone
import sys

seconds = int(sys.argv[1])
base = datetime.now(timezone.utc) + timedelta(seconds=seconds)
print(base.strftime("%Y-%m-%dT%H:%M:%SZ"))
PY
}

selected_crate_version() {
  local metadata_path="$1"
  local crate="$2"
  "$python_bin" - "$metadata_path" "$crate" <<'PY'
from __future__ import annotations

import json
import sys

metadata_path, crate = sys.argv[1], sys.argv[2]
with open(metadata_path, "r", encoding="utf-8") as fp:
    metadata = json.load(fp)
for pkg in metadata["packages"]:
    if pkg.get("name") == crate:
        print(pkg["version"])
        raise SystemExit(0)
raise SystemExit(1)
PY
}

crate_version_exists_on_crates_io() {
  local crate="$1"
  local version="$2"
  "$python_bin" - "$crate" "$version" <<'PY'
from __future__ import annotations

import sys
import urllib.error
import urllib.request

crate, version = sys.argv[1], sys.argv[2]
url = f"https://crates.io/api/v1/crates/{crate}/{version}"
try:
    urllib.request.urlopen(url, timeout=15)
except urllib.error.HTTPError as exc:
    if exc.code == 404:
        raise SystemExit(1)
    raise
raise SystemExit(0)
PY
}

is_rate_limited_log() {
  local log_file="$1"
  "$python_bin" - "$log_file" <<'PY'
from __future__ import annotations

import re
import sys

text = open(sys.argv[1], "r", encoding="utf-8", errors="replace").read().lower()
patterns = [
    r"\b429\b",
    r"too many requests",
    r"rate limit",
    r"retry[- ]?after",
    r"please wait",
]
if any(re.search(p, text) for p in patterns):
    raise SystemExit(0)
raise SystemExit(1)
PY
}

extract_retry_seconds() {
  local log_file="$1"
  local fallback="$2"
  "$python_bin" - "$log_file" "$fallback" <<'PY'
from __future__ import annotations

import re
import sys

text = open(sys.argv[1], "r", encoding="utf-8", errors="replace").read().lower()
fallback = int(sys.argv[2])

patterns = [
    (r"retry[- ]?after[^0-9]*(\d+)", 1),
    (r"(\d+)\s*seconds?", 1),
    (r"(\d+)\s*secs?", 1),
    (r"(\d+)\s*minutes?", 60),
    (r"(\d+)\s*mins?", 60),
    (r"(\d+)\s*hours?", 3600),
    (r"(\d+)\s*hrs?", 3600),
]
for pattern, multiplier in patterns:
    match = re.search(pattern, text)
    if match:
        value = int(match.group(1)) * multiplier
        print(value)
        raise SystemExit(0)

print(max(0, fallback))
PY
}

extract_error_line() {
  local log_file="$1"
  "$python_bin" - "$log_file" <<'PY'
from __future__ import annotations

import sys

lines = [line.strip() for line in open(sys.argv[1], "r", encoding="utf-8", errors="replace").read().splitlines()]
for line in lines:
    if "error:" in line.lower():
        print(line)
        raise SystemExit(0)
for line in reversed(lines):
    if line:
        print(line)
        raise SystemExit(0)
print("unknown error")
PY
}

run_with_log() {
  local log_file="$1"
  shift
  set +e
  "$@" 2>&1 | tee "$log_file"
  local rc=${PIPESTATUS[0]}
  set -e
  return "$rc"
}

record_row() {
  local crate="$1"
  local version="$2"
  local status="$3"
  local started_at="$4"
  local ended_at="$5"
  local attempts="$6"
  local note_msg="$7"
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "$crate" \
    "$version" \
    "$status" \
    "$started_at" \
    "$ended_at" \
    "$attempts" \
    "$(sanitize_field "$note_msg")" >> "$rows_file"
}

write_report() {
  local path="$1"
  local mode_label="$2"
  local wait_label="$3"
  local started="$4"
  local ended="$5"
  local selected_total="$6"
  local next_retry="$7"
  local status_check_state="$8"
  local status_check_rc="$9"
  local status_json_file="${10}"
  local status_text_file="${11}"
  "$python_bin" - "$rows_file" "$path" "$mode_label" "$wait_label" "$started" "$ended" "$selected_total" "$next_retry" "$status_check_state" "$status_check_rc" "$status_json_file" "$status_text_file" <<'PY'
from __future__ import annotations

import csv
import pathlib
import sys

rows_file, out_path, mode_label, wait_label, started, ended, selected_total, next_retry, status_check_state, status_check_rc, status_json_file, status_text_file = sys.argv[1:13]
rows = []
with open(rows_file, "r", encoding="utf-8") as fp:
    reader = csv.DictReader(
        fp,
        fieldnames=["crate", "version", "status", "started_at", "ended_at", "attempts", "note"],
        delimiter="\t",
    )
    for row in reader:
        rows.append(row)

counts = {
    "published": 0,
    "skipped": 0,
    "dry-run-ok": 0,
    "failed": 0,
    "pending": 0,
}
for row in rows:
    status = row["status"]
    if status in counts:
        counts[status] += 1

def table(filter_fn):
    selected = [r for r in rows if filter_fn(r)]
    if not selected:
        return "_None_\n"
    out = ["| Crate | Version | Status | Start (UTC) | End (UTC) | Attempts | Note |", "|---|---:|---|---|---|---:|---|"]
    for row in selected:
        out.append(
            "| {crate} | {version} | {status} | {started_at} | {ended_at} | {attempts} | {note} |".format(
                crate=row["crate"] or "-",
                version=row["version"] or "-",
                status=row["status"] or "-",
                started_at=row["started_at"] or "-",
                ended_at=row["ended_at"] or "-",
                attempts=row["attempts"] or "0",
                note=row["note"] or "-",
            )
        )
    return "\n".join(out) + "\n"

content = []
content.append("# crates.io Publish Report\n")
content.append("## Summary\n")
content.append(f"- Mode: `{mode_label}`")
content.append(f"- Retry behavior: `{wait_label}`")
content.append(f"- Started (UTC): `{started}`")
content.append(f"- Ended (UTC): `{ended}`")
content.append(f"- Selected crates: `{selected_total}`")
content.append(f"- Published: `{counts['published']}`")
content.append(f"- Skipped existing: `{counts['skipped']}`")
content.append(f"- Dry-run only: `{counts['dry-run-ok']}`")
content.append(f"- Failed: `{counts['failed']}`")
content.append(f"- Not attempted: `{counts['pending']}`")
if next_retry:
    content.append(f"- Next eligible publish time (UTC): `{next_retry}`")
content.append(f"- Status snapshot: `{status_check_state}`")
if status_check_rc:
    content.append(f"- Status snapshot exit code: `{status_check_rc}`")
if status_json_file:
    content.append(f"- Status JSON: `{status_json_file}`")
if status_text_file:
    content.append(f"- Status text: `{status_text_file}`")
content.append("")
content.append("## Successful Uploads\n")
content.append(table(lambda r: r["status"] == "published"))
content.append("## Failed Uploads\n")
content.append(table(lambda r: r["status"] == "failed"))
content.append("## Full Attempts\n")
content.append(table(lambda _r: True))

path = pathlib.Path(out_path)
path.parent.mkdir(parents=True, exist_ok=True)
path.write_text("\n".join(content), encoding="utf-8")
PY
}

cargo_bin="${PUBLISH_CRATES_IO_CARGO_BIN:-cargo}"
python_bin="${PUBLISH_CRATES_IO_PYTHON_BIN:-python3}"
sleep_bin="${PUBLISH_CRATES_IO_SLEEP_BIN:-sleep}"

mode="publish"
wait_retry=0
max_retries=0
default_retry_seconds=300
list_file="release/crates-io-publish-order.txt"
registry=""
skip_existing=1
allow_dirty=0
select_all=0
report_file=""
skip_status_check=0
status_script="${PUBLISH_CRATES_IO_STATUS_SCRIPT:-}"
status_json_file=""
status_text_file=""
declare -a selected_crates=()

while [[ $# -gt 0 ]]; do
  case "${1:-}" in
    --crate)
      [[ $# -ge 2 ]] || die "--crate requires a value"
      selected_crates+=("${2:-}")
      shift 2
      ;;
    --crates)
      [[ $# -ge 2 ]] || die "--crates requires a value"
      append_crates_from_words "${2:-}"
      shift 2
      ;;
    --all)
      select_all=1
      shift
      ;;
    --list-file)
      [[ $# -ge 2 ]] || die "--list-file requires a value"
      list_file="${2:-}"
      shift 2
      ;;
    --publish)
      mode="publish"
      shift
      ;;
    --dry-run-only)
      mode="dry-run-only"
      shift
      ;;
    --wait-retry)
      wait_retry=1
      shift
      ;;
    --max-retries)
      [[ $# -ge 2 ]] || die "--max-retries requires a value"
      max_retries="${2:-}"
      shift 2
      ;;
    --default-retry-seconds)
      [[ $# -ge 2 ]] || die "--default-retry-seconds requires a value"
      default_retry_seconds="${2:-}"
      shift 2
      ;;
    --registry)
      [[ $# -ge 2 ]] || die "--registry requires a value"
      registry="${2:-}"
      shift 2
      ;;
    --skip-existing)
      skip_existing=1
      shift
      ;;
    --no-skip-existing)
      skip_existing=0
      shift
      ;;
    --allow-dirty)
      allow_dirty=1
      shift
      ;;
    --skip-status-check)
      skip_status_check=1
      shift
      ;;
    --status-script)
      [[ $# -ge 2 ]] || die "--status-script requires a value"
      status_script="${2:-}"
      shift 2
      ;;
    --status-json-file)
      [[ $# -ge 2 ]] || die "--status-json-file requires a value"
      status_json_file="${2:-}"
      shift 2
      ;;
    --status-text-file)
      [[ $# -ge 2 ]] || die "--status-text-file requires a value"
      status_text_file="${2:-}"
      shift 2
      ;;
    --report-file)
      [[ $# -ge 2 ]] || die "--report-file requires a value"
      report_file="${2:-}"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "error: unknown argument: ${1:-}" >&2
      usage >&2
      exit 2
      ;;
  esac
done

[[ "$max_retries" =~ ^[0-9]+$ ]] || die "--max-retries must be an integer >= 0"
[[ "$default_retry_seconds" =~ ^[0-9]+$ ]] || die "--default-retry-seconds must be an integer >= 0"

if [[ "$select_all" -eq 1 && ${#selected_crates[@]} -gt 0 ]]; then
  die "--all cannot be combined with --crate/--crates"
fi

repo_root="$(git rev-parse --show-toplevel 2>/dev/null || true)"
[[ -n "$repo_root" ]] || die "must run inside a git work tree"
cd "$repo_root"

if [[ "$select_all" -eq 1 || ${#selected_crates[@]} -eq 0 ]]; then
  append_crates_from_file "$list_file"
fi

declare -a deduped_crates=()
for crate in "${selected_crates[@]}"; do
  [[ "$crate" =~ ^[A-Za-z0-9_-]+$ ]] || die "invalid crate name: '$crate'"
  if ! contains "$crate" "${deduped_crates[@]}"; then
    deduped_crates+=("$crate")
  fi
done
selected_crates=("${deduped_crates[@]}")
[[ ${#selected_crates[@]} -gt 0 ]] || die "no crates selected"

metadata_file="$(mktemp)"
rows_file="$(mktemp)"
cleanup() {
  rm -f "$metadata_file" "$rows_file"
}
trap cleanup EXIT

"$cargo_bin" metadata --format-version 1 --no-deps > "$metadata_file"

"$python_bin" - "$metadata_file" "${selected_crates[@]}" <<'PY'
from __future__ import annotations

import json
import sys

metadata_path = sys.argv[1]
selected = sys.argv[2:]
with open(metadata_path, "r", encoding="utf-8") as fp:
    metadata = json.load(fp)
packages = {pkg["name"]: pkg for pkg in metadata["packages"]}

errors: list[str] = []
order = {name: idx for idx, name in enumerate(selected)}

for name in selected:
    pkg = packages.get(name)
    if pkg is None:
        errors.append(f"selected crate '{name}' is not in this workspace")
        continue
    if pkg.get("publish") == []:
        errors.append(f"selected crate '{name}' has publish=false")

for name in selected:
    pkg = packages.get(name)
    if pkg is None:
        continue
    for dep in pkg.get("dependencies", []):
        dep_name = dep.get("name")
        if dep.get("path") and dep_name in order and order[dep_name] > order[name]:
            errors.append(
                f"publish order invalid: '{name}' depends on '{dep_name}', "
                "so dependency must appear earlier in the crate list"
            )

if errors:
    for err in errors:
        print(f"error: {err}", file=sys.stderr)
    raise SystemExit(1)
PY

if [[ "$mode" == "publish" && "$allow_dirty" -eq 0 ]]; then
  if [[ -n "$(git status --porcelain)" ]]; then
    die "working tree is not clean; commit/stash changes or use --allow-dirty"
  fi
fi

if [[ -z "$report_file" ]]; then
  codex_home="${CODEX_HOME:-$HOME/.codex}"
  timestamp="$(date -u +%Y%m%dT%H%M%SZ)"
  report_file="${codex_home}/out/crates-io-publish-report-${timestamp}.md"
fi
mkdir -p "$(dirname "$report_file")"

if [[ -z "$status_script" ]]; then
  status_script="${repo_root}/scripts/crates-io-status.sh"
fi
if [[ -z "$status_json_file" ]]; then
  status_json_file="${report_file%.md}.status.json"
fi
if [[ -z "$status_text_file" ]]; then
  status_text_file="${report_file%.md}.status.md"
fi
mkdir -p "$(dirname "$status_json_file")" "$(dirname "$status_text_file")"

declare -a cargo_args=(--locked)
if [[ -n "$registry" ]]; then
  cargo_args+=(--registry "$registry")
fi
if [[ "$allow_dirty" -eq 1 ]]; then
  cargo_args+=(--allow-dirty)
fi

if [[ "$mode" == "dry-run-only" ]]; then
  skip_existing=0
fi

note "mode: $mode"
note "crates: ${selected_crates[*]}"
note "wait retry: $wait_retry"
note "report: $report_file"
if [[ "$skip_status_check" -eq 1 ]]; then
  note "status snapshot: disabled"
else
  note "status snapshot script: $status_script"
fi

run_started_at="$(now_utc)"
halted=0
halt_reason=""
next_retry_at=""
published_count=0
skipped_count=0
dry_run_ok_count=0
failed_count=0
pending_count=0
status_check_state="skipped"
status_check_rc=""

for ((idx=0; idx<${#selected_crates[@]}; idx++)); do
  crate="${selected_crates[$idx]}"
  version="$(selected_crate_version "$metadata_file" "$crate" || true)"
  [[ -n "$version" ]] || version="unknown"
  crate_started_at="$(now_utc)"

  if [[ "$halted" -eq 1 ]]; then
    record_row "$crate" "$version" "pending" "" "" "0" "not attempted due to earlier halt: $halt_reason"
    pending_count=$((pending_count + 1))
    continue
  fi

  if [[ "$mode" == "publish" && "$skip_existing" -eq 1 && -z "$registry" ]]; then
    if crate_version_exists_on_crates_io "$crate" "$version"; then
      crate_ended_at="$(now_utc)"
      note "skip ${crate} v${version} (already published on crates.io)"
      record_row "$crate" "$version" "skipped" "$crate_started_at" "$crate_ended_at" "0" "already exists on crates.io"
      skipped_count=$((skipped_count + 1))
      continue
    fi
  fi

  dry_log="$(mktemp)"
  note "[dry-run] cargo publish -p ${crate} --dry-run ${cargo_args[*]}"
  if ! run_with_log "$dry_log" "$cargo_bin" publish -p "$crate" --dry-run "${cargo_args[@]}"; then
    crate_ended_at="$(now_utc)"
    err_line="$(extract_error_line "$dry_log")"
    record_row "$crate" "$version" "failed" "$crate_started_at" "$crate_ended_at" "0" "dry-run failed: ${err_line}"
    rm -f "$dry_log"
    failed_count=$((failed_count + 1))
    halted=1
    halt_reason="dry-run failed on ${crate}"
    continue
  fi
  rm -f "$dry_log"

  if [[ "$mode" == "dry-run-only" ]]; then
    crate_ended_at="$(now_utc)"
    record_row "$crate" "$version" "dry-run-ok" "$crate_started_at" "$crate_ended_at" "0" "dry-run passed"
    dry_run_ok_count=$((dry_run_ok_count + 1))
    continue
  fi

  attempts=0
  while true; do
    attempts=$((attempts + 1))
    publish_log="$(mktemp)"
    note "[publish] cargo publish -p ${crate} ${cargo_args[*]} (attempt ${attempts})"
    if run_with_log "$publish_log" "$cargo_bin" publish -p "$crate" "${cargo_args[@]}"; then
      crate_ended_at="$(now_utc)"
      record_row "$crate" "$version" "published" "$crate_started_at" "$crate_ended_at" "$attempts" "publish succeeded"
      rm -f "$publish_log"
      published_count=$((published_count + 1))
      break
    fi

    if is_rate_limited_log "$publish_log"; then
      retry_seconds="$(extract_retry_seconds "$publish_log" "$default_retry_seconds")"
      retry_at="$(add_seconds_utc "$retry_seconds")"
      if [[ "$wait_retry" -eq 1 ]]; then
        if [[ "$max_retries" -gt 0 && "$attempts" -ge "$max_retries" ]]; then
          crate_ended_at="$(now_utc)"
          record_row "$crate" "$version" "failed" "$crate_started_at" "$crate_ended_at" "$attempts" "rate-limited and hit max retries (${max_retries}); next retry at ${retry_at}"
          rm -f "$publish_log"
          failed_count=$((failed_count + 1))
          halted=1
          halt_reason="max retries reached on ${crate}"
          next_retry_at="$retry_at"
          break
        fi
        warn "rate-limited on ${crate}; waiting ${retry_seconds}s (next retry at ${retry_at})"
        rm -f "$publish_log"
        "$sleep_bin" "$retry_seconds"
        continue
      fi

      crate_ended_at="$(now_utc)"
      record_row "$crate" "$version" "failed" "$crate_started_at" "$crate_ended_at" "$attempts" "rate-limited; next eligible publish time: ${retry_at}"
      rm -f "$publish_log"
      failed_count=$((failed_count + 1))
      halted=1
      halt_reason="rate limit on ${crate}"
      next_retry_at="$retry_at"
      break
    fi

    crate_ended_at="$(now_utc)"
    err_line="$(extract_error_line "$publish_log")"
    record_row "$crate" "$version" "failed" "$crate_started_at" "$crate_ended_at" "$attempts" "publish failed: ${err_line}"
    rm -f "$publish_log"
    failed_count=$((failed_count + 1))
    halted=1
    halt_reason="publish failed on ${crate}"
    break
  done
done

if [[ "$skip_status_check" -eq 0 ]]; then
  if [[ -x "$status_script" ]]; then
    declare -a status_cmd=("$status_script" --format both --json-out "$status_json_file" --text-out "$status_text_file")
    for crate in "${selected_crates[@]}"; do
      status_cmd+=(--crate "$crate")
    done
    if [[ "$mode" == "publish" && "$failed_count" -eq 0 && "$pending_count" -eq 0 ]]; then
      status_cmd+=(--fail-on-missing)
    fi

    note "running status snapshot"
    set +e
    "${status_cmd[@]}"
    status_check_rc="$?"
    set -e
    if [[ "$status_check_rc" == "0" ]]; then
      status_check_state="ok"
    else
      status_check_state="failed"
      warn "status snapshot failed with exit code ${status_check_rc}"
      if [[ "$failed_count" -eq 0 && "$pending_count" -eq 0 ]]; then
        failed_count=$((failed_count + 1))
      fi
    fi
  else
    warn "status snapshot script missing or not executable: $status_script"
    status_check_state="skipped"
  fi
fi

run_ended_at="$(now_utc)"
mode_label="$mode"
wait_label="stop-on-rate-limit"
if [[ "$wait_retry" -eq 1 ]]; then
  wait_label="wait-and-retry"
fi
write_report "$report_file" "$mode_label" "$wait_label" "$run_started_at" "$run_ended_at" "${#selected_crates[@]}" "$next_retry_at" "$status_check_state" "$status_check_rc" "$status_json_file" "$status_text_file"

note "report written: $report_file"
if [[ "$status_check_state" == "ok" ]]; then
  note "status json: $status_json_file"
  note "status text: $status_text_file"
fi
note "published=${published_count} skipped=${skipped_count} dry-run-ok=${dry_run_ok_count} failed=${failed_count} pending=${pending_count}"

if [[ "$failed_count" -gt 0 || "$pending_count" -gt 0 ]]; then
  exit 1
fi

exit 0
