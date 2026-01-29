#!/usr/bin/env -S zsh -f

setopt pipe_fail nounset

SCRIPT_PATH="${0:A}"
REPO_ROOT="${SCRIPT_PATH:h:h:h}"
COMP_FILE="$REPO_ROOT/completions/zsh/_git-scope"
COMP_SUMMARY_FILE="$REPO_ROOT/completions/zsh/_git-summary"

if [[ ! -f "$COMP_FILE" ]]; then
  print -u2 -r -- "FAIL: missing completion file"
  exit 1
fi

if [[ ! -f "$COMP_SUMMARY_FILE" ]]; then
  print -u2 -r -- "FAIL: missing git-summary completion file"
  exit 1
fi

# Avoid compinit in CI (non-interactive shells); stub compdef so sourcing works.
compdef() { :; }

source "$COMP_FILE" || {
  print -u2 -r -- "FAIL: failed to source completion file"
  exit 1
}

source "$COMP_SUMMARY_FILE" || {
  print -u2 -r -- "FAIL: failed to source git-summary completion file"
  exit 1
}

if (( ! $+functions[_git-scope] )); then
  print -u2 -r -- "FAIL: _git-scope function not defined"
  exit 1
fi

if (( ! $+functions[_git-summary] )); then
  print -u2 -r -- "FAIL: _git-summary function not defined"
  exit 1
fi

grep -q "tracked:Show tracked files" "$COMP_FILE" || {
  print -u2 -r -- "FAIL: tracked subcommand not defined in completion"
  exit 1
}

grep -q "commit:Inspect a commit" "$COMP_FILE" || {
  print -u2 -r -- "FAIL: commit subcommand not defined in completion"
  exit 1
}

grep -q "git-summary command" "$COMP_SUMMARY_FILE" || {
  print -u2 -r -- "FAIL: git-summary completion missing command list"
  exit 1
}

grep -q "this-week" "$COMP_SUMMARY_FILE" || {
  print -u2 -r -- "FAIL: git-summary completion missing this-week"
  exit 1
}

print -r -- "OK"
