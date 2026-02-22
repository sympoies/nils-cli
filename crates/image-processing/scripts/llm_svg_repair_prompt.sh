#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage:
  llm_svg_repair_prompt.sh --intent <text> --candidate <path> --diagnostics <json-path> --out <path>
USAGE
}

intent=""
candidate=""
diagnostics=""
out=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --intent)
      intent="$2"
      shift 2
      ;;
    --candidate)
      candidate="$2"
      shift 2
      ;;
    --diagnostics)
      diagnostics="$2"
      shift 2
      ;;
    --out)
      out="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "llm_svg_repair_prompt: unknown arg: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

if [[ -z "$intent" || -z "$candidate" || -z "$diagnostics" || -z "$out" ]]; then
  echo "llm_svg_repair_prompt: missing required args" >&2
  usage >&2
  exit 2
fi

mkdir -p "$(dirname "$out")"

cat > "$out" <<PROMPT
# Repair Request: SVG Policy Fix

## Intent
$intent

## Candidate SVG Path
$candidate

## Validation Diagnostics JSON Path
$diagnostics

## Instructions
- Return one complete \`<svg ...>...</svg>\` document only.
- Keep the same visual intent.
- Fix all policy violations reported in diagnostics.
- Do not include markdown fences or explanations.
PROMPT
