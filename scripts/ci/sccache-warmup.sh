#!/usr/bin/env bash
# Start or reuse sccache without passing the caller's potentially long TMPDIR
# to its startup notification socket. The server retains this directory.
set -euo pipefail

if ! command -v sccache >/dev/null 2>&1; then
  exit 0
fi

socket_tmp="/tmp/nils-cli-sccache-$UID"
if [[ -L "$socket_tmp" ]]; then
  echo "error: sccache temporary directory is a symlink: $socket_tmp" >&2
  exit 1
fi
mkdir -p -m 700 "$socket_tmp"
if [[ ! -d "$socket_tmp" || ! -O "$socket_tmp" || -L "$socket_tmp" ]]; then
  echo "error: sccache temporary directory is not privately owned: $socket_tmp" >&2
  exit 1
fi
chmod 700 "$socket_tmp"
export SCCACHE_SERVER_UDS="${SCCACHE_SERVER_UDS:-$socket_tmp/server.sock}"
if python3 - "$SCCACHE_SERVER_UDS" <<'PY'
import socket
import sys

try:
    with socket.socket(socket.AF_UNIX) as client:
        client.connect(sys.argv[1])
except OSError:
    sys.exit(1)
PY
then
  exit 0
fi
# A workspace test run can be idle longer than sccache's default 10 minutes.
SCCACHE_IDLE_TIMEOUT=0 TMPDIR="$socket_tmp" sccache --start-server >/dev/null
