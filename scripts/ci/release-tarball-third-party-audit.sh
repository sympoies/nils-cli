#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage:
  scripts/ci/release-tarball-third-party-audit.sh --target <target-triple> [--tag <tag>] [--dist-dir <path>]

Checks that a release tarball includes required third-party artifacts:
  - THIRD_PARTY_LICENSES.md
  - THIRD_PARTY_NOTICES.md

Options:
  --target <triple>  Required Rust target triple used in tarball name.
  --tag <tag>        Optional release tag. If omitted, script requires exactly one tarball for the target.
  --dist-dir <path>  Optional dist directory. Default: dist
  -h, --help         Show this help.
USAGE
}

target=""
tag=""
dist_dir="dist"

while [[ $# -gt 0 ]]; do
  case "${1:-}" in
    --target)
      if [[ $# -lt 2 ]]; then
        echo "error: --target requires a value" >&2
        usage >&2
        exit 2
      fi
      target="$2"
      shift 2
      ;;
    --tag)
      if [[ $# -lt 2 ]]; then
        echo "error: --tag requires a value" >&2
        usage >&2
        exit 2
      fi
      tag="$2"
      shift 2
      ;;
    --dist-dir)
      if [[ $# -lt 2 ]]; then
        echo "error: --dist-dir requires a value" >&2
        usage >&2
        exit 2
      fi
      dist_dir="$2"
      shift 2
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

if [[ -z "$target" ]]; then
  echo "error: --target is required" >&2
  usage >&2
  exit 2
fi

repo_root="$(git rev-parse --show-toplevel 2>/dev/null || true)"
if [[ -z "$repo_root" || ! -d "$repo_root" ]]; then
  echo "error: must run inside a git work tree" >&2
  exit 2
fi
cd "$repo_root"

if [[ ! -d "$dist_dir" ]]; then
  echo "FAIL: missing dist directory: $dist_dir"
  exit 1
fi

if [[ -n "$tag" ]]; then
  tarball="${dist_dir}/nils-cli-${tag}-${target}.tar.gz"
else
  mapfile -t matches < <(find "$dist_dir" -maxdepth 1 -type f -name "nils-cli-*-${target}.tar.gz" -print | LC_ALL=C sort)
  if (( ${#matches[@]} == 0 )); then
    echo "FAIL: no tarball found for target: ${target} in ${dist_dir}"
    exit 1
  fi
  if (( ${#matches[@]} > 1 )); then
    echo "error: multiple tarballs found for target ${target}; pass --tag to disambiguate" >&2
    printf '%s\n' "${matches[@]}" >&2
    exit 2
  fi
  tarball="${matches[0]}"
fi

if [[ ! -f "$tarball" ]]; then
  echo "FAIL: missing tarball: $tarball"
  exit 1
fi

tmp_dir="$(mktemp -d "${TMPDIR:-/tmp}/release-tarball-third-party-audit.XXXXXX")"
listing_file="${tmp_dir}/contents.txt"

cleanup() {
  rm -rf "$tmp_dir"
}
trap cleanup EXIT

if ! tar -tzf "$tarball" >"$listing_file"; then
  echo "error: failed to read tarball: $tarball" >&2
  exit 2
fi

if ! command -v rg >/dev/null 2>&1; then
  echo "error: rg (ripgrep) is required for release tarball audit" >&2
  exit 2
fi

required_artifacts=("THIRD_PARTY_LICENSES.md" "THIRD_PARTY_NOTICES.md")
missing_count=0
for artifact in "${required_artifacts[@]}"; do
  artifact_pattern="/${artifact//./\\.}$"
  if ! rg -q "$artifact_pattern" "$listing_file"; then
    echo "FAIL: missing required file in tarball: ${artifact}"
    missing_count=$((missing_count + 1))
  fi
done

if (( missing_count > 0 )); then
  echo "FAIL: release tarball third-party audit (target=${target}, missing=${missing_count}, tarball=${tarball})"
  exit 1
fi

echo "PASS: release tarball third-party audit (target=${target}, missing=0, tarball=${tarball})"
