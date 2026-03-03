#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage:
  scripts/ci/completion-asset-audit.sh [--strict]

Checks completion-asset matrix coverage for workspace binaries:
  - required binaries must have both zsh/bash completion assets present
  - every workspace binary must be represented in the matrix
  - matrix rows must align with workspace binaries and policy fields

Options:
  --strict   Treat policy warnings as hard failures
  -h, --help Show this help
USAGE
}

strict=0
while [[ $# -gt 0 ]]; do
  case "${1:-}" in
    --strict)
      strict=1
      shift
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

repo_root="$(git rev-parse --show-toplevel 2>/dev/null || true)"
if [[ -z "$repo_root" || ! -d "$repo_root" ]]; then
  echo "error: must run inside a git work tree" >&2
  exit 2
fi
cd "$repo_root"

matrix_path="docs/specs/completion-coverage-matrix-v1.md"
workspace_bins_script="scripts/workspace-bins.sh"

if [[ ! -f "$matrix_path" ]]; then
  echo "error: missing matrix file: $matrix_path" >&2
  exit 2
fi

if [[ ! -f "$workspace_bins_script" ]]; then
  echo "error: missing workspace inventory script: $workspace_bins_script" >&2
  exit 2
fi

declare -a errors=()
declare -a warnings=()
declare -A matrix_obligation=()
declare -A matrix_zsh_cell=()
declare -A matrix_bash_cell=()
declare -A workspace_set=()

escalate_or_warn() {
  local message="$1"
  if [[ $strict -eq 1 ]]; then
    errors+=("$message")
  else
    warnings+=("$message")
  fi
}

matrix_rows=0
while IFS=$'\t' read -r bin obligation zsh_cell bash_cell; do
  matrix_rows=$((matrix_rows + 1))
  if [[ -n "${matrix_obligation[$bin]+x}" ]]; then
    errors+=("duplicate matrix row for binary: $bin")
    continue
  fi
  matrix_obligation["$bin"]="$obligation"
  matrix_zsh_cell["$bin"]="$zsh_cell"
  matrix_bash_cell["$bin"]="$bash_cell"
done < <(
  awk -F'|' '
    function trim(s) {
      gsub(/^[ \t]+|[ \t]+$/, "", s)
      return s
    }
    {
      bin = trim($2)
      obligation = trim($3)
      zsh = trim($4)
      bash = trim($5)
      if (bin ~ /^`[^`]+`$/ && obligation ~ /^`(required|excluded)`$/) {
        gsub(/`/, "", bin)
        gsub(/`/, "", obligation)
        print bin "\t" obligation "\t" zsh "\t" bash
      }
    }
  ' "$matrix_path"
)

if [[ $matrix_rows -eq 0 ]]; then
  echo "error: no matrix rows found in $matrix_path" >&2
  exit 2
fi

mapfile -t workspace_bins < <(bash "$workspace_bins_script")
if [[ ${#workspace_bins[@]} -eq 0 ]]; then
  echo "error: no workspace binaries found via $workspace_bins_script" >&2
  exit 2
fi

for bin in "${workspace_bins[@]}"; do
  workspace_set["$bin"]=1
done

for bin in "${workspace_bins[@]}"; do
  if [[ -z "${matrix_obligation[$bin]+x}" ]]; then
    errors+=("undeclared exclusion: workspace binary missing from matrix: $bin")
  fi
done

mapfile -t matrix_bins_sorted < <(printf '%s\n' "${!matrix_obligation[@]}" | sort)
for bin in "${matrix_bins_sorted[@]}"; do
  if [[ -z "${workspace_set[$bin]+x}" ]]; then
    errors+=("matrix drift: matrix row does not match workspace inventory: $bin")
  fi
done

required_count=0
excluded_count=0
for bin in "${matrix_bins_sorted[@]}"; do
  if [[ -z "${workspace_set[$bin]+x}" ]]; then
    continue
  fi

  obligation="${matrix_obligation[$bin]}"
  zsh_cell="${matrix_zsh_cell[$bin]}"
  bash_cell="${matrix_bash_cell[$bin]}"

  case "$obligation" in
    required)
      required_count=$((required_count + 1))
      expected_zsh="\`present\` (\`_${bin}\`)"
      expected_bash="\`present\` (\`${bin}\`)"
      if [[ "$zsh_cell" != "$expected_zsh" ]]; then
        errors+=("matrix drift: required binary $bin zsh column expected '$expected_zsh' but found '$zsh_cell'")
      fi
      if [[ "$bash_cell" != "$expected_bash" ]]; then
        errors+=("matrix drift: required binary $bin bash column expected '$expected_bash' but found '$bash_cell'")
      fi
      if [[ ! -f "completions/zsh/_$bin" ]]; then
        errors+=("missing completion asset: completions/zsh/_$bin (required binary: $bin)")
      fi
      if [[ ! -f "completions/bash/$bin" ]]; then
        errors+=("missing completion asset: completions/bash/$bin (required binary: $bin)")
      fi
      ;;
    excluded)
      excluded_count=$((excluded_count + 1))
      if [[ "$zsh_cell" != "\`missing\`" ]]; then
        errors+=("matrix drift: excluded binary $bin zsh column expected '\`missing\`' but found '$zsh_cell'")
      fi
      if [[ "$bash_cell" != "\`missing\`" ]]; then
        errors+=("matrix drift: excluded binary $bin bash column expected '\`missing\`' but found '$bash_cell'")
      fi
      if [[ -f "completions/zsh/_$bin" ]]; then
        escalate_or_warn "excluded binary has zsh completion asset: completions/zsh/_$bin"
      fi
      if [[ -f "completions/bash/$bin" ]]; then
        escalate_or_warn "excluded binary has bash completion asset: completions/bash/$bin"
      fi
      ;;
    *)
      errors+=("matrix drift: unsupported obligation '$obligation' for binary $bin")
      ;;
  esac
done

for warn in "${warnings[@]}"; do
  echo "WARN: $warn"
done

if [[ ${#errors[@]} -gt 0 ]]; then
  for err in "${errors[@]}"; do
    echo "FAIL: $err"
  done
  echo "FAIL: completion asset audit (strict=$strict, workspace_bins=${#workspace_bins[@]}, matrix_rows=$matrix_rows, required=$required_count, excluded=$excluded_count, errors=${#errors[@]}, warnings=${#warnings[@]})"
  exit 1
fi

echo "PASS: completion asset audit (strict=$strict, workspace_bins=${#workspace_bins[@]}, matrix_rows=$matrix_rows, required=$required_count, excluded=$excluded_count, warnings=${#warnings[@]})"
