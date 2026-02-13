#!/usr/bin/env -S zsh -f

setopt pipe_fail nounset

SCRIPT_PATH="${0:A}"
REPO_ROOT="${SCRIPT_PATH:h:h:h}"
COMP_FILE="$REPO_ROOT/completions/zsh/_git-scope"
COMP_SUMMARY_FILE="$REPO_ROOT/completions/zsh/_git-summary"
COMP_LOCK_FILE="$REPO_ROOT/completions/zsh/_git-lock"
COMP_GIT_CLI_FILE="$REPO_ROOT/completions/zsh/_git-cli"
COMP_FZF_CLI_FILE="$REPO_ROOT/completions/zsh/_fzf-cli"
COMP_SEMANTIC_COMMIT_FILE="$REPO_ROOT/completions/zsh/_semantic-commit"
COMP_API_REST_FILE="$REPO_ROOT/completions/zsh/_api-rest"
COMP_API_GQL_FILE="$REPO_ROOT/completions/zsh/_api-gql"
COMP_API_TEST_FILE="$REPO_ROOT/completions/zsh/_api-test"
COMP_PLAN_TOOLING_FILE="$REPO_ROOT/completions/zsh/_plan-tooling"
COMP_CODEX_CLI_FILE="$REPO_ROOT/completions/zsh/_codex-cli"
COMP_AGENT_DOCS_FILE="$REPO_ROOT/completions/zsh/_agent-docs"
COMP_MACOS_AGENT_FILE="$REPO_ROOT/completions/zsh/_macos-agent"
COMP_AGENTCTL_FILE="$REPO_ROOT/completions/zsh/_agentctl"
COMP_MEMO_CLI_FILE="$REPO_ROOT/completions/zsh/_memo-cli"
ALIASES_FILE="$REPO_ROOT/completions/zsh/aliases.zsh"

BASH_GIT_SCOPE_FILE="$REPO_ROOT/completions/bash/git-scope"
BASH_SUMMARY_FILE="$REPO_ROOT/completions/bash/git-summary"
BASH_LOCK_FILE="$REPO_ROOT/completions/bash/git-lock"
BASH_GIT_CLI_FILE="$REPO_ROOT/completions/bash/git-cli"
BASH_FZF_CLI_FILE="$REPO_ROOT/completions/bash/fzf-cli"
BASH_SEMANTIC_COMMIT_FILE="$REPO_ROOT/completions/bash/semantic-commit"
BASH_API_REST_FILE="$REPO_ROOT/completions/bash/api-rest"
BASH_API_GQL_FILE="$REPO_ROOT/completions/bash/api-gql"
BASH_API_TEST_FILE="$REPO_ROOT/completions/bash/api-test"
BASH_PLAN_TOOLING_FILE="$REPO_ROOT/completions/bash/plan-tooling"
BASH_CODEX_CLI_FILE="$REPO_ROOT/completions/bash/codex-cli"
BASH_AGENT_DOCS_FILE="$REPO_ROOT/completions/bash/agent-docs"
BASH_MACOS_AGENT_FILE="$REPO_ROOT/completions/bash/macos-agent"
BASH_AGENTCTL_FILE="$REPO_ROOT/completions/bash/agentctl"
BASH_MEMO_CLI_FILE="$REPO_ROOT/completions/bash/memo-cli"
BASH_ALIASES_FILE="$REPO_ROOT/completions/bash/aliases.bash"

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

if [[ ! -f "$COMP_GIT_CLI_FILE" ]]; then
  print -u2 -r -- "FAIL: missing git-cli completion file"
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

if [[ ! -f "$COMP_AGENT_DOCS_FILE" ]]; then
  print -u2 -r -- "FAIL: missing agent-docs completion file"
  exit 1
fi

if [[ ! -f "$COMP_MACOS_AGENT_FILE" ]]; then
  print -u2 -r -- "FAIL: missing macos-agent completion file"
  exit 1
fi

if [[ ! -f "$COMP_AGENTCTL_FILE" ]]; then
  print -u2 -r -- "FAIL: missing agentctl completion file"
  exit 1
fi

if [[ ! -f "$COMP_MEMO_CLI_FILE" ]]; then
  print -u2 -r -- "FAIL: missing memo-cli completion file"
  exit 1
fi

if [[ ! -f "$ALIASES_FILE" ]]; then
  print -u2 -r -- "FAIL: missing nils-cli aliases file"
  exit 1
fi

if [[ ! -f "$BASH_GIT_SCOPE_FILE" ]]; then
  print -u2 -r -- "FAIL: missing bash git-scope completion file"
  exit 1
fi

if [[ ! -f "$BASH_SUMMARY_FILE" ]]; then
  print -u2 -r -- "FAIL: missing bash git-summary completion file"
  exit 1
fi

if [[ ! -f "$BASH_LOCK_FILE" ]]; then
  print -u2 -r -- "FAIL: missing bash git-lock completion file"
  exit 1
fi

if [[ ! -f "$BASH_GIT_CLI_FILE" ]]; then
  print -u2 -r -- "FAIL: missing bash git-cli completion file"
  exit 1
fi

if [[ ! -f "$BASH_FZF_CLI_FILE" ]]; then
  print -u2 -r -- "FAIL: missing bash fzf-cli completion file"
  exit 1
fi

if [[ ! -f "$BASH_SEMANTIC_COMMIT_FILE" ]]; then
  print -u2 -r -- "FAIL: missing bash semantic-commit completion file"
  exit 1
fi

if [[ ! -f "$BASH_API_REST_FILE" ]]; then
  print -u2 -r -- "FAIL: missing bash api-rest completion file"
  exit 1
fi

if [[ ! -f "$BASH_API_GQL_FILE" ]]; then
  print -u2 -r -- "FAIL: missing bash api-gql completion file"
  exit 1
fi

if [[ ! -f "$BASH_API_TEST_FILE" ]]; then
  print -u2 -r -- "FAIL: missing bash api-test completion file"
  exit 1
fi

if [[ ! -f "$BASH_PLAN_TOOLING_FILE" ]]; then
  print -u2 -r -- "FAIL: missing bash plan-tooling completion file"
  exit 1
fi

if [[ ! -f "$BASH_CODEX_CLI_FILE" ]]; then
  print -u2 -r -- "FAIL: missing bash codex-cli completion file"
  exit 1
fi

if [[ ! -f "$BASH_AGENT_DOCS_FILE" ]]; then
  print -u2 -r -- "FAIL: missing bash agent-docs completion file"
  exit 1
fi

if [[ ! -f "$BASH_MACOS_AGENT_FILE" ]]; then
  print -u2 -r -- "FAIL: missing bash macos-agent completion file"
  exit 1
fi

if [[ ! -f "$BASH_AGENTCTL_FILE" ]]; then
  print -u2 -r -- "FAIL: missing bash agentctl completion file"
  exit 1
fi

if [[ ! -f "$BASH_MEMO_CLI_FILE" ]]; then
  print -u2 -r -- "FAIL: missing bash memo-cli completion file"
  exit 1
fi

if [[ ! -f "$BASH_ALIASES_FILE" ]]; then
  print -u2 -r -- "FAIL: missing bash nils-cli aliases file"
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

source "$COMP_GIT_CLI_FILE" || {
  print -u2 -r -- "FAIL: failed to source git-cli completion file"
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

source "$COMP_AGENT_DOCS_FILE" || {
  print -u2 -r -- "FAIL: failed to source agent-docs completion file"
  exit 1
}

source "$COMP_MACOS_AGENT_FILE" || {
  print -u2 -r -- "FAIL: failed to source macos-agent completion file"
  exit 1
}

source "$COMP_AGENTCTL_FILE" || {
  print -u2 -r -- "FAIL: failed to source agentctl completion file"
  exit 1
}

source "$COMP_MEMO_CLI_FILE" || {
  print -u2 -r -- "FAIL: failed to source memo-cli completion file"
  exit 1
}

source "$ALIASES_FILE" || {
  print -u2 -r -- "FAIL: failed to source nils-cli aliases file"
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

if (( ! $+functions[_git-cli] )); then
  print -u2 -r -- "FAIL: _git-cli function not defined"
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

if (( ! $+functions[_agent-docs] )); then
  print -u2 -r -- "FAIL: _agent-docs function not defined"
  exit 1
fi

if (( ! $+functions[_macos-agent] )); then
  print -u2 -r -- "FAIL: _macos-agent function not defined"
  exit 1
fi

if (( ! $+functions[_agentctl] )); then
  print -u2 -r -- "FAIL: _agentctl function not defined"
  exit 1
fi

if (( ! $+functions[_memo-cli] )); then
  print -u2 -r -- "FAIL: _memo-cli function not defined"
  exit 1
fi

if (( ! $+aliases[gs] )); then
  print -u2 -r -- "FAIL: gs alias not defined"
  exit 1
fi

if (( ! $+aliases[gst] )); then
  print -u2 -r -- "FAIL: gst alias not defined"
  exit 1
fi

if (( ! $+aliases[gss] )); then
  print -u2 -r -- "FAIL: gss alias not defined"
  exit 1
fi

if (( ! $+aliases[gsu] )); then
  print -u2 -r -- "FAIL: gsu alias not defined"
  exit 1
fi

if (( ! $+aliases[gsa] )); then
  print -u2 -r -- "FAIL: gsa alias not defined"
  exit 1
fi

if (( ! $+aliases[gsun] )); then
  print -u2 -r -- "FAIL: gsun alias not defined"
  exit 1
fi

if (( ! $+aliases[gsc] )); then
  print -u2 -r -- "FAIL: gsc alias not defined"
  exit 1
fi

if (( ! $+aliases[gsh] )); then
  print -u2 -r -- "FAIL: gsh alias not defined"
  exit 1
fi

if (( ! $+aliases[gx] )); then
  print -u2 -r -- "FAIL: gx alias not defined"
  exit 1
fi

if (( ! $+aliases[gxh] )); then
  print -u2 -r -- "FAIL: gxh alias not defined"
  exit 1
fi

if (( ! $+aliases[gxu] )); then
  print -u2 -r -- "FAIL: gxu alias not defined"
  exit 1
fi

if (( ! $+aliases[gxrs] )); then
  print -u2 -r -- "FAIL: gxrs alias not defined"
  exit 1
fi

if (( ! $+aliases[gxrr] )); then
  print -u2 -r -- "FAIL: gxrr alias not defined"
  exit 1
fi

if (( ! $+aliases[gxcc] )); then
  print -u2 -r -- "FAIL: gxcc alias not defined"
  exit 1
fi

if (( ! $+aliases[gxip] )); then
  print -u2 -r -- "FAIL: gxip alias not defined"
  exit 1
fi

if (( ! $+functions[gxur] )); then
  print -u2 -r -- "FAIL: gxur function not defined"
  exit 1
fi

if (( ! $+aliases[cx] )); then
  print -u2 -r -- "FAIL: cx alias not defined"
  exit 1
fi

if (( ! $+aliases[cxau] )); then
  print -u2 -r -- "FAIL: cxau alias not defined"
  exit 1
fi

if (( ! $+aliases[cxst] )); then
  print -u2 -r -- "FAIL: cxst alias not defined"
  exit 1
fi

if (( ! $+aliases[cxdr] )); then
  print -u2 -r -- "FAIL: cxdr alias not defined"
  exit 1
fi

if (( ! $+aliases[cxdra] )); then
  print -u2 -r -- "FAIL: cxdra alias not defined"
  exit 1
fi

if (( ! $+aliases[fx] )); then
  print -u2 -r -- "FAIL: fx alias not defined"
  exit 1
fi

if (( ! $+aliases[fxf] )); then
  print -u2 -r -- "FAIL: fxf alias not defined"
  exit 1
fi

if (( ! $+aliases[fxgs] )); then
  print -u2 -r -- "FAIL: fxgs alias not defined"
  exit 1
fi

if (( ! $+aliases[fxdef] )); then
  print -u2 -r -- "FAIL: fxdef alias not defined"
  exit 1
fi

if (( ! $+functions[fxd] )); then
  print -u2 -r -- "FAIL: fxd function not defined"
  exit 1
fi

if (( ! $+functions[fxh] )); then
  print -u2 -r -- "FAIL: fxh function not defined"
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

grep -q "login:Login with ChatGPT" "$COMP_CODEX_CLI_FILE" || {
  print -u2 -r -- "FAIL: codex-cli completion missing auth login command"
  exit 1
}

grep -q "save:Save CODEX_AUTH_FILE" "$COMP_CODEX_CLI_FILE" || {
  print -u2 -r -- "FAIL: codex-cli completion missing auth save command"
  exit 1
}

grep -q -- "--api-key\\[" "$COMP_CODEX_CLI_FILE" || {
  print -u2 -r -- "FAIL: codex-cli completion missing --api-key"
  exit 1
}

grep -q -- "--device-code\\[" "$COMP_CODEX_CLI_FILE" || {
  print -u2 -r -- "FAIL: codex-cli completion missing --device-code"
  exit 1
}

grep -q -- "--yes\\[" "$COMP_CODEX_CLI_FILE" || {
  print -u2 -r -- "FAIL: codex-cli completion missing --yes"
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

grep -q "resolve:Resolve required docs for a context" "$COMP_AGENT_DOCS_FILE" || {
  print -u2 -r -- "FAIL: agent-docs completion missing resolve command"
  exit 1
}

grep -q "scaffold-baseline:Scaffold missing baseline docs" "$COMP_AGENT_DOCS_FILE" || {
  print -u2 -r -- "FAIL: agent-docs completion missing scaffold-baseline command"
  exit 1
}

grep -q -- "--strict\\[" "$COMP_AGENT_DOCS_FILE" || {
  print -u2 -r -- "FAIL: agent-docs completion missing --strict"
  exit 1
}

grep -q -- "--missing-only\\[" "$COMP_AGENT_DOCS_FILE" || {
  print -u2 -r -- "FAIL: agent-docs completion missing --missing-only"
  exit 1
}

if [[ "$(grep -c -- "format:(text json checklist)" "$COMP_AGENT_DOCS_FILE")" -ne 1 ]]; then
  print -u2 -r -- "FAIL: agent-docs resolve completion missing checklist format"
  exit 1
fi

if [[ "$(grep -c -- "format:(text json)" "$COMP_AGENT_DOCS_FILE")" -ne 3 ]]; then
  print -u2 -r -- "FAIL: non-resolve agent-docs format completions must remain text|json"
  exit 1
fi

grep -q "local -a resolve_formats=(text json checklist)" "$BASH_AGENT_DOCS_FILE" || {
  print -u2 -r -- "FAIL: bash agent-docs resolve format completion missing checklist"
  exit 1
}

grep -q "local -a formats=(text json)" "$BASH_AGENT_DOCS_FILE" || {
  print -u2 -r -- "FAIL: bash agent-docs non-resolve formats must remain text|json"
  exit 1
}

grep -q -- "resolve_formats\\[\\*\\]" "$BASH_AGENT_DOCS_FILE" || {
  print -u2 -r -- "FAIL: bash agent-docs resolve must use resolve-specific formats"
  exit 1
}

grep -q "wait:UI stabilization wait primitives" "$COMP_MACOS_AGENT_FILE" || {
  print -u2 -r -- "FAIL: macos-agent completion missing wait command"
  exit 1
}

grep -q -- "--dry-run\\[" "$COMP_MACOS_AGENT_FILE" || {
  print -u2 -r -- "FAIL: macos-agent completion missing --dry-run"
  exit 1
}

grep -q -- "--window-title-contains=\\[Narrow app selection by window title\\]" "$COMP_MACOS_AGENT_FILE" || {
  print -u2 -r -- "FAIL: macos-agent completion missing canonical --window-title-contains"
  exit 1
}

grep -q -- "--window-name=\\[Deprecated alias of --window-title-contains\\]" "$COMP_MACOS_AGENT_FILE" || {
  print -u2 -r -- "FAIL: macos-agent completion missing --window-name alias contract"
  exit 1
}

grep -q -- "--submit\\[Press Enter after typing\\]" "$COMP_MACOS_AGENT_FILE" || {
  print -u2 -r -- "FAIL: macos-agent completion missing canonical --submit"
  exit 1
}

grep -q -- "--enter\\[Deprecated alias of --submit\\]" "$COMP_MACOS_AGENT_FILE" || {
  print -u2 -r -- "FAIL: macos-agent completion missing --enter alias contract"
  exit 1
}

grep -q "provider:Provider registry and selection" "$COMP_AGENTCTL_FILE" || {
  print -u2 -r -- "FAIL: agentctl completion missing provider command"
  exit 1
}

grep -q "diag:Provider-neutral diagnostics" "$COMP_AGENTCTL_FILE" || {
  print -u2 -r -- "FAIL: agentctl completion missing diag command"
  exit 1
}

grep -q "workflow:Declarative workflow orchestration" "$COMP_AGENTCTL_FILE" || {
  print -u2 -r -- "FAIL: agentctl completion missing workflow command"
  exit 1
}

grep -q -- "--probe-mode" "$COMP_AGENTCTL_FILE" || {
  print -u2 -r -- "FAIL: agentctl completion missing --probe-mode"
  exit 1
}

grep -q "fetch:Fetch pending items for agent enrichment" "$COMP_MEMO_CLI_FILE" || {
  print -u2 -r -- "FAIL: memo-cli completion missing fetch command"
  exit 1
}

grep -q "apply:Apply enrichment payloads" "$COMP_MEMO_CLI_FILE" || {
  print -u2 -r -- "FAIL: memo-cli completion missing apply command"
  exit 1
}

grep -q -- "--json\\[Output JSON\\]" "$COMP_MEMO_CLI_FILE" || {
  print -u2 -r -- "FAIL: memo-cli completion missing --json"
  exit 1
}

grep -q -- "--dry-run\\[Validate payload without write-back\\]" "$COMP_MEMO_CLI_FILE" || {
  print -u2 -r -- "FAIL: memo-cli completion missing --dry-run"
  exit 1
}

grep -q -- "--field=\\[Search fields (comma-separated)\\]" "$COMP_MEMO_CLI_FILE" || {
  print -u2 -r -- "FAIL: memo-cli completion missing --field"
  exit 1
}

grep -q -- "--window-title-contains" "$BASH_MACOS_AGENT_FILE" || {
  print -u2 -r -- "FAIL: bash macos-agent completion missing canonical --window-title-contains"
  exit 1
}

grep -q -- "--window-name" "$BASH_MACOS_AGENT_FILE" || {
  print -u2 -r -- "FAIL: bash macos-agent completion missing --window-name alias"
  exit 1
}

grep -q -- "--submit" "$BASH_MACOS_AGENT_FILE" || {
  print -u2 -r -- "FAIL: bash macos-agent completion missing canonical --submit"
  exit 1
}

grep -q -- "--enter" "$BASH_MACOS_AGENT_FILE" || {
  print -u2 -r -- "FAIL: bash macos-agent completion missing --enter alias"
  exit 1
}

grep -q "_nils_cli_agentctl_complete" "$BASH_AGENTCTL_FILE" || {
  print -u2 -r -- "FAIL: bash agentctl completion missing completion function"
  exit 1
}

grep -q -- "--include-experimental" "$BASH_AGENTCTL_FILE" || {
  print -u2 -r -- "FAIL: bash agentctl completion missing --include-experimental"
  exit 1
}

grep -q -- "--probe-mode" "$BASH_AGENTCTL_FILE" || {
  print -u2 -r -- "FAIL: bash agentctl completion missing --probe-mode"
  exit 1
}

grep -q -- "--field" "$BASH_MEMO_CLI_FILE" || {
  print -u2 -r -- "FAIL: bash memo-cli completion missing --field"
  exit 1
}

bash -c "set -euo pipefail; source \"$BASH_GIT_SCOPE_FILE\"; complete -p git-scope | grep -q _nils_cli_git_scope_complete" || {
  print -u2 -r -- "FAIL: failed to source bash git-scope completion file"
  exit 1
}

bash -c "set -euo pipefail; source \"$BASH_SUMMARY_FILE\"; complete -p git-summary | grep -q _nils_cli_git_summary_complete" || {
  print -u2 -r -- "FAIL: failed to source bash git-summary completion file"
  exit 1
}

bash -c "set -euo pipefail; source \"$BASH_LOCK_FILE\"; complete -p git-lock | grep -q _nils_cli_git_lock_complete" || {
  print -u2 -r -- "FAIL: failed to source bash git-lock completion file"
  exit 1
}

bash -c "set -euo pipefail; source \"$BASH_GIT_CLI_FILE\"; complete -p git-cli | grep -q _nils_cli_git_cli_complete; complete -p gx | grep -q _nils_cli_git_cli_complete" || {
  print -u2 -r -- "FAIL: failed to source bash git-cli completion file"
  exit 1
}

bash -c "set -euo pipefail; source \"$BASH_FZF_CLI_FILE\"; complete -p fzf-cli | grep -q _nils_cli_fzf_cli_complete; complete -p fx | grep -q _nils_cli_fzf_cli_complete" || {
  print -u2 -r -- "FAIL: failed to source bash fzf-cli completion file"
  exit 1
}

bash -c "set -euo pipefail; source \"$BASH_SEMANTIC_COMMIT_FILE\"; complete -p semantic-commit | grep -q _nils_cli_semantic_commit_complete" || {
  print -u2 -r -- "FAIL: failed to source bash semantic-commit completion file"
  exit 1
}

bash -c "set -euo pipefail; source \"$BASH_PLAN_TOOLING_FILE\"; complete -p plan-tooling | grep -q _nils_cli_plan_tooling_complete" || {
  print -u2 -r -- "FAIL: failed to source bash plan-tooling completion file"
  exit 1
}

bash -c "set -euo pipefail; source \"$BASH_API_REST_FILE\"; complete -p api-rest | grep -q _nils_cli_api_rest_complete" || {
  print -u2 -r -- "FAIL: failed to source bash api-rest completion file"
  exit 1
}

bash -c "set -euo pipefail; source \"$BASH_API_GQL_FILE\"; complete -p api-gql | grep -q _nils_cli_api_gql_complete" || {
  print -u2 -r -- "FAIL: failed to source bash api-gql completion file"
  exit 1
}

bash -c "set -euo pipefail; source \"$BASH_API_TEST_FILE\"; complete -p api-test | grep -q _nils_cli_api_test_complete" || {
  print -u2 -r -- "FAIL: failed to source bash api-test completion file"
  exit 1
}

bash -c "set -euo pipefail; source \"$BASH_CODEX_CLI_FILE\"; complete -p codex-cli | grep -q _nils_cli_codex_cli_complete; complete -p cx | grep -q _nils_cli_codex_cli_complete" || {
  print -u2 -r -- "FAIL: failed to source bash codex-cli completion file"
  exit 1
}

grep -q "auth_cmds=(login use save refresh auto-refresh current sync)" "$BASH_CODEX_CLI_FILE" || {
  print -u2 -r -- "FAIL: bash codex-cli completion missing auth login/save command set"
  exit 1
}

grep -q -- "--api-key --device-code" "$BASH_CODEX_CLI_FILE" || {
  print -u2 -r -- "FAIL: bash codex-cli completion missing login mode flags"
  exit 1
}

grep -q -- "-y --yes" "$BASH_CODEX_CLI_FILE" || {
  print -u2 -r -- "FAIL: bash codex-cli completion missing save overwrite flags"
  exit 1
}

bash -c "set -euo pipefail; source \"$BASH_AGENT_DOCS_FILE\"; complete -p agent-docs | grep -q _nils_cli_agent_docs_complete" || {
  print -u2 -r -- "FAIL: failed to source bash agent-docs completion file"
  exit 1
}

bash -c "set -euo pipefail; source \"$BASH_MACOS_AGENT_FILE\"; complete -p macos-agent | grep -q _nils_cli_macos_agent_complete" || {
  print -u2 -r -- "FAIL: failed to source bash macos-agent completion file"
  exit 1
}

bash -c "set -euo pipefail; source \"$BASH_AGENTCTL_FILE\"; complete -p agentctl | grep -q _nils_cli_agentctl_complete" || {
  print -u2 -r -- "FAIL: failed to source bash agentctl completion file"
  exit 1
}

bash -c "set -euo pipefail; source \"$BASH_MEMO_CLI_FILE\"; complete -p memo-cli | grep -q _nils_cli_memo_cli_complete" || {
  print -u2 -r -- "FAIL: failed to source bash memo-cli completion file"
  exit 1
}

bash -c "set -euo pipefail; source \"$BASH_ALIASES_FILE\"; alias gs >/dev/null; alias gx >/dev/null; alias cx >/dev/null; alias fx >/dev/null; declare -F gxur >/dev/null; declare -F fxd >/dev/null; declare -F fxh >/dev/null" || {
  print -u2 -r -- "FAIL: failed to source bash nils-cli aliases file"
  exit 1
}

print -r -- "OK"
