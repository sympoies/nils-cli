#!/usr/bin/env bash
# scripts/ci/tempdir-leak-probe.sh — run a test selection against a private,
# empty TMPDIR and fail if anything survives the run.
#
# This is the runtime counterpart to tempdir-leak-audit.sh. The audit greps for
# handles whose cleanup can *never* run; it cannot see the two classes that
# actually dominated this workspace:
#
#   * cleanup runs, then a detached child re-creates the tree. `write_atomic`
#     calls `create_dir_all` before writing, so a child that writes one marker
#     after teardown resurrects the whole fixture directory.
#   * a lock or marker is written as a *sibling* of the fixture root, so it
#     lands next to the `TempDir` in `$TMPDIR` where teardown never reaches.
#
# Neither is visible by inspection and neither fails a test, so the only honest
# detector is to give the run an empty directory and look at what is left.
#
# Why a private TMPDIR and not a diff of the shared /tmp: the shared directory
# has concurrent writers (editors, agents, the user's own shells), so a
# before/after diff reports their entries as leaks. It also hides the real ones
# — temp directories are dotfiles, and a plain `ls` does not list them.
#
# Do NOT point TMPDIR inside any repository work tree. Many tests assert that a
# path is outside a git repository (`*_outside_git_repo`, `not_a_repo`), and a
# TMPDIR with Git marker ancestry invalidates those fixtures. The probe therefore
# selects the first writable system temp root outside Git marker ancestry and
# outside host paths hidden by workspace sandbox mount plans.
#
# Compatibility: must run on macOS (system bash 3.2) and Linux runners. Avoid
# associative arrays, mapfile, and `${var,,}` lowercasing.

set -euo pipefail

usage() {
  cat <<'USAGE'
Usage:
  bash scripts/ci/tempdir-leak-probe.sh [--runs <n>] [--probe-root <dir>]
                                        [--] [<cargo nextest args>...]

Runs `cargo nextest run` with TMPDIR pointed at a private empty directory and
fails if the run leaves anything behind. Defaults to `--workspace` when no
nextest arguments are given.

  --runs <n>         repeat the run <n> times before checking (default: 1).
                     Several leaks are races that only lose sometimes; repeat
                     to raise the odds of catching one.
  --probe-root <dir> where to create the private TMPDIR (default: the first
                     writable system temp root outside Git marker ancestry).
                     Must not have Git marker ancestry or live below `/run` or
                     `/dev`, which workspace sandboxes replace with private
                     mounts.
  --allow <glob>     tolerate a surviving entry whose name matches <glob>.
                     Repeatable. Only for state a test deliberately reuses
                     across runs under a fixed name — such state is bounded, so
                     it does not grow the way a leak does. Never use it to
                     silence a randomly-named directory.

Exits 0 when nothing is left behind, 1 on a leak or a failing test run, 2 on a
usage error.
USAGE
}

runs=1
probe_root=""
probe_root_explicit=0
allow_globs=()
while [ $# -gt 0 ]; do
  case "$1" in
    -h | --help)
      usage
      exit 0
      ;;
    --allow)
      shift
      if [ $# -eq 0 ]; then
        echo "tempdir-leak-probe: --allow requires a glob" >&2
        exit 2
      fi
      allow_globs+=("$1")
      shift
      ;;
    --runs)
      shift
      if [ $# -eq 0 ]; then
        echo "tempdir-leak-probe: --runs requires a count" >&2
        exit 2
      fi
      runs="$1"
      shift
      ;;
    --probe-root)
      shift
      if [ $# -eq 0 ]; then
        echo "tempdir-leak-probe: --probe-root requires a path" >&2
        exit 2
      fi
      probe_root="$1"
      probe_root_explicit=1
      shift
      ;;
    --)
      shift
      break
      ;;
    -*)
      break
      ;;
    *)
      break
      ;;
  esac
done

case "$runs" in
  '' | *[!0-9]*)
    echo "tempdir-leak-probe: --runs must be a positive integer: $runs" >&2
    exit 2
    ;;
esac
if [ "$runs" -lt 1 ]; then
  echo "tempdir-leak-probe: --runs must be at least 1" >&2
  exit 2
fi

has_git_marker_ancestry() {
  local cursor="$1"
  local parent
  while :; do
    if [ -e "$cursor/.git" ] || [ -L "$cursor/.git" ]; then
      return 0
    fi
    parent="$(dirname "$cursor")"
    if [ "$parent" = "$cursor" ]; then
      return 1
    fi
    cursor="$parent"
  done
}

SANDBOX_MASKED_ROOTS="/run /proc /dev"

has_sandbox_masked_ancestry() {
  local path="$1"
  local root
  for root in $SANDBOX_MASKED_ROOTS; do
    case "$path/" in
      "$root"/*)
        return 0
        ;;
    esac
  done
  return 1
}

select_default_probe_root() {
  local candidate
  local candidate_abs
  for candidate in "${TMPDIR:-}" "${XDG_RUNTIME_DIR:-}" /var/tmp /dev/shm /tmp; do
    [ -n "$candidate" ] || continue
    [ -d "$candidate" ] || continue
    [ -w "$candidate" ] || continue
    candidate_abs="$(cd "$candidate" && pwd -P)" || continue
    if ! has_git_marker_ancestry "$candidate_abs" &&
      ! has_sandbox_masked_ancestry "$candidate_abs"; then
      printf '%s\n' "$candidate_abs"
      return 0
    fi
  done
  return 1
}

if [ "$probe_root_explicit" -eq 0 ]; then
  if ! probe_root="$(select_default_probe_root)"; then
    echo "tempdir-leak-probe: no writable system temp root outside Git marker ancestry" >&2
    exit 2
  fi
fi
if [ ! -d "$probe_root" ]; then
  echo "tempdir-leak-probe: not a directory: $probe_root" >&2
  exit 2
fi

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
probe_root_abs="$(cd "$probe_root" && pwd -P)"
if has_git_marker_ancestry "$probe_root_abs"; then
  echo "tempdir-leak-probe: --probe-root must be outside Git marker ancestry;" >&2
  echo "  tests that assert a path is outside a git repo fail under it." >&2
  exit 2
fi
if has_sandbox_masked_ancestry "$probe_root_abs"; then
  echo "tempdir-leak-probe: --probe-root must remain visible to sandboxed tests;" >&2
  echo "  /run and /dev are replaced by private mounts." >&2
  exit 2
fi

if [ $# -eq 0 ]; then
  set -- --workspace
fi

probe_state="$(mktemp -d "$probe_root_abs/tempdir-leak-probe.XXXXXX")"
probe_dir="$probe_state/tmp"
mkdir "$probe_dir" "$probe_state/index"
# Nextest 0.9.98+ supports per-test wrappers. Keep the repository profile and
# add the wrapper only for this probe, so ordinary nextest runs are unchanged.
python3 - "$repo_root" "$probe_state/config.toml" <<'PYCONFIG'
import json
import pathlib
import sys
root = pathlib.Path(sys.argv[1])
config = (root / ".config/nextest.toml").read_text()
wrapper = json.dumps(str(root / "scripts/ci/tempdir-leak-wrapper.sh"))
pathlib.Path(sys.argv[2]).write_text(
    'experimental = ["wrapper-scripts"]\n' + config +
    '\n[scripts.wrapper.tempdir-probe]\ncommand = ["bash", ' + wrapper + ']\n'
    '[[profile.default.scripts]]\nfilter = "all()"\nrun-wrapper = "tempdir-probe"\n')
PYCONFIG
cleanup() {
  rm -rf "$probe_state"
}
trap cleanup EXIT

cd "$repo_root"

# Build before entering the disposable TMPDIR. Persistent compiler helpers such
# as sccache retain the environment from their first request; starting one under
# the probe directory would leave it pointing at a path cleanup removes before
# later Rust commands run. The selected tests still execute under the private
# directory below, which is the behavior this probe owns.
if ! python3 "$repo_root/scripts/ci/gate-reports.py" cargo nextest run --no-run "$@"; then
  echo "tempdir-leak-probe: the test build failed" >&2
  exit 1
fi

run_status=0
i=1
while [ "$i" -le "$runs" ]; do
  if [ "$runs" -gt 1 ]; then
    echo "tempdir-leak-probe: run $i/$runs"
  fi
  if ! TMPDIR="$probe_dir" TEMP_PROBE_STATE="$probe_state" python3 "$repo_root/scripts/ci/gate-reports.py" cargo nextest run --config-file "$probe_state/config.toml" "$@"; then
    run_status=1
  fi
  i=$((i + 1))
done

if ! python3 "$repo_root/scripts/ci/tempdir-leak-report.py" "$probe_dir" "$probe_state/index" ${allow_globs+"${allow_globs[@]}"}; then
  exit 1
fi

if [ "$run_status" -ne 0 ]; then
  echo "tempdir-leak-probe: no leak, but the test run failed" >&2
  exit 1
fi

echo "tempdir-leak-probe: ok — nothing left behind"
