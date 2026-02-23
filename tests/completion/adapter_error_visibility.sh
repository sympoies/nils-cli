#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
ZSH_HELPER="$REPO_ROOT/completions/zsh/_completion-adapter-common.zsh"
BASH_HELPER="$REPO_ROOT/completions/bash/completion-adapter-common.bash"

fail() {
  printf 'FAIL: %s\n' "$*" >&2
  exit 1
}

assert_contains() {
  local haystack="$1"
  local needle="$2"
  local context="$3"
  if [[ "$haystack" != *"$needle"* ]]; then
    fail "$context missing '$needle'"
  fi
}

assert_file_line_count() {
  local file="$1"
  local expected="$2"
  local actual=0
  if [[ -f "$file" ]]; then
    actual="$(wc -l < "$file" | tr -d '[:space:]')"
  fi
  [[ "$actual" == "$expected" ]] || fail "expected $expected calls in $file, got $actual"
}

[[ -f "$ZSH_HELPER" ]] || fail "missing zsh helper: $ZSH_HELPER"
[[ -f "$BASH_HELPER" ]] || fail "missing bash helper: $BASH_HELPER"
command -v zsh >/dev/null 2>&1 || fail "zsh not found"

TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

cat >"$TMP_DIR/fake-zsh-cli" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "${FAKE_CLI_CALL_MARKER:-call}" >> "${FAKE_CLI_COUNT_FILE:?}"
printf 'fake-zsh-cli simulated failure (%s %s)\n' "${1-}" "${2-}" >&2
exit 23
EOF
chmod +x "$TMP_DIR/fake-zsh-cli"

cat >"$TMP_DIR/fake-bash-cli" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "${FAKE_CLI_CALL_MARKER:-call}" >> "${FAKE_CLI_COUNT_FILE:?}"
printf 'fake-bash-cli simulated failure (%s %s)\n' "${1-}" "${2-}" >&2
exit 29
EOF
chmod +x "$TMP_DIR/fake-bash-cli"

ZSH_COUNT_FILE="$TMP_DIR/zsh.count"
ZSH_STDOUT_FILE="$TMP_DIR/zsh.stdout"
ZSH_STDERR_FILE="$TMP_DIR/zsh.stderr"

FAKE_CLI_COUNT_FILE="$ZSH_COUNT_FILE" PATH="$TMP_DIR:$PATH" \
  zsh -f -c '
source "$1"
typeset -gi ZSH_STATE=0
_nils_cli_completion_common_load_generated_zsh \
  "ZSH_STATE" "_nils_test_generated_zsh" "fake-zsh-cli" "_fake-zsh-cli" "" ""
first_rc=$?
_nils_cli_completion_common_load_generated_zsh \
  "ZSH_STATE" "_nils_test_generated_zsh" "fake-zsh-cli" "_fake-zsh-cli" "" ""
second_rc=$?
print -r -- "state=${ZSH_STATE} first_rc=${first_rc} second_rc=${second_rc}"
' -- "$ZSH_HELPER" >"$ZSH_STDOUT_FILE" 2>"$ZSH_STDERR_FILE" || true

ZSH_STDOUT="$(cat "$ZSH_STDOUT_FILE")"
ZSH_STDERR="$(cat "$ZSH_STDERR_FILE")"
assert_contains "$ZSH_STDOUT" "state=-1" "zsh state"
assert_contains "$ZSH_STDOUT" "first_rc=1" "zsh first rc"
assert_contains "$ZSH_STDOUT" "second_rc=1" "zsh second rc"
assert_contains "$ZSH_STDERR" "fake-zsh-cli simulated failure (completion zsh)" "zsh stderr passthrough"
assert_contains "$ZSH_STDERR" "nils-cli completion (zsh): generated completion load failed for 'fake-zsh-cli'" "zsh diagnostic"
assert_contains "$ZSH_STDERR" "fail-closed mode active" "zsh fail-closed message"
assert_file_line_count "$ZSH_COUNT_FILE" "1"

BASH_COUNT_FILE="$TMP_DIR/bash.count"
BASH_STDOUT_FILE="$TMP_DIR/bash.stdout"
BASH_STDERR_FILE="$TMP_DIR/bash.stderr"

FAKE_CLI_COUNT_FILE="$BASH_COUNT_FILE" PATH="$TMP_DIR:$PATH" \
  bash -c '
source "$1"
BASH_STATE=0
_nils_cli_completion_common_load_generated_bash \
  "BASH_STATE" "_nils_test_generated_bash" "fake-bash-cli" "_fake-bash-cli" "" ""
first_rc=$?
_nils_cli_completion_common_load_generated_bash \
  "BASH_STATE" "_nils_test_generated_bash" "fake-bash-cli" "_fake-bash-cli" "" ""
second_rc=$?
printf "state=%s first_rc=%s second_rc=%s compreply_len=%s\n" \
  "$BASH_STATE" "$first_rc" "$second_rc" "${#COMPREPLY[@]}"
' -- "$BASH_HELPER" >"$BASH_STDOUT_FILE" 2>"$BASH_STDERR_FILE" || true

BASH_STDOUT="$(cat "$BASH_STDOUT_FILE")"
BASH_STDERR="$(cat "$BASH_STDERR_FILE")"
assert_contains "$BASH_STDOUT" "state=-1" "bash state"
assert_contains "$BASH_STDOUT" "first_rc=1" "bash first rc"
assert_contains "$BASH_STDOUT" "second_rc=1" "bash second rc"
assert_contains "$BASH_STDOUT" "compreply_len=0" "bash fail-closed COMPREPLY"
assert_contains "$BASH_STDERR" "fake-bash-cli simulated failure (completion bash)" "bash stderr passthrough"
assert_contains "$BASH_STDERR" "nils-cli completion (bash): generated completion load failed for 'fake-bash-cli'" "bash diagnostic"
assert_contains "$BASH_STDERR" "fail-closed mode active" "bash fail-closed message"
assert_file_line_count "$BASH_COUNT_FILE" "1"

printf 'OK\n'
