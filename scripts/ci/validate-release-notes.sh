#!/usr/bin/env bash
set -euo pipefail

notes="docs/releases/v${1}.md"
if [[ ! -f "$notes" ]] || ! grep -qx '## Upgrade notes' "$notes"; then
  echo "error: $notes must exist and contain an ## Upgrade notes section" >&2
  exit 1
fi
