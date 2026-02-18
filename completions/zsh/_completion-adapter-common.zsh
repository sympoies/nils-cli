# nils-cli zsh completion adapter common helpers
#
# Shared by thin zsh adapters that load clap-generated completion scripts.

if [[ -z ${ZSH_VERSION-} ]]; then
  return 0 2>/dev/null || exit 0
fi

if [[ -n ${_NILS_CLI_COMPLETION_ADAPTER_COMMON_ZSH_LOADED-} ]]; then
  return 0
fi

typeset -g _NILS_CLI_COMPLETION_ADAPTER_COMMON_ZSH_LOADED=1

_nils_cli_completion_common_fail_closed_no_legacy_zsh() {
  # Explicit no-legacy policy: fail closed and do not route to legacy completers.
  return 1
}

_nils_cli_completion_common_load_generated_zsh() {
  emulate -L zsh -o extendedglob

  local state_var="${1-}"
  local generated_fn="${2-}"
  local cli_bin="${3-}"
  local generated_symbol="${4-}"
  local strip_begin_regex="${5-}"
  local strip_end_regex="${6-}"

  if [[ -z "$state_var" || -z "$generated_fn" || -z "$cli_bin" || -z "$generated_symbol" ]]; then
    _nils_cli_completion_common_fail_closed_no_legacy_zsh
    return 1
  fi

  local state="${(P)state_var-0}"
  if [[ "$state" == "1" ]]; then
    (( $+functions[$generated_fn] )) && return 0
    typeset -g "${state_var}=0"
  elif [[ "$state" == "-1" ]]; then
    _nils_cli_completion_common_fail_closed_no_legacy_zsh
    return 1
  fi

  local script=''
  script="$(command "$cli_bin" completion zsh 2>/dev/null)" || {
    typeset -g "${state_var}=-1"
    _nils_cli_completion_common_fail_closed_no_legacy_zsh
    return 1
  }

  script="${script//${generated_symbol}/${generated_fn}}"

  if [[ -n "$strip_begin_regex" && -n "$strip_end_regex" ]]; then
    script="$(printf '%s\n' "$script" | sed "/${strip_begin_regex}/,/${strip_end_regex}/d")" || {
      typeset -g "${state_var}=-1"
      _nils_cli_completion_common_fail_closed_no_legacy_zsh
      return 1
    }
  fi

  eval "$script" || {
    typeset -g "${state_var}=-1"
    _nils_cli_completion_common_fail_closed_no_legacy_zsh
    return 1
  }

  if (( ! $+functions[$generated_fn] )); then
    typeset -g "${state_var}=-1"
    _nils_cli_completion_common_fail_closed_no_legacy_zsh
    return 1
  fi

  typeset -g "${state_var}=1"
  return 0
}

_nils_cli_completion_common_register_zsh() {
  emulate -L zsh

  local completion_fn="${1-}"
  shift || true

  if [[ -z "$completion_fn" ]]; then
    return 1
  fi

  (( $+functions[compdef] )) || return 0
  compdef "$completion_fn" "$@"
}
