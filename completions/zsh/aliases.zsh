# nils-cli aliases (Zsh)
#
# Opt-in: source this file from your ~/.zshrc after installing nils-cli.
# Designed to avoid clobbering user-defined aliases/functions.

if [[ -z ${ZSH_VERSION-} ]]; then
  return 0 2>/dev/null || exit 0
fi

# ---------------------------------------------------------------------------
# git-scope (gs*)
# ---------------------------------------------------------------------------
(( $+aliases[gs] )) || alias gs='git-scope'

(( $+aliases[gst] )) || alias gst='git-scope tracked'
(( $+aliases[gss] )) || alias gss='git-scope staged'
(( $+aliases[gsu] )) || alias gsu='git-scope unstaged'
(( $+aliases[gsa] )) || alias gsa='git-scope all'
(( $+aliases[gsun] )) || alias gsun='git-scope untracked'
(( $+aliases[gsc] )) || alias gsc='git-scope commit'
(( $+aliases[gsh] )) || alias gsh='git-scope help'

# ---------------------------------------------------------------------------
# codex-cli (cx*)
# ---------------------------------------------------------------------------
(( $+aliases[cx] )) || alias cx='codex-cli'

(( $+aliases[cxau] )) || alias cxau='codex-cli auth use'
(( $+aliases[cxar] )) || alias cxar='codex-cli auth refresh'
(( $+aliases[cxaa] )) || alias cxaa='codex-cli auth auto-refresh'
(( $+aliases[cxac] )) || alias cxac='codex-cli auth current'
(( $+aliases[cxas] )) || alias cxas='codex-cli auth sync'

(( $+aliases[cxdr] )) || alias cxdr='codex-cli diag rate-limits'
(( $+aliases[cxdra] )) || alias cxdra='codex-cli diag rate-limits --async'

(( $+aliases[cxcs] )) || alias cxcs='codex-cli config show'
(( $+aliases[cxct] )) || alias cxct='codex-cli config set'

(( $+aliases[cxgp] )) || alias cxgp='codex-cli agent prompt'
(( $+aliases[cxga] )) || alias cxga='codex-cli agent advice'
(( $+aliases[cxgk] )) || alias cxgk='codex-cli agent knowledge'
(( $+aliases[cxgc] )) || alias cxgc='codex-cli agent commit'

(( $+aliases[cxst] )) || alias cxst='codex-cli starship'

# ---------------------------------------------------------------------------
# fzf-cli (fx*)
# ---------------------------------------------------------------------------
(( $+aliases[fx] )) || alias fx='fzf-cli'
(( $+aliases[fxf] )) || alias fxf='fzf-cli file'

# These use eval to preserve parent-shell effects:
if (( ! $+functions[fxd] )); then
  fxd() { eval "$(fzf-cli directory -- "$@")"; }
fi
if (( ! $+functions[fxh] )); then
  fxh() { eval "$(fzf-cli history -- "$@")"; }
fi

(( $+aliases[fxgs] )) || alias fxgs='fzf-cli git-status'
(( $+aliases[fxgc] )) || alias fxgc='fzf-cli git-commit'
(( $+aliases[fxgco] )) || alias fxgco='fzf-cli git-checkout'
(( $+aliases[fxgb] )) || alias fxgb='fzf-cli git-branch'
(( $+aliases[fxgt] )) || alias fxgt='fzf-cli git-tag'
(( $+aliases[fxp] )) || alias fxp='fzf-cli process'
(( $+aliases[fxpo] )) || alias fxpo='fzf-cli port'
(( $+aliases[fxenv] )) || alias fxenv='fzf-cli env'
(( $+aliases[fxal] )) || alias fxal='fzf-cli alias'
(( $+aliases[fxfn] )) || alias fxfn='fzf-cli function'
(( $+aliases[fxdef] )) || alias fxdef='fzf-cli def'
