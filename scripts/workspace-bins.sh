#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage:
  scripts/workspace-bins.sh [--release-default]

Lists workspace bin target names from `cargo metadata`, sorted unique.

Options:
  --release-default  List binaries for release packaging and default local
                     installs. Excludes cli-template and bins gated by
                     `[[bin]] required-features`.
  -h, --help         Show this help.
USAGE
}

release_default=0
while [[ $# -gt 0 ]]; do
  case "${1:-}" in
    --release-default)
      release_default=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "error: unknown argument: ${1:-}" >&2
      usage >&2
      exit 2
      ;;
  esac
done

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$script_dir/.." && pwd)"

metadata_json="$(cargo metadata --no-deps --format-version 1 --manifest-path "$repo_root/Cargo.toml" | tr -d '\n')"

printf '%s\n' "$metadata_json" \
  | awk -v release_default="$release_default" '
      {
        text = $0
        while (match(text, /"kind":\[[^][]*\],"crate_types":\[[^][]*\],"name":"[^"]+"[^{}]*/)) {
          block = substr(text, RSTART, RLENGTH)
          text = substr(text, RSTART + RLENGTH)

          if (block !~ /"kind":\[[^]]*"bin"[^]]*\]/) continue

          if (release_default == "1" && block ~ /"required-features":\[[^][]+\]/) continue

          name = block
          sub(/^.*"name":"/, "", name)
          sub(/".*$/, "", name)
          if (release_default == "1" && name == "cli-template") continue
          print name
        }
      }
    ' \
  | LC_ALL=C sort -u
