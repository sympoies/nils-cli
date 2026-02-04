# nils-cli aliases (Bash)
#
# Opt-in: source this file from your ~/.bashrc after installing nils-cli.
# Designed to avoid clobbering user-defined aliases/functions.

if [[ -z ${BASH_VERSION-} ]]; then
  return 0 2>/dev/null || exit 0
fi

shopt -s expand_aliases 2>/dev/null || true

_nils_cli__has_alias() {
  alias "$1" >/dev/null 2>&1
}

_nils_cli__has_function() {
  declare -F "$1" >/dev/null 2>&1
}

# ---------------------------------------------------------------------------
# git-scope (gs*)
# ---------------------------------------------------------------------------
_nils_cli__has_alias gs || alias gs='git-scope'

_nils_cli__has_alias gst || alias gst='git-scope tracked'
_nils_cli__has_alias gss || alias gss='git-scope staged'
_nils_cli__has_alias gsu || alias gsu='git-scope unstaged'
_nils_cli__has_alias gsa || alias gsa='git-scope all'
_nils_cli__has_alias gsun || alias gsun='git-scope untracked'
_nils_cli__has_alias gsc || alias gsc='git-scope commit'
_nils_cli__has_alias gsh || alias gsh='git-scope help'

# ---------------------------------------------------------------------------
# codex-cli (cx*)
# ---------------------------------------------------------------------------
_nils_cli__has_alias cx || alias cx='codex-cli'

_nils_cli__has_alias cxau || alias cxau='codex-cli auth use'
_nils_cli__has_alias cxar || alias cxar='codex-cli auth refresh'
_nils_cli__has_alias cxaa || alias cxaa='codex-cli auth auto-refresh'
_nils_cli__has_alias cxac || alias cxac='codex-cli auth current'
_nils_cli__has_alias cxas || alias cxas='codex-cli auth sync'

_nils_cli__has_alias cxdr || alias cxdr='codex-cli diag rate-limits'
_nils_cli__has_alias cxdra || alias cxdra='codex-cli diag rate-limits --async'

_nils_cli__has_alias cxcs || alias cxcs='codex-cli config show'
_nils_cli__has_alias cxct || alias cxct='codex-cli config set'

_nils_cli__has_alias cxgp || alias cxgp='codex-cli agent prompt'
_nils_cli__has_alias cxga || alias cxga='codex-cli agent advice'
_nils_cli__has_alias cxgk || alias cxgk='codex-cli agent knowledge'
_nils_cli__has_alias cxgc || alias cxgc='codex-cli agent commit'

_nils_cli__has_alias cxst || alias cxst='codex-cli starship'

# ---------------------------------------------------------------------------
# fzf-cli (ff*)
# ---------------------------------------------------------------------------
_nils_cli__has_alias ff || alias ff='fzf-cli'
_nils_cli__has_alias fff || alias fff='fzf-cli file'

# These use eval to preserve parent-shell effects:
_nils_cli__has_function ffd || ffd() { eval "$(fzf-cli directory -- "$@")"; }
_nils_cli__has_function ffh || ffh() { eval "$(fzf-cli history -- "$@")"; }

_nils_cli__has_alias ffgs || alias ffgs='fzf-cli git-status'
_nils_cli__has_alias ffgc || alias ffgc='fzf-cli git-commit'
_nils_cli__has_alias ffgco || alias ffgco='fzf-cli git-checkout'
_nils_cli__has_alias ffgb || alias ffgb='fzf-cli git-branch'
_nils_cli__has_alias ffgt || alias ffgt='fzf-cli git-tag'
_nils_cli__has_alias ffp || alias ffp='fzf-cli process'
_nils_cli__has_alias ffpo || alias ffpo='fzf-cli port'
_nils_cli__has_alias ffenv || alias ffenv='fzf-cli env'
_nils_cli__has_alias ffal || alias ffal='fzf-cli alias'
_nils_cli__has_alias fffn || alias fffn='fzf-cli function'
_nils_cli__has_alias ffdef || alias ffdef='fzf-cli def'

