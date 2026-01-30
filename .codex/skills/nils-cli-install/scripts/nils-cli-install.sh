#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage:
  nils-cli-install.sh [--help] [--prefix PATH] [--bin NAME]... [--skip-build]

Builds the Rust workspace in release mode and installs the binaries into a local
directory (default: ~/.local/nils-cli).

Options:
  --prefix PATH   Destination directory (default: ~/.local/nils-cli)
  --bin NAME      Install only a specific binary (repeatable)
  --skip-build    Skip `cargo build --release --workspace` and only install from target/
  -h, --help      Show help

Default binaries:
  - cli-template
  - fzf-cli
  - git-lock
  - git-scope
  - git-summary
  - image-processing
  - semantic-commit

Example:
  ./.codex/skills/nils-cli-install/scripts/nils-cli-install.sh
  ./.codex/skills/nils-cli-install/scripts/nils-cli-install.sh --bin git-scope
  ./.codex/skills/nils-cli-install/scripts/nils-cli-install.sh --prefix ~/.local/nils-cli
USAGE
}

prefix="${HOME}/.local/nils-cli"
skip_build=0
bins=()

while [[ $# -gt 0 ]]; do
  case "${1:-}" in
    -h|--help)
      usage
      exit 0
      ;;
    --prefix)
      prefix="${2:-}"
      if [[ -z "$prefix" ]]; then
        echo "error: --prefix requires a path" >&2
        exit 2
      fi
      shift 2
      ;;
    --bin)
      if [[ -z "${2:-}" ]]; then
        echo "error: --bin requires a binary name" >&2
        exit 2
      fi
      bins+=( "${2}" )
      shift 2
      ;;
    --skip-build)
      skip_build=1
      shift
      ;;
    *)
      echo "error: unknown argument: ${1:-}" >&2
      usage >&2
      exit 2
      ;;
  esac
done

if [[ "$prefix" == "~" ]]; then
  prefix="$HOME"
elif [[ "$prefix" == "~/"* ]]; then
  prefix="$HOME/${prefix#~/}"
fi

default_bins=(cli-template fzf-cli git-lock git-scope git-summary image-processing semantic-commit)
if [[ ${#bins[@]} -eq 0 ]]; then
  bins=( "${default_bins[@]}" )
fi

for cmd in git cargo install; do
  if ! command -v "$cmd" >/dev/null 2>&1; then
    echo "error: missing required tool on PATH: $cmd" >&2
    exit 2
  fi
done

repo_root="$(git rev-parse --show-toplevel 2>/dev/null || true)"
if [[ -z "$repo_root" || ! -d "$repo_root" ]]; then
  echo "error: must run inside a git work tree" >&2
  exit 2
fi

cd "$repo_root"

run() {
  local -a cmd=( "$@" )
  echo "+ ${cmd[*]}"
  if "${cmd[@]}"; then
    return 0
  else
    local code=$?
    echo "error: command failed (exit $code): ${cmd[*]}" >&2
    exit "$code"
  fi
}

if [[ "$skip_build" -eq 0 ]]; then
  run cargo build --release --workspace
fi

run mkdir -p "$prefix"

for bin in "${bins[@]}"; do
  src="$repo_root/target/release/$bin"
  if [[ ! -x "$src" ]]; then
    echo "error: release binary not found or not executable: $src" >&2
    echo "hint: run: cargo build --release --workspace" >&2
    exit 1
  fi
  run install -m 0755 "$src" "$prefix/"
done

echo "ok: installed ${#bins[@]} binaries into: $prefix"
echo "note: add to PATH if needed:"
echo "  export PATH=\"$prefix:\$PATH\""
