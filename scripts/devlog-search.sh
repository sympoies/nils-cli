#!/usr/bin/env bash
# Search the development log (docs/devlog/*.md).
#
# `-e` is deliberately omitted: grep returns 1 for "no matches", which this
# script reports as its own documented exit status rather than aborting on.
set -uo pipefail

# An exported BASHOPTS can carry nullglob into this shell, which would turn an
# unmatched month glob into an empty array instead of a literal path. Clear it
# so the emptiness check below owns that case.
shopt -u nullglob

DIR="docs/devlog"

usage() {
  cat <<'USAGE'
Usage:
  devlog-search.sh <term> [YYYY-MM]

Search the development log under docs/devlog.

Arguments:
  <term>     Case-insensitive literal search string (required)
  YYYY-MM    Restrict the search to one month file (optional)

Options:
  -h, --help Show this help

Exit status:
  0  at least one match
  1  no matches, or no devlog month files to search
  2  usage error
USAGE
}

while [[ $# -gt 0 ]]; do
  case "${1:-}" in
    -h|--help)
      usage
      exit 0
      ;;
    *)
      break
      ;;
  esac
done

repo_root="$(git rev-parse --show-toplevel 2>/dev/null || true)"
if [[ -z "$repo_root" || ! -d "$repo_root" ]]; then
  echo "error: must run inside a git work tree" >&2
  exit 2
fi
if ! cd "$repo_root"; then
  echo "error: unable to enter the repository root" >&2
  exit 2
fi

term="${1:-}"
month="${2:-}"

if [[ $# -gt 2 || -z "$term" ]]; then
  usage >&2
  exit 2
fi

declare -a files=()
if [[ -n "$month" ]]; then
  if [[ ! "$month" =~ ^[0-9]{4}-(0[1-9]|1[0-2])$ ]]; then
    usage >&2
    exit 2
  fi
  if [[ ! -e "$DIR/$month.md" ]]; then
    echo "error: no devlog file for month: $DIR/$month.md" >&2
    exit 1
  fi
  files=("$DIR/$month.md")
else
  files=("$DIR"/????-??.md)
  if [[ ${#files[@]} -eq 0 || ! -e "${files[0]}" ]]; then
    echo "error: no devlog month files found under $DIR" >&2
    exit 1
  fi
fi

if ! grep -n -i -F -- "$term" "${files[@]}"; then
  echo "(no matches for '$term')" >&2
  exit 1
fi
