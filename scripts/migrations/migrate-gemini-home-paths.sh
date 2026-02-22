#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: migrate-gemini-home-paths.sh [--yes] [--help]

Migrate Gemini previous HOME paths to modern paths:
  $HOME/.config/gemini_secrets -> $HOME/.gemini/secrets
  $HOME/.agents/auth.json      -> $HOME/.gemini/oauth_creds.json

Options:
  --yes   Apply changes (default is dry-run)
  --help  Show this help message

Behavior:
  - Dry-run by default (safe, non-destructive)
  - Moves previous paths only when the modern target does not already exist
  - Never overwrites an existing modern target
  - Exits non-zero when conflicts need manual resolution
EOF
}

apply_changes=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --yes)
      apply_changes=1
      ;;
    --help|-h)
      usage
      exit 0
      ;;
    *)
      printf 'Error: unknown option: %s\n\n' "$1" >&2
      usage >&2
      exit 64
      ;;
  esac
  shift
done

if [[ -z "${HOME:-}" ]]; then
  printf 'Error: HOME is not set.\n' >&2
  exit 1
fi

old_secrets="$HOME/.config/gemini_secrets"
modern_secrets="$HOME/.gemini/secrets"
old_auth="$HOME/.agents/auth.json"
modern_auth="$HOME/.gemini/oauth_creds.json"

planned=0
migrated=0
skipped=0
conflicts=0

log() {
  printf '%s\n' "$*"
}

migrate_path() {
  local label="$1"
  local old_path="$2"
  local modern_path="$3"

  if [[ ! -e "$old_path" ]]; then
    if [[ -e "$modern_path" ]]; then
      log "[ok] $label: modern path already present ($modern_path)"
    else
      log "[skip] $label: previous path not found ($old_path)"
    fi
    skipped=$((skipped + 1))
    return
  fi

  if [[ -e "$modern_path" ]]; then
    log "[conflict] $label: modern path already exists; leaving previous path unchanged."
    log "          previous: $old_path"
    log "          modern: $modern_path"
    conflicts=$((conflicts + 1))
    return
  fi

  planned=$((planned + 1))

  if [[ "$apply_changes" -eq 0 ]]; then
    log "[plan] $label: move $old_path -> $modern_path"
    return
  fi

  mkdir -p -- "$(dirname -- "$modern_path")"
  mv -- "$old_path" "$modern_path"
  migrated=$((migrated + 1))
  log "[migrated] $label: $old_path -> $modern_path"
}

log "Gemini previous-path migration"
if [[ "$apply_changes" -eq 0 ]]; then
  log "Mode: dry-run (no filesystem changes)"
else
  log "Mode: apply"
fi

migrate_path "Secrets directory" "$old_secrets" "$modern_secrets"
migrate_path "OAuth credentials" "$old_auth" "$modern_auth"

if [[ "$apply_changes" -eq 0 ]]; then
  log "Summary: planned=$planned skipped=$skipped conflicts=$conflicts"
  if [[ "$planned" -gt 0 ]]; then
    log "Re-run with --yes to apply planned migrations."
  fi
else
  log "Summary: migrated=$migrated skipped=$skipped conflicts=$conflicts"
fi

if [[ "$conflicts" -gt 0 ]]; then
  log "One or more conflicts require manual resolution."
  exit 1
fi

exit 0
