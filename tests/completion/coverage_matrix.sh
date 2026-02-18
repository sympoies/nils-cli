#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
MATRIX_FILE="$REPO_ROOT/docs/reports/completion-coverage-matrix.md"
ZSH_COMPLETIONS_DIR="$REPO_ROOT/completions/zsh"
BASH_COMPLETIONS_DIR="$REPO_ROOT/completions/bash"
ZSH_ALIASES_FILE="$ZSH_COMPLETIONS_DIR/aliases.zsh"
BASH_ALIASES_FILE="$BASH_COMPLETIONS_DIR/aliases.bash"

fail() {
  printf 'FAIL: %s\n' "$*" >&2
  exit 1
}

has_zsh_compdef_header_token() {
  local zsh_file="$1"
  local token="$2"

  awk -v token="$token" '
/^[[:space:]]*#compdef[[:space:]]+/ {
  for (i = 2; i <= NF; i++) {
    if ($i == token) {
      found = 1
    }
  }
}
END {
  exit(found ? 0 : 1)
}
' "$zsh_file"
}

run_zsh_registration_check() {
  local binary="$1"
  local zsh_file="$2"
  local rc

  if zsh -f -c '
setopt pipe_fail nounset

typeset -ga compdef_calls
compdef() {
  compdef_calls+=("$*")
}

source "$1"

function_name="$2"
command_name="$3"

(( $+functions[$function_name] )) || exit 10

for call in "${compdef_calls[@]}"; do
  if [[ " $call " == *" $function_name "* && " $call " == *" $command_name "* ]]; then
    exit 0
  fi
done

exit 11
' -- "$zsh_file" "_$binary" "$binary"; then
    return 0
  else
    rc=$?
  fi

  case "$rc" in
    10)
      fail "zsh completion function '_$binary' not defined by $zsh_file"
      ;;
    11)
      if has_zsh_compdef_header_token "$zsh_file" "$binary"; then
        return 0
      fi
      fail "zsh completion '_$binary' missing registration for '$binary' (compdef/#compdef) in $zsh_file"
      ;;
    *)
      fail "failed to source zsh completion for '$binary': $zsh_file (exit $rc)"
      ;;
  esac
}

run_zsh_alias_registration_check() {
  local binary="$1"
  local zsh_file="$2"
  local alias_prefix="$3"
  local output
  local rc

  if output="$(zsh -f -c '
setopt pipe_fail nounset

typeset -ga compdef_calls
compdef() {
  compdef_calls+=("$*")
}

source "$1"
source "$2"

function_name="$3"
prefix="$4"
comp_file="$1"

has_compdef_header_token() {
  local target="$1"
  local token="$2"

  awk -v token="$token" "BEGIN { found = 0 }\n\
/^[[:space:]]*#compdef[[:space:]]+/ {\n\
  for (i = 2; i <= NF; i++) {\n\
    if (\\$i == token) {\n\
      found = 1\n\
    }\n\
  }\n\
}\n\
END {\n\
  exit(found ? 0 : 1)\n\
}" "$target"
}

typeset -a family_aliases
family_aliases=()

for alias_name in ${(k)aliases}; do
  [[ "$alias_name" == ${prefix}* ]] && family_aliases+=("$alias_name")
done

(( ${#family_aliases[@]} > 0 )) || exit 20

for alias_name in "${family_aliases[@]}"; do
  integer matched=0
  for call in "${compdef_calls[@]}"; do
    if [[ " $call " == *" $function_name "* && " $call " == *" $alias_name "* ]]; then
      matched=1
      break
    fi
  done

  if (( ! matched )); then
    if ! has_compdef_header_token "$comp_file" "$alias_name"; then
      print -r -- "$alias_name"
      exit 21
    fi
  fi
done
' -- "$zsh_file" "$ZSH_ALIASES_FILE" "_$binary" "$alias_prefix" 2>&1)"; then
    return 0
  fi

  rc=$?
  case "$rc" in
    20)
      fail "no zsh aliases found for required alias prefix '${alias_prefix}*' (binary '$binary')"
      ;;
    21)
      fail "zsh completion '_$binary' missing alias registration for '$output' from aliases.zsh"
      ;;
    *)
      fail "failed zsh alias registration check for '$binary': ${output:-unknown error}"
      ;;
  esac
}

run_bash_registration_check() {
  local binary="$1"
  local bash_file="$2"

  if ! bash -c 'set -euo pipefail; source "$1"; complete -p "$2" >/dev/null' -- "$bash_file" "$binary"; then
    fail "bash completion missing canonical registration for '$binary' in $bash_file"
  fi
}

run_bash_alias_registration_check() {
  local binary="$1"
  local bash_file="$2"
  local alias_prefix="$3"
  local alias_name
  local -a family_aliases=()

  mapfile -t family_aliases < <(
    bash -c 'set -euo pipefail; source "$1"; alias -p' -- "$BASH_ALIASES_FILE" |
      awk -v prefix="$alias_prefix" '
$1 == "alias" {
  name = $2
  sub(/=.*/, "", name)
  if (name ~ ("^" prefix)) {
    print name
  }
}
'
  )

  if [[ ${#family_aliases[@]} -eq 0 ]]; then
    fail "no bash aliases found for required alias prefix '${alias_prefix}*' (binary '$binary')"
  fi

  for alias_name in "${family_aliases[@]}"; do
    if ! bash -c 'set -euo pipefail; source "$1"; complete -p "$2" >/dev/null' -- "$bash_file" "$alias_name"; then
      fail "bash completion for '$binary' missing alias registration for '$alias_name'"
    fi
  done
}

assert_required_no_legacy_contract() {
  local binary="$1"
  local metadata_cell="$2"
  local zsh_file="$3"
  local bash_file="$4"

  [[ "$metadata_cell" == *"completion_mode=clap-first"* ]] || {
    fail "matrix no-legacy metadata missing completion_mode=clap-first for '$binary'"
  }
  [[ "$metadata_cell" == *"legacy_completion_mode_toggles=forbidden"* ]] || {
    fail "matrix no-legacy metadata missing legacy_completion_mode_toggles=forbidden for '$binary'"
  }
  [[ "$metadata_cell" == *"legacy_completion_dispatch=forbidden"* ]] || {
    fail "matrix no-legacy metadata missing legacy_completion_dispatch=forbidden for '$binary'"
  }
  [[ "$metadata_cell" == *"generated_load_failure=fail-closed"* ]] || {
    fail "matrix no-legacy metadata missing generated_load_failure=fail-closed for '$binary'"
  }

  if rg -n --fixed-strings "COMPLETION_MODE" "$zsh_file" "$bash_file" >/dev/null; then
    fail "legacy completion mode toggle detected for '$binary' in completion assets"
  fi

  if rg -n "legacy completion mode" "$zsh_file" "$bash_file" >/dev/null; then
    fail "legacy completion mode wording detected for '$binary' in completion assets"
  fi
}

[[ -f "$MATRIX_FILE" ]] || fail "missing completion coverage matrix: $MATRIX_FILE"
[[ -d "$ZSH_COMPLETIONS_DIR" ]] || fail "missing zsh completions directory: $ZSH_COMPLETIONS_DIR"
[[ -d "$BASH_COMPLETIONS_DIR" ]] || fail "missing bash completions directory: $BASH_COMPLETIONS_DIR"
[[ -f "$ZSH_ALIASES_FILE" ]] || fail "missing zsh aliases file: $ZSH_ALIASES_FILE"
[[ -f "$BASH_ALIASES_FILE" ]] || fail "missing bash aliases file: $BASH_ALIASES_FILE"

declare -a matrix_rows=()
mapfile -t matrix_rows < <(
  awk '
/^[[:space:]]*\| `[^`]+` \|/ {
  split($0, cells, "|")
  if (length(cells) < 7) {
    next
  }

  binary = cells[2]
  obligation = cells[3]
  zsh_cell = cells[4]
  bash_cell = cells[5]
  alias_cell = cells[6]
  metadata_cell = cells[7]

  gsub(/^[[:space:]]+|[[:space:]]+$/, "", binary)
  gsub(/^[[:space:]]+|[[:space:]]+$/, "", obligation)
  gsub(/^[[:space:]]+|[[:space:]]+$/, "", zsh_cell)
  gsub(/^[[:space:]]+|[[:space:]]+$/, "", bash_cell)
  gsub(/^[[:space:]]+|[[:space:]]+$/, "", alias_cell)
  gsub(/^[[:space:]]+|[[:space:]]+$/, "", metadata_cell)

  gsub(/`/, "", binary)
  gsub(/`/, "", obligation)

  alias_required = (alias_cell ~ /`required`/) ? "1" : "0"
  alias_prefix = "__none__"
  if (match(alias_cell, /`[[:alnum:]-]+\*`/)) {
    alias_pattern = substr(alias_cell, RSTART + 1, RLENGTH - 2)
    sub(/\*$/, "", alias_pattern)
    alias_prefix = alias_pattern
  }

  printf "%s\t%s\t%s\t%s\t%s\t%s\t%s\n", binary, obligation, zsh_cell, bash_cell, alias_required, alias_prefix, metadata_cell
}
' "$MATRIX_FILE"
)

if [[ ${#matrix_rows[@]} -eq 0 ]]; then
  fail "no matrix rows found in $MATRIX_FILE"
fi

declare -A seen_binaries=()
declare -A required_alias_prefix_by_binary=()
required_count=0

for row in "${matrix_rows[@]}"; do
  IFS=$'\t' read -r binary obligation zsh_cell bash_cell alias_required alias_prefix metadata_cell <<< "$row"

  [[ -n "$binary" ]] || continue

  if [[ -n "${seen_binaries[$binary]:-}" ]]; then
    fail "duplicate binary row in matrix: $binary"
  fi
  seen_binaries[$binary]=1

  case "$obligation" in
    required)
      required_count=$((required_count + 1))

      [[ "$zsh_cell" == *'`present`'* ]] || fail "matrix marks required CLI '$binary' as zsh '$zsh_cell'"
      [[ "$bash_cell" == *'`present`'* ]] || fail "matrix marks required CLI '$binary' as bash '$bash_cell'"

      zsh_file="$ZSH_COMPLETIONS_DIR/_$binary"
      bash_file="$BASH_COMPLETIONS_DIR/$binary"

      [[ -f "$zsh_file" ]] || fail "required zsh completion file missing for '$binary': $zsh_file"
      [[ -f "$bash_file" ]] || fail "required bash completion file missing for '$binary': $bash_file"

      run_zsh_registration_check "$binary" "$zsh_file"
      run_bash_registration_check "$binary" "$bash_file"
      assert_required_no_legacy_contract "$binary" "$metadata_cell" "$zsh_file" "$bash_file"

      if [[ "$alias_required" == "1" ]]; then
        [[ "$alias_prefix" != "__none__" ]] || fail "matrix alias requirement for '$binary' is missing prefix metadata"
        required_alias_prefix_by_binary["$binary"]="$alias_prefix"
      fi
      ;;
    excluded)
      ;;
    *)
      fail "unsupported obligation '$obligation' for '$binary'"
      ;;
  esac
done

if [[ "$required_count" -eq 0 ]]; then
  fail "matrix produced zero required CLIs"
fi

for binary in "${!required_alias_prefix_by_binary[@]}"; do
  alias_prefix="${required_alias_prefix_by_binary[$binary]}"
  zsh_file="$ZSH_COMPLETIONS_DIR/_$binary"
  bash_file="$BASH_COMPLETIONS_DIR/$binary"

  run_zsh_alias_registration_check "$binary" "$zsh_file" "$alias_prefix"
  run_bash_alias_registration_check "$binary" "$bash_file" "$alias_prefix"
done

printf 'OK\n'
