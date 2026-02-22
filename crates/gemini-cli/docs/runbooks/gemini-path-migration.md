# Gemini Path Migration Runbook

## Purpose

Migrate one-time Gemini previous HOME paths to modern paths before runtime fallback removal.

| Previous path | Modern path |
|---|---|
| `$HOME/.config/gemini_secrets` | `$HOME/.gemini/secrets` |
| `$HOME/.agents/auth.json` | `$HOME/.gemini/oauth_creds.json` |

Migration behavior (`scripts/migrations/migrate-gemini-home-paths.sh`):
- default mode is dry-run
- `--yes` applies changes
- existing modern targets are never overwritten
- conflicts exit non-zero for manual resolution

## Pre-check

```bash
for path in \
  "$HOME/.config/gemini_secrets" \
  "$HOME/.agents/auth.json" \
  "$HOME/.gemini/secrets" \
  "$HOME/.gemini/oauth_creds.json"
do
  if [ -e "$path" ]; then
    ls -ald "$path"
  else
    printf 'missing %s\n' "$path"
  fi
done
```

Preview planned actions:

```bash
bash scripts/migrations/migrate-gemini-home-paths.sh
```

## Execution

```bash
bash scripts/migrations/migrate-gemini-home-paths.sh --yes
```

## Post-check

```bash
test -d "$HOME/.gemini/secrets"
test -f "$HOME/.gemini/oauth_creds.json"
test ! -e "$HOME/.config/gemini_secrets"
test ! -e "$HOME/.agents/auth.json"
```

Optional visibility check:

```bash
ls -ald "$HOME/.gemini/secrets" "$HOME/.gemini/oauth_creds.json"
```

## Validation Commands (Plan)

```bash
bash scripts/migrations/migrate-gemini-home-paths.sh --help
```

```bash
tmp="$(mktemp -d)" && mkdir -p "$tmp/home/.config/gemini_secrets" "$tmp/home/.agents" && printf '{}' > "$tmp/home/.agents/auth.json" && HOME="$tmp/home" bash scripts/migrations/migrate-gemini-home-paths.sh --yes
test -d "$tmp/home/.gemini/secrets" && test -f "$tmp/home/.gemini/oauth_creds.json"
```
