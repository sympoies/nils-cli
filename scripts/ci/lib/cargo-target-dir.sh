# shellcheck shell=bash
# Shared by the completion audits: locate the cargo target directory that a
# `cargo build` run from the repository root writes binaries into.
#
# Resolution order:
#   1. CARGO_TARGET_DIR (isolated-target gates, e.g. a shared gate runner).
#   2. `target_directory` from `cargo metadata`, which also covers
#      `build.target-dir` in cargo config. Only attempted when the root has a
#      Cargo.toml; any failure falls through to the default.
#   3. `<repo_root>/target` (the cargo default).
#
# A relative value is resolved against the repository root because the audits
# run cargo with the repository root as its working directory.

cargo_metadata_target_directory() {
  local repo_root="$1"
  [[ -f "$repo_root/Cargo.toml" ]] || return 1
  command -v cargo >/dev/null 2>&1 || return 1
  command -v python3 >/dev/null 2>&1 || return 1
  (
    cd "$repo_root" || exit 1
    cargo metadata --no-deps --offline --format-version 1 2>/dev/null
  ) | python3 -c 'import json, sys; print(json.load(sys.stdin).get("target_directory", ""))' 2>/dev/null
}

resolve_cargo_target_dir() {
  local repo_root="$1"
  local target="${CARGO_TARGET_DIR:-}"

  if [[ -z "$target" ]]; then
    target="$(cargo_metadata_target_directory "$repo_root" || true)"
  fi
  if [[ -z "$target" ]]; then
    target="target"
  fi

  case "$target" in
    /* | [A-Za-z]:[\\/]*) ;;
    *) target="$repo_root/$target" ;;
  esac
  printf '%s\n' "${target%/}"
}
