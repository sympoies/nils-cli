#!/usr/bin/env bash
set -euo pipefail

# Self-test for scripts/ci/completion-flag-parity-audit.sh.
#
# Uses a synthetic mini workspace with fake binaries so the audit's report
# contract and its per-binary concurrency are exercised without building the
# real workspace.

repo_root="$(git rev-parse --show-toplevel 2>/dev/null || true)"
if [[ -z "$repo_root" || ! -d "$repo_root" ]]; then
  echo "error: must run inside the nils-cli git work tree" >&2
  exit 2
fi

script="$repo_root/scripts/ci/completion-flag-parity-audit.sh"
if [[ ! -f "$script" ]]; then
  echo "error: missing flag parity audit script: $script" >&2
  exit 2
fi

# The audit honors an ambient isolated target; keep the default-layout fixtures
# below independent of the caller's environment.
unset CARGO_TARGET_DIR

tmp="$(mktemp -d "${TMPDIR:-/tmp}/completion-flag-parity-test.XXXXXX")"
cleanup() {
  rm -rf "$tmp"
}
trap cleanup EXIT

failures=0
fail() {
  echo "FAIL: $*" >&2
  failures=$((failures + 1))
}

matrix_row() {
  local bin="$1"
  printf '| `%s` | `required` | `present` (`_%s`) | `present` (`%s`) | not required | `completion_mode=clap-first; completion_mode_toggles=forbidden; alternate_completion_dispatch=forbidden; generated_load_failure=fail-closed` | synthetic test binary |\n' \
    "$bin" "$bin" "$bin"
}

# Build a mini workspace whose fake binaries answer `--help`, `completion bash`,
# and `completion zsh` like a clap CLI with root options only.
#
#   make_workspace <dir> <bin>:<help flags>:<bash flags>:<zsh flags> ...
#
# A binary named `*-waits` blocks its `--help` until a sibling binary named
# `*-signals` has started, and fails if that never happens. That only passes
# when the audit runs binaries concurrently.
make_workspace() {
  local dir="$1"
  shift
  mkdir -p "$dir/scripts/ci/lib" "$dir/docs/specs" "$dir/target/debug"
  cp "$script" "$dir/scripts/ci/completion-flag-parity-audit.sh"
  cp "$(dirname "$script")/lib/cargo-target-dir.sh" "$dir/scripts/ci/lib/cargo-target-dir.sh"

  {
    echo '| Binary | Obligation | Zsh completion (`completions/zsh`) | Bash completion (`completions/bash`) | Alias requirement | Completion enforcement metadata | Rationale |'
    echo '| --- | --- | --- | --- | --- | --- | --- |'
  } >"$dir/docs/specs/completion-coverage-matrix-v1.md"

  local spec bin help_flags bash_flags zsh_flags
  for spec in "$@"; do
    IFS=':' read -r bin help_flags bash_flags zsh_flags <<<"$spec"
    matrix_row "$bin" >>"$dir/docs/specs/completion-coverage-matrix-v1.md"

    local help_lines="" zsh_specs="" flag
    for flag in $help_flags; do
      help_lines+="      ${flag}  Synthetic ${flag#--} option"$'\n'
    done
    for flag in $zsh_flags; do
      zsh_specs+="'${flag}[Synthetic ${flag#--} option]' \\"$'\n'
    done
    local label="${bin//-/__}"

    cat >"$dir/target/debug/$bin" <<EOF
#!/usr/bin/env bash
set -euo pipefail
marker_dir="$dir/markers"
case "\${1:-}" in
  completion)
    mkdir -p "\$marker_dir"
    : >"\$marker_dir/$bin.started"
    if [[ "\${2:-}" == "bash" ]]; then
      cat <<'BASH'
_${bin}() {
    case "\${cmd}" in
        ${label})
            opts="-h --help ${bash_flags}"
            ;;
    esac
}
BASH
    else
      cat <<'ZSH'
#compdef ${bin}
_${bin}() {
    _arguments "\${_arguments_options[@]}" : \\
${zsh_specs}'-h[Print help]' \\
'--help[Print help]' \\
&& ret=0
}
ZSH
    fi
    ;;
  --help)
    if [[ "$bin" == *-waits ]]; then
      for _ in \$(seq 1 60); do
        if compgen -G "\$marker_dir/*-signals.started" >/dev/null; then
          break
        fi
        sleep 0.05
      done
      if ! compgen -G "\$marker_dir/*-signals.started" >/dev/null; then
        echo "sibling binary never started" >&2
        exit 1
      fi
    fi
    cat <<'HELP'
Usage: ${bin} [OPTIONS]

Options:
${help_lines}  -h, --help  Print help
HELP
    ;;
  *)
    echo "unexpected ${bin} invocation: \$*" >&2
    exit 64
    ;;
esac
EOF
    chmod +x "$dir/target/debug/$bin"
  done
}

run_audit() {
  local dir="$1"
  local jobs="$2"
  local out_file="$3"
  local code=0
  COMPLETION_PARITY_JOBS="$jobs" \
    bash "$dir/scripts/ci/completion-flag-parity-audit.sh" --strict >"$out_file" 2>&1 || code=$?
  printf '%s' "$code"
}

# 1. Report contract: failures from several binaries keep binary order and
#    exact wording whether the audit runs serially or concurrently. `zeta-cli`
#    completes only `--color-mode`, which must not count as `--color`.
contract="$tmp/contract"
make_workspace "$contract" \
  "alpha-cli:--verbose --color:--verbose --color:--color" \
  "beta-cli:--json:--json:--json" \
  "gamma-cli:--dry-run --force:--dry-run:--dry-run --force" \
  "zeta-cli:--color:--color-mode:--color-mode"

expected_contract="$(cat <<'EOF'
FAIL: alpha-cli: zsh completion missing flag `--verbose` for command `<root>`
FAIL: gamma-cli: bash completion missing flag `--force` for command `<root>`
FAIL: zeta-cli: bash completion missing flag `--color` for command `<root>`
FAIL: zeta-cli: zsh completion missing flag `--color` for command `<root>`
FAIL: completion flag parity audit (required=4, dynamic_engine_skipped=0, failures=4)
EOF
)"

for jobs in 1 4; do
  out="$tmp/contract-$jobs.out"
  code="$(run_audit "$contract" "$jobs" "$out")"
  if [[ "$code" != "1" ]]; then
    fail "contract fixture with COMPLETION_PARITY_JOBS=$jobs exited $code, expected 1"
  fi
  if [[ "$(cat "$out")" != "$expected_contract" ]]; then
    fail "contract fixture with COMPLETION_PARITY_JOBS=$jobs reported:"$'\n'"$(cat "$out")"$'\n'"expected:"$'\n'"$expected_contract"
  fi
done

# 2. Nested commands: the zsh block for `run` is found through its
#    `curcontext` marker, and a script without that block is reported.
nested="$tmp/nested"
make_workspace "$nested"
{
  matrix_row delta-cli
  matrix_row epsilon-cli
} >>"$nested/docs/specs/completion-coverage-matrix-v1.md"

write_nested_bin() {
  local bin="$1"
  local zsh_run_block="$2"
  local label="${bin//-/__}"
  cat >"$nested/target/debug/$bin" <<EOF
#!/usr/bin/env bash
set -euo pipefail
case "\${1:-} \${2:-}" in
  "completion bash")
    cat <<'BASH'
_${bin}() {
    case "\${cmd}" in
        ${label})
            opts="-h --help run"
            ;;
        ${label}__run)
            opts="-h --help --fast"
            ;;
    esac
}
BASH
    ;;
  "completion zsh")
    cat <<'ZSH'
#compdef ${bin}
_${bin}() {
    _arguments "\${_arguments_options[@]}" : \\
'-h[Print help]' \\
'--help[Print help]' \\
":: :_${label}_commands" \\
&& ret=0
    case \$state in
    (${bin})
        curcontext="\${curcontext%:*:*}:${bin}-command-\$line[1]:"
        case \$line[1] in
${zsh_run_block}
        esac
    ;;
    esac
}
ZSH
    ;;
  "--help "*)
    printf 'Usage: ${bin} <COMMAND>\n\nCommands:\n  run   Run it\n  help  Print this message\n\nOptions:\n  -h, --help  Print help\n'
    ;;
  "run --help")
    printf 'Usage: ${bin} run [OPTIONS]\n\nOptions:\n      --fast  Go fast\n  -h, --help  Print help\n'
    ;;
  *)
    echo "unexpected ${bin} invocation: \$*" >&2
    exit 64
    ;;
esac
EOF
  chmod +x "$nested/target/debug/$bin"
}

write_nested_bin delta-cli "$(cat <<'EOF'
            (run)
_arguments "${_arguments_options[@]}" : \
'--fast[Go fast]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
EOF
)"
write_nested_bin epsilon-cli ""

expected_nested="$(cat <<'EOF'
FAIL: epsilon-cli: missing zsh completion block for run
FAIL: completion flag parity audit (required=2, dynamic_engine_skipped=0, failures=1)
EOF
)"
out="$tmp/nested.out"
code="$(run_audit "$nested" 4 "$out")"
if [[ "$code" != "1" || "$(cat "$out")" != "$expected_nested" ]]; then
  fail "nested fixture exited $code and reported:"$'\n'"$(cat "$out")"$'\n'"expected:"$'\n'"$expected_nested"
fi

# 3. Concurrency: `alpha-waits` can only print help while `beta-signals` is
#    being audited, which a serial audit never allows.
concurrent="$tmp/concurrent"
make_workspace "$concurrent" \
  "alpha-waits:--verbose:--verbose:--verbose" \
  "beta-signals:--json:--json:--json"

out="$tmp/concurrent.out"
code="$(run_audit "$concurrent" 4 "$out")"
if [[ "$code" != "0" ]]; then
  fail "concurrent fixture exited $code; binaries were not audited concurrently:"$'\n'"$(cat "$out")"
fi
if ! grep -qx 'PASS: completion flag parity audit (required=2, dynamic_engine_skipped=0, failures=0)' "$out"; then
  fail "concurrent fixture did not report PASS:"$'\n'"$(cat "$out")"
fi

# 4. Isolated cargo target: binaries that only exist under CARGO_TARGET_DIR
#    (absolute or relative, with spaces) are audited in place. A stub `cargo`
#    records any build attempt so the audit cannot silently rebuild instead.
stub_dir="$tmp/stub-bin"
mkdir -p "$stub_dir"
cat >"$stub_dir/cargo" <<'EOF'
#!/usr/bin/env bash
echo "$*" >>"$STUB_CARGO_LOG"
echo "stub cargo refuses to build" >&2
exit 1
EOF
chmod +x "$stub_dir/cargo"

run_isolated_audit() {
  # Run from an unrelated cwd so a relative CARGO_TARGET_DIR must resolve
  # against the audited root, where cargo itself would run.
  local dir="$1"
  local out_file="$2"
  shift 2
  local code=0
  (cd "$tmp" && env "$@" PATH="$stub_dir:$PATH" STUB_CARGO_LOG="$dir/cargo.log" \
    COMPLETION_PARITY_JOBS=2 \
    bash "$dir/scripts/ci/completion-flag-parity-audit.sh" --strict) >"$out_file" 2>&1 || code=$?
  printf '%s' "$code"
}

expected_isolated='PASS: completion flag parity audit (required=2, dynamic_engine_skipped=0, failures=0)'

iso_abs="$tmp/iso-abs"
make_workspace "$iso_abs" \
  "alpha-cli:--verbose:--verbose:--verbose" \
  "beta-cli:--json:--json:--json"
mkdir -p "$tmp/iso abs target"
mv "$iso_abs/target/debug" "$tmp/iso abs target/debug"
rmdir "$iso_abs/target"
out="$tmp/iso-abs.out"
code="$(run_isolated_audit "$iso_abs" "$out" "CARGO_TARGET_DIR=$tmp/iso abs target")"
if [[ "$code" != "0" ]] || ! grep -qxF "$expected_isolated" "$out"; then
  fail "absolute isolated target fixture exited $code and reported:"$'\n'"$(cat "$out")"
fi
if [[ -e "$iso_abs/cargo.log" ]]; then
  fail "absolute isolated target fixture attempted a cargo build: $(cat "$iso_abs/cargo.log")"
fi

iso_rel="$tmp/iso-rel"
make_workspace "$iso_rel" \
  "alpha-cli:--verbose:--verbose:--verbose" \
  "beta-cli:--json:--json:--json"
mv "$iso_rel/target" "$iso_rel/rel target"
out="$tmp/iso-rel.out"
code="$(run_isolated_audit "$iso_rel" "$out" "CARGO_TARGET_DIR=rel target")"
if [[ "$code" != "0" ]] || ! grep -qxF "$expected_isolated" "$out"; then
  fail "relative isolated target fixture exited $code and reported:"$'\n'"$(cat "$out")"
fi
if [[ -e "$iso_rel/cargo.log" ]]; then
  fail "relative isolated target fixture attempted a cargo build: $(cat "$iso_rel/cargo.log")"
fi

# A genuinely missing binary still fails (via the build step) and still names
# the effective target directory, not the default one.
iso_missing="$tmp/iso-missing"
make_workspace "$iso_missing" "alpha-cli:--verbose:--verbose:--verbose"
mkdir -p "$tmp/iso missing target/debug"
out="$tmp/iso-missing.out"
code="$(run_isolated_audit "$iso_missing" "$out" "CARGO_TARGET_DIR=$tmp/iso missing target")"
if [[ "$code" == "0" ]]; then
  fail "missing isolated binary fixture should fail:"$'\n'"$(cat "$out")"
fi
if [[ ! -s "$iso_missing/cargo.log" ]]; then
  fail "missing isolated binary fixture did not reach the build step"
fi

if (( failures > 0 )); then
  echo "FAIL: completion-flag-parity-audit self-test ($failures failure(s))" >&2
  exit 1
fi

echo "ok: completion-flag-parity-audit self-test passed"
