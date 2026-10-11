# shellcheck shell=bash
# Run a Node.js command (node, npx, npm) inside a memory-bounded systemd user
# scope: MemoryMax=6G and no swap.
#
# Inside a gate that exports NILS_CLI_GATE_SLICE, the scope is placed under that
# slice, so the gate's own limits, monitoring and cleanup still cover it. There
# the scope is required: if it cannot be started, the command fails closed and
# never runs.
#
# Outside a gate, a host where no systemd user manager can start a scope (for
# example macOS and most hosted CI runners) runs the command unchanged.

# Probe result for this shell: empty (unprobed), 1 (available) or 0. It is reset
# on every source, so an inherited environment value cannot skip the probe.
_nils_node_scope_state=""

node_scope_available() {
  if [[ -z "$_nils_node_scope_state" ]]; then
    _nils_node_scope_state=0
    if command -v systemd-run >/dev/null 2>&1 &&
      systemd-run --user --scope --quiet -p MemoryMax=64M -p MemorySwapMax=0 -- true >/dev/null 2>&1; then
      _nils_node_scope_state=1
    fi
  fi
  [[ "$_nils_node_scope_state" == 1 ]]
}

node_scoped() {
  if ! node_scope_available; then
    if [[ -n "${NILS_CLI_GATE_SLICE:-}" ]]; then
      echo "error: node scope unavailable inside gate slice ${NILS_CLI_GATE_SLICE}; refusing to run: $1" >&2
      return 1
    fi
    "$@"
    return
  fi
  local -a scope=(systemd-run --user --scope --quiet -p MemoryMax=6G -p MemorySwapMax=0)
  if [[ -n "${NILS_CLI_GATE_SLICE:-}" ]]; then
    scope+=("--slice=${NILS_CLI_GATE_SLICE}")
  fi
  "${scope[@]}" -- "$@"
}
