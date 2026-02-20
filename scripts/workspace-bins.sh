#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$script_dir/.." && pwd)"

metadata_json="$(cargo metadata --no-deps --format-version 1 --manifest-path "$repo_root/Cargo.toml" | tr -d '\n')"

printf '%s\n' "$metadata_json" \
  | awk '
      {
        text = $0
        while (match(text, /"kind":\[[^][]*\],"crate_types":\[[^][]*\],"name":"[^"]+"/)) {
          block = substr(text, RSTART, RLENGTH)
          text = substr(text, RSTART + RLENGTH)

          if (block ~ /"kind":\[[^]]*"bin"[^]]*\]/) {
            name = block
            sub(/^.*"name":"/, "", name)
            sub(/".*$/, "", name)
            print name
          }
        }
      }
    ' \
  | LC_ALL=C sort -u
