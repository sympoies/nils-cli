#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage:
  scripts/crates-io-status.sh [options]

Options:
  --crate NAME            Add one crate (repeatable).
  --crates "A B,C"        Add multiple crates (space/comma separated).
  --all                   Query all crates from the list file.
  --list-file PATH        Default crate list file (default: release/crates-io-publish-order.txt).
  --version X.Y.Z         Check a specific version (accepts vX.Y.Z).
  --format MODE           text|json|both (default: text).
  --json-out PATH         Write JSON output to file.
  --text-out PATH         Write human-readable output to file.
  --fail-on-missing       Exit with non-zero when any crate is missing/error.
  --max-attempts N        HTTP retry attempts for transient errors (default: 3).
  --timeout-seconds N     HTTP timeout seconds per request (default: 15).
  -h, --help              Show help.

Environment:
  CRATES_IO_STATUS_CARGO_BIN      Override cargo executable (default: cargo)
  CRATES_IO_STATUS_PYTHON_BIN     Override python executable (default: python3)
  CRATES_IO_STATUS_API_BASE       Override crates.io API base
                                  (default: https://crates.io/api/v1)
USAGE
}

die() {
  echo "error: $*" >&2
  exit 1
}

note() {
  echo "info: $*" >&2
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

cargo_bin="${CRATES_IO_STATUS_CARGO_BIN:-cargo}"
python_bin="${CRATES_IO_STATUS_PYTHON_BIN:-python3}"
api_base="${CRATES_IO_STATUS_API_BASE:-https://crates.io/api/v1}"

format="text"
version=""
list_file="release/crates-io-publish-order.txt"
fail_on_missing=0
select_all=0
max_attempts=3
timeout_seconds=15
json_out=""
text_out=""
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
    --version)
      [[ $# -ge 2 ]] || die "--version requires a value"
      version="${2:-}"
      shift 2
      ;;
    --format)
      [[ $# -ge 2 ]] || die "--format requires a value"
      format="${2:-}"
      shift 2
      ;;
    --json-out)
      [[ $# -ge 2 ]] || die "--json-out requires a value"
      json_out="${2:-}"
      shift 2
      ;;
    --text-out)
      [[ $# -ge 2 ]] || die "--text-out requires a value"
      text_out="${2:-}"
      shift 2
      ;;
    --fail-on-missing)
      fail_on_missing=1
      shift
      ;;
    --max-attempts)
      [[ $# -ge 2 ]] || die "--max-attempts requires a value"
      max_attempts="${2:-}"
      shift 2
      ;;
    --timeout-seconds)
      [[ $# -ge 2 ]] || die "--timeout-seconds requires a value"
      timeout_seconds="${2:-}"
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

case "$format" in
  text|json|both) ;;
  *) die "--format must be one of: text, json, both" ;;
esac

[[ "$max_attempts" =~ ^[0-9]+$ ]] || die "--max-attempts must be an integer >= 0"
[[ "$timeout_seconds" =~ ^[0-9]+$ ]] || die "--timeout-seconds must be an integer >= 0"
if [[ "$max_attempts" -lt 1 ]]; then
  die "--max-attempts must be >= 1"
fi

if [[ "$select_all" -eq 1 && ${#selected_crates[@]} -gt 0 ]]; then
  die "--all cannot be combined with --crate/--crates"
fi

version="${version#v}"
if [[ -n "$version" ]]; then
  if ! [[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+([.-][0-9A-Za-z.-]+)?$ ]]; then
    die "invalid --version: $version"
  fi
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

if [[ "$format" == "both" && -z "$json_out" ]]; then
  agents_home="${AGENTS_HOME:-$HOME/.agents}"
  timestamp="$(date -u +%Y%m%dT%H%M%SZ)"
  json_out="${agents_home}/out/crates-io-status-${timestamp}.json"
fi
if [[ -n "$json_out" ]]; then
  mkdir -p "$(dirname "$json_out")"
fi
if [[ -n "$text_out" ]]; then
  mkdir -p "$(dirname "$text_out")"
fi

metadata_file="$(mktemp)"
trap 'rm -f "$metadata_file"' EXIT
"$cargo_bin" metadata --format-version 1 --no-deps > "$metadata_file"

"$python_bin" - "$metadata_file" "$format" "$json_out" "$text_out" "$version" "$fail_on_missing" "$api_base" "$max_attempts" "$timeout_seconds" "${selected_crates[@]}" <<'PY'
from __future__ import annotations

from datetime import datetime, timezone
import json
import pathlib
import re
import sys
import time
import urllib.error
import urllib.parse
import urllib.request

metadata_path = sys.argv[1]
fmt = sys.argv[2]
json_out = sys.argv[3]
text_out = sys.argv[4]
version_arg = sys.argv[5]
fail_on_missing = sys.argv[6] == "1"
api_base = sys.argv[7].rstrip("/")
max_attempts = int(sys.argv[8])
timeout_seconds = int(sys.argv[9])
selected = sys.argv[10:]

with open(metadata_path, "r", encoding="utf-8") as fp:
    metadata = json.load(fp)
packages = {pkg["name"]: pkg for pkg in metadata["packages"]}

errors: list[str] = []
for name in selected:
    if name not in packages:
        errors.append(f"selected crate '{name}' is not in this workspace")
if errors:
    for err in errors:
        print(f"error: {err}", file=sys.stderr)
    raise SystemExit(1)

checked_at = datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
mode = "explicit-version" if version_arg else "workspace-version"
user_agent = "nils-cli-crates-io-status/1.0"

def parse_retry_after(headers: dict[str, str], attempt: int) -> float:
    retry_after = headers.get("Retry-After", "").strip()
    if retry_after.isdigit():
        return float(max(0, int(retry_after)))
    return float(min(30, 2 ** max(0, attempt - 1)))

def parse_error_message(raw: str) -> str:
    raw = raw.strip()
    if not raw:
        return ""
    try:
        payload = json.loads(raw)
    except json.JSONDecodeError:
        payload = None
    if isinstance(payload, dict):
        errors = payload.get("errors")
        if isinstance(errors, list) and errors:
            first = errors[0]
            if isinstance(first, dict):
                detail = first.get("detail")
                if isinstance(detail, str):
                    return detail
            if isinstance(first, str):
                return first
        message = payload.get("error")
        if isinstance(message, str):
            return message
    return raw.splitlines()[0][:200]

def fetch_json(path: str) -> dict[str, object]:
    url = f"{api_base}{path}"
    request = urllib.request.Request(
        url,
        headers={
            "Accept": "application/json",
            "User-Agent": user_agent,
        },
    )
    last_error = ""
    for attempt in range(1, max_attempts + 1):
        try:
            with urllib.request.urlopen(request, timeout=timeout_seconds) as response:
                text = response.read().decode("utf-8", errors="replace")
                return {
                    "ok": True,
                    "status": response.status,
                    "json": json.loads(text),
                    "error": "",
                }
        except urllib.error.HTTPError as exc:
            body = exc.read().decode("utf-8", errors="replace")
            if exc.code == 404:
                return {"ok": False, "status": 404, "json": None, "error": "not found"}
            message = parse_error_message(body)
            last_error = f"http {exc.code}: {message}" if message else f"http {exc.code}"
            if exc.code in (429, 500, 502, 503, 504) and attempt < max_attempts:
                headers = {k: v for k, v in exc.headers.items()}
                time.sleep(parse_retry_after(headers, attempt))
                continue
            return {"ok": False, "status": exc.code, "json": None, "error": last_error}
        except urllib.error.URLError as exc:
            reason = str(exc.reason)
            last_error = f"url error: {reason}"
            if attempt < max_attempts:
                time.sleep(parse_retry_after({}, attempt))
                continue
            return {"ok": False, "status": None, "json": None, "error": last_error}
        except json.JSONDecodeError:
            last_error = "invalid json response"
            return {"ok": False, "status": None, "json": None, "error": last_error}
    return {"ok": False, "status": None, "json": None, "error": last_error or "request failed"}

results: list[dict[str, object]] = []
summary = {
    "total": len(selected),
    "published": 0,
    "yanked": 0,
    "missing": 0,
    "error": 0,
}

for crate in selected:
    pkg = packages[crate]
    publish_field = pkg.get("publish")
    publishable = publish_field != []
    workspace_version = pkg.get("version") or ""
    checked_version = version_arg if version_arg else workspace_version

    encoded_crate = urllib.parse.quote(crate, safe="")
    crate_resp = fetch_json(f"/crates/{encoded_crate}")
    version_resp = fetch_json(f"/crates/{encoded_crate}/{urllib.parse.quote(checked_version, safe='')}")

    latest_version = None
    crate_updated_at = None
    if crate_resp["ok"]:
        crate_payload = crate_resp["json"].get("crate", {})
        latest_version = crate_payload.get("newest_version")
        crate_updated_at = crate_payload.get("updated_at")

    status = "missing"
    published = False
    yanked = None
    published_at = None
    version_downloads = None
    error_message = ""

    if version_resp["ok"]:
        version_payload = version_resp["json"].get("version", {})
        published = True
        yanked = bool(version_payload.get("yanked", False))
        status = "yanked" if yanked else "published"
        published_at = version_payload.get("created_at")
        version_downloads = version_payload.get("downloads")
    elif version_resp["status"] == 404:
        status = "missing"
    else:
        status = "error"
        error_message = str(version_resp["error"] or "")

    if not crate_resp["ok"] and crate_resp["status"] not in (None, 404) and not error_message:
        error_message = str(crate_resp["error"] or "")
        status = "error"
    if crate_resp["status"] == 404 and status == "missing":
        error_message = "crate not found on crates.io"

    summary_key = "published"
    if status == "yanked":
        summary_key = "yanked"
    elif status == "missing":
        summary_key = "missing"
    elif status == "error":
        summary_key = "error"
    summary[summary_key] += 1

    results.append(
        {
            "crate": crate,
            "publishable": publishable,
            "workspace_version": workspace_version,
            "checked_version": checked_version,
            "status": status,
            "published": published,
            "yanked": yanked,
            "published_at": published_at,
            "latest_version": latest_version,
            "crate_updated_at": crate_updated_at,
            "version_downloads": version_downloads,
            "crate_exists": crate_resp["status"] != 404,
            "error": error_message or None,
        }
    )

payload = {
    "checked_at": checked_at,
    "query": {
        "mode": mode,
        "target_version": version_arg or None,
    },
    "summary": summary,
    "results": results,
}

def render_text(data: dict[str, object]) -> str:
    lines: list[str] = []
    query = data["query"]
    summary = data["summary"]
    rows = data["results"]
    lines.append("# crates.io Status Report")
    lines.append("")
    lines.append(f"- Checked at (UTC): `{data['checked_at']}`")
    lines.append(f"- Query mode: `{query['mode']}`")
    if query["target_version"]:
        lines.append(f"- Target version: `{query['target_version']}`")
    lines.append(f"- Total crates: `{summary['total']}`")
    lines.append(f"- Published: `{summary['published']}`")
    lines.append(f"- Yanked: `{summary['yanked']}`")
    lines.append(f"- Missing: `{summary['missing']}`")
    lines.append(f"- Error: `{summary['error']}`")
    lines.append("")
    lines.append("| Crate | Workspace | Checked | Status | Latest | Published at (UTC) | Note |")
    lines.append("|---|---:|---:|---|---:|---|---|")
    for row in rows:
        note = row["error"] or "-"
        lines.append(
            "| {crate} | {workspace_version} | {checked_version} | {status} | {latest_version} | {published_at} | {note} |".format(
                crate=row["crate"],
                workspace_version=row["workspace_version"] or "-",
                checked_version=row["checked_version"] or "-",
                status=row["status"],
                latest_version=row["latest_version"] or "-",
                published_at=row["published_at"] or "-",
                note=re.sub(r"[\r\n\t]+", " ", str(note)),
            )
        )
    return "\n".join(lines) + "\n"

json_text = json.dumps(payload, ensure_ascii=False, indent=2)
text_report = render_text(payload)

if fmt == "json":
    if json_out:
        path = pathlib.Path(json_out)
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json_text + "\n", encoding="utf-8")
    else:
        sys.stdout.write(json_text + "\n")
elif fmt == "text":
    if text_out:
        path = pathlib.Path(text_out)
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text_report, encoding="utf-8")
    else:
        sys.stdout.write(text_report)
else:
    if text_out:
        path = pathlib.Path(text_out)
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text_report, encoding="utf-8")
    else:
        sys.stdout.write(text_report)
    if json_out:
        path = pathlib.Path(json_out)
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json_text + "\n", encoding="utf-8")
    else:
        sys.stdout.write(json_text + "\n")

if fail_on_missing and any(r["status"] not in {"published", "yanked"} for r in results):
    raise SystemExit(1)
PY

if [[ -n "$json_out" ]]; then
  note "json output: $json_out"
fi
if [[ -n "$text_out" ]]; then
  note "text output: $text_out"
fi
