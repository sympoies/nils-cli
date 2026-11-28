#!/usr/bin/env bash
set -euo pipefail

notes="docs/releases/v${1}.md"
if [[ ! -f "$notes" ]] || ! awk '
  /^## Upgrade notes$/ { section = 1; next }
  section && /^#{1,2}[[:space:]]/ { exit }
  section && /[^[:space:]]/ { found = 1; exit }
  END { exit !found }
' "$notes"; then
  echo "error: $notes must exist and contain a nonempty ## Upgrade notes section (use None. when no action is needed)" >&2
  exit 1
fi
