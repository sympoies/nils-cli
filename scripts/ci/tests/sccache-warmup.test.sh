#!/usr/bin/env bash
set -euo pipefail

if ! command -v sccache >/dev/null 2>&1; then
  echo "skip: sccache is unavailable"
  exit 0
fi

repo_root="$(git rev-parse --show-toplevel)"
work="$(mktemp -d /tmp/nils-sccache-test.XXXXXX)"
export SCCACHE_SERVER_UDS="$work/socket"
export SCCACHE_IDLE_TIMEOUT=1
cleanup() {
  sccache --stop-server >/dev/null 2>&1 || true
  rm -rf "$work"
}
trap cleanup EXIT

wait_for_socket_state() {
  python3 - "$SCCACHE_SERVER_UDS" "$1" "$2" <<'PY'
import socket
import sys
import time

path, expected, description = sys.argv[1:]
deadline = time.monotonic() + 15
unreachable_since = None
while True:
    try:
        with socket.socket(socket.AF_UNIX) as client:
            client.settimeout(0.2)
            client.connect(path)
        reachable = True
    except OSError:
        reachable = False
    now = time.monotonic()
    if expected == "reachable" and reachable:
        break
    if expected == "unreachable" and not reachable:
        if unreachable_since is None:
            unreachable_since = now
        if now - unreachable_since >= 0.2:
            break
    else:
        unreachable_since = None
    if now >= deadline:
        print(f"FAIL: {description}", file=sys.stderr)
        sys.exit(1)
    time.sleep(0.05)
PY
}

long_tmp="$work/abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyz"
mkdir -p "$long_tmp"
TMPDIR="$long_tmp" bash "$repo_root/scripts/ci/sccache-warmup.sh"
sleep 2
wait_for_socket_state reachable "warmup server did not remain reachable"
TMPDIR="$long_tmp" sccache "$(command -v rustc)" -vV >/dev/null
[[ -d "$long_tmp" ]]
sccache --stop-server >/dev/null
TMPDIR="$work" sccache --start-server >/dev/null
wait_for_socket_state unreachable "existing sccache server did not expire"
TMPDIR="$long_tmp" bash "$repo_root/scripts/ci/sccache-warmup.sh"
wait_for_socket_state reachable "restarted sccache server did not become reachable"
TMPDIR="$long_tmp" sccache "$(command -v rustc)" -vV >/dev/null
echo "PASS: sccache survives idle and restarts after an existing server expires"
