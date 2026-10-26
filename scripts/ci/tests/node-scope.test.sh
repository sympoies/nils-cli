#!/usr/bin/env bash
set -euo pipefail

# Self-test for scripts/ci/lib/node-scope.sh. A fake systemd-run records its
# arguments and runs the wrapped command, so no real scope or Node.js process
# is started.

repo_root="$(git rev-parse --show-toplevel)"
helper="$repo_root/scripts/ci/lib/node-scope.sh"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

fake_bin="$tmp/bin"
mkdir -p "$fake_bin"
cat >"$fake_bin/systemd-run" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "$*" >>"$NODE_SCOPE_TEST_LOG"
while [[ $# -gt 0 && "$1" != "--" ]]; do shift; done
shift
exec "$@"
EOF
chmod +x "$fake_bin/systemd-run"

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

# 1. With a user scope available, the command runs inside a 6G swap-free scope.
log="$tmp/scope.log"
out="$(
  PATH="$fake_bin:$PATH" NODE_SCOPE_TEST_LOG="$log" NILS_CLI_GATE_SLICE='' bash -c '
    source "$1"
    node_scoped printf "%s" ran
  ' bash "$helper"
)"
[[ "$out" == ran ]] || fail "scoped command output: $out"
grep -q -- '--user --scope' "$log" || fail "scope not requested: $(cat "$log")"
grep -q -- 'MemoryMax=6G' "$log" || fail "MemoryMax=6G missing: $(cat "$log")"
grep -q -- 'MemorySwapMax=0' "$log" || fail "MemorySwapMax=0 missing: $(cat "$log")"
grep -q -- '--slice=' "$log" && fail "unexpected slice without a gate: $(cat "$log")"

# 2. Inside a gate, the scope is placed under the gate's slice.
log="$tmp/gate.log"
PATH="$fake_bin:$PATH" NODE_SCOPE_TEST_LOG="$log" NILS_CLI_GATE_SLICE=fixture-gate.slice bash -c '
  source "$1"
  node_scoped true
' bash "$helper"
grep -q -- '--slice=fixture-gate.slice' "$log" || fail "gate slice missing: $(cat "$log")"

# 3. An inherited cache value cannot skip the scope probe.
log="$tmp/ambient.log"
PATH="$fake_bin:$PATH" NODE_SCOPE_TEST_LOG="$log" NILS_CLI_GATE_SLICE='' NILS_CLI_NODE_SCOPE_AVAILABLE=0 bash -c '
  source "$1"
  node_scoped true
' bash "$helper"
grep -q -- 'MemoryMax=6G' "$log" || fail "ambient cache value skipped the scope: $(cat "$log" 2>/dev/null)"

# 4. Outside a gate, a host without a working systemd user scope (for example
# macOS or a hosted runner) runs the command unchanged.
cat >"$fake_bin/systemd-run" <<'EOF'
#!/usr/bin/env bash
exit 42
EOF
chmod +x "$fake_bin/systemd-run"
out="$(
  PATH="$fake_bin:$PATH" NILS_CLI_GATE_SLICE='' bash -c '
    source "$1"
    node_scoped printf "%s" direct
  ' bash "$helper"
)"
[[ "$out" == direct ]] || fail "unscoped fallback output: $out"

# 5. Inside a gate, a scope that cannot be established fails closed: nonzero
# exit, and the payload never runs.
sentinel="$tmp/payload-ran"
status=0
PATH="$fake_bin:$PATH" NILS_CLI_GATE_SLICE=fixture-gate.slice bash -c '
  source "$1"
  node_scoped touch "$2"
' bash "$helper" "$sentinel" 2>"$tmp/fail-closed.err" || status=$?
[[ "$status" -ne 0 ]] || fail "gate scope failure returned success"
[[ ! -e "$sentinel" ]] || fail "payload ran without a scope inside a gate"
grep -q 'node scope' "$tmp/fail-closed.err" || fail "missing fail-closed diagnostic: $(cat "$tmp/fail-closed.err")"

# 6. Every Node.js entry point of the check chain goes through the helper.
grep -q 'node_scoped npx --yes rumdl@' "$repo_root/scripts/ci/markdownlint-audit.sh" ||
  fail "markdownlint-audit.sh does not run npx through node_scoped"
verify="$repo_root/scripts/ci/nils-cli-verification.sh"
for test_file in release-ci-gate.test.cjs peekaboo-release-check.test.cjs; do
  grep -q "^run node_scoped node scripts/ci/tests/$test_file" "$verify" ||
    fail "required checks do not run $test_file through node_scoped"
done

echo "PASS: node scope helper tests"
