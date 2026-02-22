#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage:
  llm_svg_pipeline.sh --intent <text> --out-svg <path> [--dry-run]

Env:
  SVG_LLM_CMD   Optional command that returns SVG (or text containing one SVG) on stdout.
USAGE
}

intent=""
out_svg=""
dry_run=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --intent)
      intent="$2"
      shift 2
      ;;
    --out-svg)
      out_svg="$2"
      shift 2
      ;;
    --dry-run)
      dry_run=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "llm_svg_pipeline: unknown arg: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

if [[ -z "$intent" || -z "$out_svg" ]]; then
  echo "llm_svg_pipeline: --intent and --out-svg are required" >&2
  usage >&2
  exit 2
fi

script_dir="$(cd "$(dirname "$0")" && pwd)"
repo_root="$(cd "$script_dir/../../.." && pwd)"

system_prompt="$repo_root/crates/image-processing/assets/llm-svg-system-prompt.md"
contract_prompt="$repo_root/crates/image-processing/assets/llm-svg-output-contract.md"

if [[ ! -f "$system_prompt" || ! -f "$contract_prompt" ]]; then
  echo "llm_svg_pipeline: missing prompt assets under crates/image-processing/assets" >&2
  exit 1
fi

mkdir -p "$(dirname "$out_svg")"
base="${out_svg%.*}"
prompt_file="$base.prompt.md"
raw_file="$base.raw.txt"
candidate_file="$base.candidate.svg"
validate_json="$base.validate.json"
repair_prompt="$base.repair.prompt.md"

cat > "$prompt_file" <<PROMPT
# Intent
$intent

# System Prompt
$(cat "$system_prompt")

# Output Contract
$(cat "$contract_prompt")
PROMPT

candidate_svg=""

if [[ -n "${SVG_LLM_CMD:-}" ]]; then
  if ! sh -c "$SVG_LLM_CMD" >"$raw_file" 2>"$base.llm.stderr.log"; then
    echo "llm_svg_pipeline: SVG_LLM_CMD failed" >&2
    exit 1
  fi

  if rg -q '<svg[^>]*>' "$raw_file"; then
    start_line="$(rg -n '<svg[^>]*>' "$raw_file" | head -n1 | cut -d: -f1)"
    end_line="$(rg -n '</svg>' "$raw_file" | tail -n1 | cut -d: -f1)"
    if [[ -z "$start_line" || -z "$end_line" || "$end_line" -lt "$start_line" ]]; then
      echo "llm_svg_pipeline: failed to extract svg from LLM output" >&2
      exit 1
    fi
    sed -n "${start_line},${end_line}p" "$raw_file" > "$candidate_file"
    candidate_svg="$candidate_file"
  else
    echo "llm_svg_pipeline: SVG_LLM_CMD output does not contain <svg>" >&2
    exit 1
  fi
else
  if [[ "$dry_run" -eq 1 ]]; then
    cat > "$candidate_file" <<'SVG'
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64">
  <rect x="8" y="8" width="48" height="48" rx="10" fill="#0f62fe"/>
  <path d="M20 32h24" stroke="#ffffff" stroke-width="5" stroke-linecap="round"/>
</svg>
SVG
    candidate_svg="$candidate_file"
  else
    echo "llm_svg_pipeline: SVG_LLM_CMD is required when not --dry-run" >&2
    exit 2
  fi
fi

set +e
cargo run -p nils-image-processing -- svg-validate --in "$candidate_svg" --out "$out_svg" --json >"$validate_json" 2>"$base.validate.stderr.log"
rc=$?
set -e

if [[ $rc -eq 0 ]]; then
  exit 0
fi

"$script_dir/llm_svg_repair_prompt.sh" \
  --intent "$intent" \
  --candidate "$candidate_svg" \
  --diagnostics "$validate_json" \
  --out "$repair_prompt"

echo "llm_svg_pipeline: validation failed, repair prompt emitted: $repair_prompt" >&2
exit $rc
