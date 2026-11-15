#!/usr/bin/env bash
# Nextest invokes this once per test attempt. The index is kept outside TMPDIR
# so it is not itself considered a leak and never contains environment values.
set -euo pipefail
test_tmp="$(mktemp -d "$TEMP_PROBE_STATE/tmp/test.XXXXXXXX")"
id="${test_tmp##*/}"
printf '%s\n%s\n' "${1##*/}" "${NEXTEST_TEST_NAME:-unknown}" > "$TEMP_PROBE_STATE/index/$id"
export TMPDIR="$test_tmp"
exec "$@"
