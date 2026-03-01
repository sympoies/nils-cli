#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage:
  scripts/ci/wrapper-mode-smoke.sh [--wrapper PATH] [--help]

Description:
  Runs an isolated smoke test for wrapper execution modes:
  - NILS_WRAPPER_MODE=debug
  - NILS_WRAPPER_MODE=installed
  - NILS_WRAPPER_MODE=auto (prefer installed + fallback to cargo)

Options:
  --wrapper PATH   Wrapper script to test (default: wrappers/cli-template)
  -h, --help       Show help
USAGE
}

die() {
  echo "error: $*" >&2
  exit 1
}

assert_file_has_content() {
  local file="$1"
  local label="$2"
  if [[ ! -s "$file" ]]; then
    die "$label expected content in $file"
  fi
}

assert_file_empty() {
  local file="$1"
  local label="$2"
  if [[ -s "$file" ]]; then
    die "$label expected empty file: $file"
  fi
}

assert_file_contains() {
  local file="$1"
  local pattern="$2"
  local label="$3"
  if ! grep -Fq "$pattern" "$file"; then
    die "$label expected pattern '$pattern' in $file"
  fi
}

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
wrapper="$repo_root/wrappers/cli-template"

while [[ $# -gt 0 ]]; do
  case "${1:-}" in
    --wrapper)
      [[ $# -ge 2 ]] || die "--wrapper requires a path"
      wrapper="${2}"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      die "unknown argument: ${1:-}"
      ;;
  esac
done

if [[ "$wrapper" != /* ]]; then
  wrapper="$repo_root/$wrapper"
fi

[[ -f "$wrapper" ]] || die "wrapper not found: $wrapper"
[[ -x "$wrapper" ]] || die "wrapper is not executable: $wrapper"

bin_name="$(basename "$wrapper")"

tmp_dir="$(mktemp -d)"
trap 'rm -rf "$tmp_dir"' EXIT

fake_path="$tmp_dir/path"
install_prefix="$tmp_dir/install"
mkdir -p "$fake_path" "$install_prefix"

cargo_log="$tmp_dir/cargo.log"
installed_log="$tmp_dir/installed.log"

touch "$cargo_log" "$installed_log"

cat > "$fake_path/cargo" <<'FAKE_CARGO'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" >> "${WRAPPER_TEST_CARGO_LOG:?}"
printf 'fake-cargo\n'
FAKE_CARGO
chmod +x "$fake_path/cargo"

cat > "$install_prefix/$bin_name" <<'FAKE_INSTALLED'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" >> "${WRAPPER_TEST_INSTALLED_LOG:?}"
printf 'fake-installed\n'
FAKE_INSTALLED
chmod +x "$install_prefix/$bin_name"

run_wrapper() {
  local mode="$1"
  local out_file="$2"
  local err_file="$3"
  shift 3

  WRAPPER_TEST_CARGO_LOG="$cargo_log" \
  WRAPPER_TEST_INSTALLED_LOG="$installed_log" \
  PATH="$fake_path:/usr/bin:/bin" \
  NILS_WRAPPER_MODE="$mode" \
  NILS_WRAPPER_INSTALL_PREFIX="$install_prefix" \
  "$wrapper" "$@" >"$out_file" 2>"$err_file"
}

reset_logs() {
  : > "$cargo_log"
  : > "$installed_log"
}

# 1) debug mode: force cargo path.
reset_logs
run_wrapper debug "$tmp_dir/debug.out" "$tmp_dir/debug.err" smoke-debug
assert_file_has_content "$cargo_log" "debug mode"
assert_file_empty "$installed_log" "debug mode"
assert_file_contains "$tmp_dir/debug.err" "exec=cargo mode=debug" "debug mode status hint"

# 2) installed mode: force installed binary path.
reset_logs
run_wrapper installed "$tmp_dir/installed.out" "$tmp_dir/installed.err" smoke-installed
assert_file_has_content "$installed_log" "installed mode"
assert_file_empty "$cargo_log" "installed mode"
assert_file_contains "$tmp_dir/installed.err" "exec=installed mode=installed" "installed mode status hint"

# 3) auto mode: prefer installed binary when present.
reset_logs
run_wrapper auto "$tmp_dir/auto-prefer.out" "$tmp_dir/auto-prefer.err" smoke-auto-prefer
assert_file_has_content "$installed_log" "auto mode (prefer installed)"
assert_file_empty "$cargo_log" "auto mode (prefer installed)"
assert_file_contains "$tmp_dir/auto-prefer.err" "exec=installed mode=auto" "auto mode (prefer installed) status hint"

# 4) auto mode fallback: if installed missing, fallback to cargo.
rm -f "$install_prefix/$bin_name"
reset_logs
run_wrapper auto "$tmp_dir/auto-fallback.out" "$tmp_dir/auto-fallback.err" smoke-auto-fallback
assert_file_has_content "$cargo_log" "auto mode (fallback cargo)"
assert_file_empty "$installed_log" "auto mode (fallback cargo)"
assert_file_contains "$tmp_dir/auto-fallback.err" "exec=cargo mode=auto" "auto mode (fallback cargo) status hint"

echo "ok: wrapper mode smoke tests passed for $wrapper"
