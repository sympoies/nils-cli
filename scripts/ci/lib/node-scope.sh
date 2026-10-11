# shellcheck shell=bash
# Run a Node.js command (node, npx, npm) inside a memory-bounded systemd user
# scope: MemoryMax=6G and no swap. Inside a gate that exports
# NILS_CLI_GATE_SLICE, the scope is placed under that slice, so the gate's own
# limits, monitoring and cleanup still cover it. Where no systemd user manager
# can start a scope (for example macOS and most hosted CI runners), the command
# runs unchanged.

node_scope_available() {
  if [[ -z "${NILS_CLI_NODE_SCOPE_AVAILABLE:-}" ]]; then
    NILS_CLI_NODE_SCOPE_AVAILABLE=0
    if command -v systemd-run >/dev/null 2>&1 &&
      systemd-run --user --scope --quiet -p MemoryMax=64M -p MemorySwapMax=0 -- true >/dev/null 2>&1; then
      NILS_CLI_NODE_SCOPE_AVAILABLE=1
    fi
  fi
  [[ "$NILS_CLI_NODE_SCOPE_AVAILABLE" == 1 ]]
}

node_scoped() {
  if ! node_scope_available; then
    "$@"
    return
  fi
  local -a scope=(systemd-run --user --scope --quiet -p MemoryMax=6G -p MemorySwapMax=0)
  if [[ -n "${NILS_CLI_GATE_SLICE:-}" ]]; then
    scope+=("--slice=${NILS_CLI_GATE_SLICE}")
  fi
  "${scope[@]}" -- "$@"
}
