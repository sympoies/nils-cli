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

long_tmp="$work/abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyz"
mkdir -p "$long_tmp"
TMPDIR="$long_tmp" bash "$repo_root/scripts/ci/sccache-warmup.sh"
sleep 2
TMPDIR="$long_tmp" sccache "$(command -v rustc)" -vV >/dev/null
[[ -d "$long_tmp" ]]
sccache --stop-server >/dev/null
TMPDIR="$work" sccache --start-server >/dev/null
sleep 2
TMPDIR="$long_tmp" bash "$repo_root/scripts/ci/sccache-warmup.sh"
TMPDIR="$long_tmp" sccache "$(command -v rustc)" -vV >/dev/null
echo "PASS: sccache survives idle and restarts after an existing server expires"
