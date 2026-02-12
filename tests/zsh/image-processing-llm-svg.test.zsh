#!/usr/bin/env -S zsh -f

setopt pipe_fail nounset

SCRIPT_PATH="${0:A}"
REPO_ROOT="${SCRIPT_PATH:h:h:h}"
PIPELINE="$REPO_ROOT/scripts/image-processing/llm_svg_pipeline.sh"

if [[ ! -x "$PIPELINE" ]]; then
  print -u2 -r -- "FAIL: missing pipeline script: $PIPELINE"
  exit 1
fi

tmp_dir="$(mktemp -d)"
trap 'rm -rf "$tmp_dir"' EXIT

valid_out="$tmp_dir/out/sun.svg"
export SVG_LLM_CMD="cat '$REPO_ROOT/crates/image-processing/tests/fixtures/llm-svg-valid.svg'"
"$PIPELINE" --intent "sun icon" --out-svg "$valid_out" --dry-run || {
  print -u2 -r -- "FAIL: pipeline valid case failed"
  exit 1
}

if [[ ! -f "$valid_out" ]]; then
  print -u2 -r -- "FAIL: valid output svg not generated"
  exit 1
fi
if [[ ! -f "${valid_out%.svg}.prompt.md" ]]; then
  print -u2 -r -- "FAIL: prompt artifact missing for valid case"
  exit 1
fi
if [[ -f "${valid_out%.svg}.repair.prompt.md" ]]; then
  print -u2 -r -- "FAIL: repair prompt should not exist for valid case"
  exit 1
fi

invalid_out="$tmp_dir/out/broken.svg"
export SVG_LLM_CMD="cat '$REPO_ROOT/crates/image-processing/tests/fixtures/llm-svg-invalid.svg'"
if "$PIPELINE" --intent "broken icon" --out-svg "$invalid_out" --dry-run; then
  print -u2 -r -- "FAIL: invalid case should fail"
  exit 1
fi
if [[ ! -f "${invalid_out%.svg}.repair.prompt.md" ]]; then
  print -u2 -r -- "FAIL: repair prompt missing for invalid case"
  exit 1
fi
if [[ ! -f "${invalid_out%.svg}.validate.json" ]]; then
  print -u2 -r -- "FAIL: validation diagnostics missing for invalid case"
  exit 1
fi

print -r -- "PASS: image-processing llm svg pipeline"
