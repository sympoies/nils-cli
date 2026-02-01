#!/usr/bin/env bash
set -euo pipefail

lcov_path="${1:-target/coverage/lcov.info}"
summary_path="${GITHUB_STEP_SUMMARY:-/dev/stdout}"

write_summary() {
  cat >>"$summary_path"
}

if [[ ! -f "$lcov_path" ]]; then
  write_summary <<EOF
## Coverage

Coverage summary unavailable (missing \`$lcov_path\`).
EOF
  exit 0
fi

read -r total_lh total_lf < <(
  awk -F: '
    $1 == "LH" { lh += $2 }
    $1 == "LF" { lf += $2 }
    END { printf "%d %d\n", lh, lf }
  ' "$lcov_path"
)

percent="$(
  awk -v lh="$total_lh" -v lf="$total_lf" 'BEGIN {
    if (lf == 0) {
      print "0.00"
      exit
    }
    printf "%.2f", (lh / lf) * 100
  }'
)"

write_summary <<EOF
## Coverage

Total line coverage: **$percent%** ($total_lh/$total_lf lines hit).
EOF
