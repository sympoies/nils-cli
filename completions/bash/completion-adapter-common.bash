# nils-cli bash completion adapter common helpers
#
# Shared by thin bash adapters that load clap-generated completion scripts.

if [[ -z ${BASH_VERSION-} ]]; then
  return 0 2>/dev/null || exit 0
fi

if [[ -n ${_NILS_CLI_COMPLETION_ADAPTER_COMMON_BASH_LOADED-} ]]; then
  return 0
fi

_NILS_CLI_COMPLETION_ADAPTER_COMMON_BASH_LOADED=1

_nils_cli_completion_common_fail_closed_bash() {
  # Single-path policy: fail closed and do not route to alternate completers.
  COMPREPLY=()
  return 0
}

_nils_cli_completion_common_has_word_bash() {
  local needle="$1"
  shift

  local w=''
  for w in "$@"; do
    [[ "$w" == "$needle" ]] && return 0
  done
  return 1
}

_nils_cli_completion_common_load_generated_bash() {
  local state_var="${1-}"
  local generated_fn="${2-}"
  local cli_bin="${3-}"
  local generated_symbol="${4-}"
  local strip_begin_regex="${5-}"
  local strip_end_regex="${6-}"

  if [[ -z "$state_var" || -z "$generated_fn" || -z "$cli_bin" || -z "$generated_symbol" ]]; then
    _nils_cli_completion_common_fail_closed_bash
    return 1
  fi

  local state="${!state_var:-0}"
  if [[ "$state" == "1" ]]; then
    declare -F "$generated_fn" >/dev/null 2>&1 && return 0
    printf -v "$state_var" '%s' '0'
  elif [[ "$state" == "-1" ]]; then
    _nils_cli_completion_common_fail_closed_bash
    return 1
  fi

  local script=''
  script="$(command "$cli_bin" completion bash 2>/dev/null)" || {
    printf -v "$state_var" '%s' '-1'
    _nils_cli_completion_common_fail_closed_bash
    return 1
  }

  script="${script//${generated_symbol}/${generated_fn}}"

  if [[ -n "$strip_begin_regex" && -n "$strip_end_regex" ]]; then
    script="$(printf '%s\n' "$script" | sed "/${strip_begin_regex}/,/${strip_end_regex}/d")" || {
      printf -v "$state_var" '%s' '-1'
      _nils_cli_completion_common_fail_closed_bash
      return 1
    }
  fi

  eval "$script" || {
    printf -v "$state_var" '%s' '-1'
    _nils_cli_completion_common_fail_closed_bash
    return 1
  }

  declare -F "$generated_fn" >/dev/null 2>&1 || {
    printf -v "$state_var" '%s' '-1'
    _nils_cli_completion_common_fail_closed_bash
    return 1
  }

  printf -v "$state_var" '%s' '1'
  return 0
}

_nils_cli_completion_common_register_bash() {
  local completion_fn="${1-}"
  shift || true

  if [[ -z "$completion_fn" ]]; then
    return 1
  fi

  complete -F "$completion_fn" "$@"
}
