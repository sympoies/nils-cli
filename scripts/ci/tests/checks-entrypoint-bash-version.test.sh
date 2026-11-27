#!/usr/bin/env bash
set -euo pipefail

repo_root="$(git rev-parse --show-toplevel)"
entrypoint="$repo_root/scripts/ci/nils-cli-checks-entrypoint.sh"
if [[ -z "${BASH_3_2:-}" ]] && /bin/bash -c '(( BASH_VERSINFO[0] == 3 && BASH_VERSINFO[1] == 2 ))'; then
  BASH_3_2=/bin/bash
fi
if [[ -n "${BASH_3_2:-}" ]]; then
  set +e
  output="$("$BASH_3_2" "$entrypoint" --local-fast 2>&1)"
  status=$?
  set -e
  if [[ "$status" -ne 2 || "$output" != 'error: Bash 4.4 or newer is required; install it and invoke this script with that bash.' ]]; then
    printf 'AssertionError: Bash 3.2 should fail immediately with the version diagnostic; got status %s: %s\n' "$status" "$output" >&2
    exit 1
  fi
fi

fixture="$(mktemp -d)"
trap 'rm -rf "$fixture"' EXIT
cat > "$fixture/bash" <<'SCRIPT'
#!/bin/sh
echo 'FAIL: nested script used PATH bash' >&2
exit 91
SCRIPT
chmod +x "$fixture/bash"
output="$(PATH="$fixture:$PATH" "$BASH" "$entrypoint" --local-fast --changed-file '' 2>&1)" || {
  printf 'FAIL: supported Bash should propagate to nested scripts: %s\n' "$output" >&2
  exit 1
}
[[ "$output" == *'ok: local-fast found no changed files'* ]] || {
  printf 'FAIL: expected empty selection: %s\n' "$output" >&2
  exit 1
}
echo 'ok: entrypoint Bash version and interpreter selection'
