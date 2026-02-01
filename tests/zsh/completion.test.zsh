#!/usr/bin/env -S zsh -f

setopt pipe_fail nounset

SCRIPT_PATH="${0:A}"
REPO_ROOT="${SCRIPT_PATH:h:h:h}"
COMP_FILE="$REPO_ROOT/completions/zsh/_git-scope"
COMP_SUMMARY_FILE="$REPO_ROOT/completions/zsh/_git-summary"
COMP_LOCK_FILE="$REPO_ROOT/completions/zsh/_git-lock"
COMP_FZF_CLI_FILE="$REPO_ROOT/completions/zsh/_fzf-cli"
COMP_SEMANTIC_COMMIT_FILE="$REPO_ROOT/completions/zsh/_semantic-commit"
COMP_API_REST_FILE="$REPO_ROOT/completions/zsh/_api-rest"
COMP_API_GQL_FILE="$REPO_ROOT/completions/zsh/_api-gql"
COMP_API_TEST_FILE="$REPO_ROOT/completions/zsh/_api-test"
COMP_PLAN_TOOLING_FILE="$REPO_ROOT/completions/zsh/_plan-tooling"
COMP_CODEX_CLI_FILE="$REPO_ROOT/completions/zsh/_codex-cli"

if [[ ! -f "$COMP_FILE" ]]; then
  print -u2 -r -- "FAIL: missing completion file"
  exit 1
fi

if [[ ! -f "$COMP_SUMMARY_FILE" ]]; then
  print -u2 -r -- "FAIL: missing git-summary completion file"
  exit 1
fi

if [[ ! -f "$COMP_LOCK_FILE" ]]; then
  print -u2 -r -- "FAIL: missing git-lock completion file"
  exit 1
fi

if [[ ! -f "$COMP_FZF_CLI_FILE" ]]; then
  print -u2 -r -- "FAIL: missing fzf-cli completion file"
  exit 1
fi

if [[ ! -f "$COMP_SEMANTIC_COMMIT_FILE" ]]; then
  print -u2 -r -- "FAIL: missing semantic-commit completion file"
  exit 1
fi

if [[ ! -f "$COMP_API_REST_FILE" ]]; then
  print -u2 -r -- "FAIL: missing api-rest completion file"
  exit 1
fi

if [[ ! -f "$COMP_API_GQL_FILE" ]]; then
  print -u2 -r -- "FAIL: missing api-gql completion file"
  exit 1
fi

if [[ ! -f "$COMP_API_TEST_FILE" ]]; then
  print -u2 -r -- "FAIL: missing api-test completion file"
  exit 1
fi

if [[ ! -f "$COMP_PLAN_TOOLING_FILE" ]]; then
  print -u2 -r -- "FAIL: missing plan-tooling completion file"
  exit 1
fi

if [[ ! -f "$COMP_CODEX_CLI_FILE" ]]; then
  print -u2 -r -- "FAIL: missing codex-cli completion file"
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

source "$COMP_LOCK_FILE" || {
  print -u2 -r -- "FAIL: failed to source git-lock completion file"
  exit 1
}

source "$COMP_FZF_CLI_FILE" || {
  print -u2 -r -- "FAIL: failed to source fzf-cli completion file"
  exit 1
}

source "$COMP_SEMANTIC_COMMIT_FILE" || {
  print -u2 -r -- "FAIL: failed to source semantic-commit completion file"
  exit 1
}

source "$COMP_API_REST_FILE" || {
  print -u2 -r -- "FAIL: failed to source api-rest completion file"
  exit 1
}

source "$COMP_API_GQL_FILE" || {
  print -u2 -r -- "FAIL: failed to source api-gql completion file"
  exit 1
}

source "$COMP_API_TEST_FILE" || {
  print -u2 -r -- "FAIL: failed to source api-test completion file"
  exit 1
}

source "$COMP_PLAN_TOOLING_FILE" || {
  print -u2 -r -- "FAIL: failed to source plan-tooling completion file"
  exit 1
}

source "$COMP_CODEX_CLI_FILE" || {
  print -u2 -r -- "FAIL: failed to source codex-cli completion file"
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

if (( ! $+functions[_git-lock] )); then
  print -u2 -r -- "FAIL: _git-lock function not defined"
  exit 1
fi

if (( ! $+functions[_fzf-cli] )); then
  print -u2 -r -- "FAIL: _fzf-cli function not defined"
  exit 1
fi

if (( ! $+functions[_semantic-commit] )); then
  print -u2 -r -- "FAIL: _semantic-commit function not defined"
  exit 1
fi

if (( ! $+functions[_api-rest] )); then
  print -u2 -r -- "FAIL: _api-rest function not defined"
  exit 1
fi

if (( ! $+functions[_api-gql] )); then
  print -u2 -r -- "FAIL: _api-gql function not defined"
  exit 1
fi

if (( ! $+functions[_api-test] )); then
  print -u2 -r -- "FAIL: _api-test function not defined"
  exit 1
fi

if (( ! $+functions[_plan-tooling] )); then
  print -u2 -r -- "FAIL: _plan-tooling function not defined"
  exit 1
fi

if (( ! $+functions[_codex-cli] )); then
  print -u2 -r -- "FAIL: _codex-cli function not defined"
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

grep -q "lock:Save commit hash to lock" "$COMP_LOCK_FILE" || {
  print -u2 -r -- "FAIL: git-lock completion missing lock command"
  exit 1
}

grep -q "diff:Compare commits between two locks" "$COMP_LOCK_FILE" || {
  print -u2 -r -- "FAIL: git-lock completion missing diff command"
  exit 1
}

grep -q "file:Search and preview text files" "$COMP_FZF_CLI_FILE" || {
  print -u2 -r -- "FAIL: fzf-cli completion missing file command"
  exit 1
}

grep -q "staged-context:Print staged change context" "$COMP_SEMANTIC_COMMIT_FILE" || {
  print -u2 -r -- "FAIL: semantic-commit completion missing staged-context command"
  exit 1
}

grep -q "commit:Commit staged changes" "$COMP_SEMANTIC_COMMIT_FILE" || {
  print -u2 -r -- "FAIL: semantic-commit completion missing commit command"
  exit 1
}

grep -q "call:Execute a request file" "$COMP_API_REST_FILE" || {
  print -u2 -r -- "FAIL: api-rest completion missing call command"
  exit 1
}

grep -q "schema:Resolve a schema file path" "$COMP_API_GQL_FILE" || {
  print -u2 -r -- "FAIL: api-gql completion missing schema command"
  exit 1
}

grep -q "summary:Render a Markdown summary" "$COMP_API_TEST_FILE" || {
  print -u2 -r -- "FAIL: api-test completion missing summary command"
  exit 1
}

grep -q "to-json:Parse a plan markdown file" "$COMP_PLAN_TOOLING_FILE" || {
  print -u2 -r -- "FAIL: plan-tooling completion missing to-json command"
  exit 1
}

grep -q "agent:Prompts and skill wrappers" "$COMP_CODEX_CLI_FILE" || {
  print -u2 -r -- "FAIL: codex-cli completion missing agent command"
  exit 1
}

grep -q "auth:Auth and secrets" "$COMP_CODEX_CLI_FILE" || {
  print -u2 -r -- "FAIL: codex-cli completion missing auth command"
  exit 1
}

grep -q "diag:Diagnostics" "$COMP_CODEX_CLI_FILE" || {
  print -u2 -r -- "FAIL: codex-cli completion missing diag command"
  exit 1
}

grep -q -- "--all\\[" "$COMP_CODEX_CLI_FILE" || {
  print -u2 -r -- "FAIL: codex-cli completion missing --all"
  exit 1
}

grep -q -- "--async\\[" "$COMP_CODEX_CLI_FILE" || {
  print -u2 -r -- "FAIL: codex-cli completion missing --async"
  exit 1
}

grep -q -- "--cached\\[" "$COMP_CODEX_CLI_FILE" || {
  print -u2 -r -- "FAIL: codex-cli completion missing --cached"
  exit 1
}

print -r -- "OK"
