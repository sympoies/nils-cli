#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage: release-only-checks.sh --base <git-ref> [--head <git-ref>] (--branch <name> | --merge-group)

Runs the reduced cross-platform validation lane after the caller has proved
that the base main commit completed full CI.
USAGE
}

base_ref=""
head_ref="HEAD"
branch=""
merge_group=0
while [[ $# -gt 0 ]]; do
  case "${1:-}" in
    --base)
      [[ $# -ge 2 ]] || { usage >&2; exit 2; }
      base_ref="$2"
      shift 2
      ;;
    --head)
      [[ $# -ge 2 ]] || { usage >&2; exit 2; }
      head_ref="$2"
      shift 2
      ;;
    --branch)
      [[ $# -ge 2 ]] || { usage >&2; exit 2; }
      [[ "$merge_group" -eq 0 && -z "$branch" ]] || { usage >&2; exit 2; }
      branch="$2"
      shift 2
      ;;
    --merge-group)
      [[ "$merge_group" -eq 0 && -z "$branch" ]] || { usage >&2; exit 2; }
      merge_group=1
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

if [[ -z "$base_ref" || -z "$head_ref" || ( -z "$branch" && "$merge_group" -ne 1 ) ]]; then
  usage >&2
  exit 2
fi

repo_root="$(git rev-parse --show-toplevel 2>/dev/null || true)"
if [[ -z "$repo_root" || ! -d "$repo_root" ]]; then
  echo "error: must run inside the nils-cli git work tree" >&2
  exit 2
fi
cd "$repo_root"

release_detector="${RELEASE_ONLY_DETECTOR:-scripts/ci/detect-release-only.sh}"
detector_args=(--base "$base_ref" --head "$head_ref")
if [[ "$merge_group" -eq 1 ]]; then
  detector_args+=(--merge-group)
else
  detector_args+=(--branch "$branch")
fi
verdict="$(bash "$release_detector" "${detector_args[@]}")"
if [[ "$verdict" != "true" ]]; then
  echo "error: release-only contract is no longer satisfied; refusing the reduced lane" >&2
  exit 1
fi

bash scripts/ci/workspace-version-lockstep.sh --strict
bash scripts/generate-third-party-artifacts.sh --check
bash scripts/ci/publish-order-audit.sh --strict
cargo check --workspace --all-targets --all-features --locked

echo "ok: release-only checks passed"
