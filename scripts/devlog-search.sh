#!/usr/bin/env bash
# Search the development log (docs/devlog/*.md).
# Usage: scripts/devlog-search.sh <term> [YYYY-MM]
#   <term>    case-insensitive literal search string (required)
#   YYYY-MM   restrict the search to one month file (optional)
set -uo pipefail

DIR="docs/devlog"
USAGE="usage: scripts/devlog-search.sh <term> [YYYY-MM]"

repo_root="$(git rev-parse --show-toplevel 2>/dev/null || true)"
if [ -z "$repo_root" ] || [ ! -d "$repo_root" ]; then
  echo "error: must run inside a git work tree" >&2
  exit 2
fi
if ! cd "$repo_root"; then
  echo "error: unable to enter the repository root" >&2
  exit 2
fi

term="${1:-}"
month="${2:-}"

if [ "$#" -gt 2 ] || [ -z "$term" ]; then
  echo "$USAGE" >&2
  exit 2
fi

if [ -n "$month" ]; then
  if [[ ! "$month" =~ ^[0-9]{4}-(0[1-9]|1[0-2])$ ]]; then
    echo "$USAGE" >&2
    exit 2
  fi
  files=("$DIR/$month.md")
else
  files=("$DIR"/????-??.md)
fi

if [ ! -e "${files[0]}" ]; then
  echo "error: no devlog month files found under $DIR" >&2
  exit 1
fi

if ! grep -n -i -F -- "$term" "${files[@]}"; then
  echo "(no matches for '$term')" >&2
  exit 1
fi
