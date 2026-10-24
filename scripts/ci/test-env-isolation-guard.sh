#!/usr/bin/env bash
# Guard: representative tests must not touch the developer's real agent state.
#
# Runs selected cargo tests under `env -i` with a poisoned environment: every
# home/config variable points at a sentinel directory, and the auth-sync and
# remote-auth hooks are commands that record a hit. A hermetic test never
# reads or writes those locations nor runs those commands, so afterwards every
# sentinel must be byte-for-byte unchanged and no hit may be recorded.
#
# Usage: scripts/ci/test-env-isolation-guard.sh [--list]
#   --list   print the representative test selection and exit
#
# A run that cannot build also fails the guard; ordinary test failures are left to
# the regular gate, and test output is never printed because it may echo fixture data.
#
# Environment:
#   ENV_GUARD_CARGO_ARGS  extra arguments for `cargo test` (for example --offline)
#   ENV_GUARD_SELECTION   newline-separated package|target|filter entries replacing the default
#   ENV_GUARD_KEEP=1      keep the sentinel root after the run for inspection
set -euo pipefail

# package (or `workspace`)|test target|test name filter. The target is a test binary name, `lib`
# for the unit tests, or `all` for every target of the package.
SELECTION=(
  "nils-test-support|lib|"
  "nils-codex-cli|all|"
  "nils-secrets|all|"
  "nils-agent-session|lib|"
  "nils-agent-session|integration|"
  "nils-agent-session|startup_failure|"
  "nils-agent-hook|lib|"
  "nils-agent-runtime|lib|"
  "nils-forge-cli|integration|"
)

# A caller can replace the selection with newline-separated entries in the same
# format, for example to run whole packages under the poisoned environment.
if [[ -n "${ENV_GUARD_SELECTION:-}" ]]; then
  SELECTION=()
  while IFS= read -r line; do
    [[ -n "$line" ]] && SELECTION+=("$line")
  done <<<"$ENV_GUARD_SELECTION"
fi

if [[ "${1:-}" == "--list" ]]; then
  printf '%s\n' "${SELECTION[@]}"
  exit 0
fi

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
real_cargo_home="${CARGO_HOME:-${HOME:?HOME is required to locate cargo}/.cargo}"
real_rustup_home="${RUSTUP_HOME:-${HOME}/.rustup}"

root="$(mktemp -d "${TMPDIR:-/tmp}/nils-env-guard.XXXXXX")"
if [[ "${ENV_GUARD_KEEP:-0}" != 1 ]]; then
  trap 'rm -rf -- "$root"' EXIT
fi
sentinels="$root/sentinels"
scratch="$root/scratch"
mkdir -p "$sentinels" "$scratch/tmp" "$scratch/runtime" "$root/bin"
chmod 700 "$scratch/runtime"

sentinel_dirs=(home agent-home codex-home claude-home xdg-config xdg-state xdg-data xdg-cache secret-dir)
for dir in "${sentinel_dirs[@]}"; do
  mkdir -p "$sentinels/$dir"
done
# Canary credential files: present so a test that copies or rewrites them is caught.
printf '%s\n' '{"canary":"guard-auth-canary"}' >"$sentinels/codex-home/auth.json"
printf '%s\n' '[user]' >"$sentinels/git-config"

# Any command a test runs through an inherited hook records a hit.
hit_command="$root/bin/poison-hit"
cat >"$hit_command" <<EOF
#!/bin/sh
printf '%s\n' "\$0 \$*" >>"$sentinels/hits"
EOF
chmod +x "$hit_command"

# One line per entry: path, type, size, mtime, and a content digest for files.
snapshot() {
  python3 -I - "$sentinels" <<'PY'
import hashlib, os, sys

root = sys.argv[1]
rows = []
for base, dirs, files in os.walk(root):
    for name in dirs + files:
        path = os.path.join(base, name)
        info = os.lstat(path)
        digest = ""
        if os.path.isfile(path) and not os.path.islink(path):
            with open(path, "rb") as handle:
                digest = hashlib.sha256(handle.read()).hexdigest()
        rows.append(f"{os.path.relpath(path, root)}\t{info.st_mode:o}\t{info.st_size}\t{info.st_mtime_ns}\t{digest}")
print("\n".join(sorted(rows)))
PY
}

# Empty RUSTC_WRAPPER overrides Cargo's build.rustc-wrapper config, preventing
# a user sccache cache from writing under the poisoned XDG_CACHE_HOME sentinel.
poisoned_env() {
  env -i \
    PATH="$PATH" \
    LANG=C.UTF-8 \
    TERM=dumb \
    TMPDIR="$scratch/tmp" \
    XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-$scratch/runtime}" \
    CARGO_HOME="$real_cargo_home" \
    RUSTUP_HOME="$real_rustup_home" \
    CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$repo_root/target}" \
    CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-4}" \
    RUSTC_WRAPPER="" \
    HOME="$sentinels/home" \
    AGENT_HOME="$sentinels/agent-home" \
    CODEX_HOME="$sentinels/codex-home" \
    CODEX_AUTH_FILE="$sentinels/codex-home/auth.json" \
    CODEX_SECRET_DIR="$sentinels/secret-dir" \
    CODEX_SECRET_CACHE_DIR="$sentinels/secret-dir" \
    CLAUDE_CONFIG_DIR="$sentinels/claude-home" \
    CLAUDE_HOME="$sentinels/claude-home" \
    GIT_CONFIG_GLOBAL="$sentinels/git-config" \
    XDG_CONFIG_HOME="$sentinels/xdg-config" \
    XDG_STATE_HOME="$sentinels/xdg-state" \
    XDG_DATA_HOME="$sentinels/xdg-data" \
    XDG_CACHE_HOME="$sentinels/xdg-cache" \
    CODEX_PROXY_AUTH_SYNC_ENABLED=true \
    CODEX_PROXY_AUTH_SYNC_COMMAND="$hit_command" \
    CODEX_AUTH_REMOTE_SSH="$hit_command" \
    CODEX_AUTH_REMOTE_NAME=guard-remote \
    CODEX_AUTH_REMOTE_REFRESH=true "$@"
}

# Binaries owned by other packages must exist before a package-scoped run.
printf 'env-guard: cargo build -p nils-agent-hook -p nils-agent-session --bins\n' >&2
poisoned_env cargo build -p nils-agent-hook -p nils-agent-session --bins >"$scratch/build.log" 2>&1 || {
  printf 'env-guard: FAIL: helper binaries did not build\n' >&2
  exit 1
}

before="$(snapshot)"
status=0
leaked=()
for entry in "${SELECTION[@]}"; do
  IFS='|' read -r package target filter <<<"$entry"
  # nextest runs each test in its own process, which also avoids the
  # fork-vs-fresh-executable ETXTBSY race that a threaded `cargo test` has.
  use_nextest=0
  if poisoned_env cargo nextest --version >/dev/null 2>&1; then
    use_nextest=1
    args=(nextest run --profile ci --no-fail-fast)
  else
    args=(test --no-fail-fast)
  fi
  # `workspace` selects every package.
  if [[ "$package" == workspace ]]; then
    args+=(--workspace)
  else
    args+=(-p "$package")
  fi
  case "$target" in
    all) ;;
    lib) args+=(--lib) ;;
    bins) args+=(--bins) ;;
    *) args+=(--test "$target") ;;
  esac
  # shellcheck disable=SC2206
  args+=(${ENV_GUARD_CARGO_ARGS:-})
  [[ -n "$filter" ]] && args+=("$filter")
  printf 'env-guard: cargo %s\n' "${args[*]}" >&2
  entry_before="$(snapshot)"
  rc=0
  poisoned_env cargo "${args[@]}" >"$scratch/run-$package-$target.log" 2>&1 || rc=$?
  # Test output can echo fixture data, so it is never printed here. A failing
  # test (nextest exit 100) is the regular gate's concern and may be a load flake;
  # this guard fails on a build or setup failure, where nothing was exercised.
  # nextest exits 4 when a target has no tests, which proves nothing is wrong.
  if [[ "$rc" -ne 0 && ! ( "$use_nextest" -eq 1 && ( "$rc" -eq 100 || "$rc" -eq 4 ) ) ]]; then
    printf 'env-guard: FAIL: cargo for %s exited %s (log kept with ENV_GUARD_KEEP=1)\n' "$package" "$rc" >&2
    status=1
  elif [[ "$rc" -ne 0 ]]; then
    printf 'env-guard: note: %s has failing tests; the regular gate reports them\n' "$package" >&2
  fi
  # Attribute a change to the selection entry that made it.
  entry_after="$(snapshot)"
  if [[ "$entry_after" != "$entry_before" ]]; then
    leaked+=("$package|$target")
    printf 'env-guard: sentinel change during %s (%s):\n' "$package" "$target" >&2
    diff <(printf '%s\n' "$entry_before" | cut -f1) <(printf '%s\n' "$entry_after" | cut -f1) |
      grep '^[<>]' | head -n 20 >&2 || true
  fi
done

after="$(snapshot)"
if [[ "$before" != "$after" ]]; then
  printf 'env-guard: FAIL: tests changed the poisoned sentinel locations\n' >&2
  status=1
fi
if [[ -e "$sentinels/hits" ]]; then
  printf 'env-guard: FAIL: an inherited hook command ran during the tests\n' >&2
  status=1
fi
[[ "$status" -eq 0 ]] && printf 'env-guard: ok\n' >&2
exit "$status"
