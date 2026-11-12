#!/usr/bin/env bash
# scripts/ci/apt-install-retry.sh — install apt packages on a CI runner,
# retrying `update` + `install` on transient archive or DNS failures.
#
# Usage: scripts/ci/apt-install-retry.sh <package>...
#
# Environment:
#   APT_INSTALL_ATTEMPTS  total attempts before failing (default 4)
#   APT_INSTALL_BACKOFF   seconds slept after attempt N is N * backoff (default 5)
set -euo pipefail

if [[ $# -eq 0 ]]; then
  echo "usage: ${0##*/} <package>..." >&2
  exit 2
fi

attempts="${APT_INSTALL_ATTEMPTS:-4}"
backoff="${APT_INSTALL_BACKOFF:-5}"
apt_opts=(-o Acquire::Retries=3)

attempt=1
while true; do
  if sudo apt-get "${apt_opts[@]}" update &&
    sudo apt-get "${apt_opts[@]}" install -y "$@"; then
    exit 0
  fi
  if ((attempt >= attempts)); then
    echo "error: apt-get update/install failed after ${attempts} attempts: $*" >&2
    exit 1
  fi
  delay=$((attempt * backoff))
  echo "warning: apt-get attempt ${attempt}/${attempts} failed; retrying in ${delay}s" >&2
  sleep "$delay"
  attempt=$((attempt + 1))
done
