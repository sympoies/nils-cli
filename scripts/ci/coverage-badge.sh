#!/usr/bin/env bash
set -euo pipefail

lcov_path="${1:-target/coverage/lcov.info}"
out_svg="${2:-badges/coverage.svg}"

if [[ ! -f "$lcov_path" ]]; then
  echo "error: missing LCOV file: $lcov_path" >&2
  exit 1
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

color="$(
  awk -v p="$percent" 'BEGIN {
    if (p >= 90) print "#4c1";
    else if (p >= 80) print "#97CA00";
    else if (p >= 70) print "#a4a61d";
    else if (p >= 60) print "#dfb317";
    else if (p >= 50) print "#fe7d37";
    else print "#e05d44";
  }'
)"

label="coverage"
message="${percent}%"

label_width=62
message_width=58
total_width=$((label_width + message_width))
label_x=$((label_width / 2))
message_x=$((label_width + (message_width / 2)))

mkdir -p "$(dirname "$out_svg")"

cat >"$out_svg" <<EOF
<svg xmlns="http://www.w3.org/2000/svg" width="$total_width" height="20" role="img" aria-label="$label: $message">
  <linearGradient id="s" x2="0" y2="100%">
    <stop offset="0" stop-color="#bbb" stop-opacity=".1"/>
    <stop offset="1" stop-opacity=".1"/>
  </linearGradient>
  <clipPath id="r">
    <rect width="$total_width" height="20" rx="3" fill="#fff"/>
  </clipPath>
  <g clip-path="url(#r)">
    <rect width="$label_width" height="20" fill="#555"/>
    <rect x="$label_width" width="$message_width" height="20" fill="$color"/>
    <rect width="$total_width" height="20" fill="url(#s)"/>
  </g>
  <g fill="#fff" text-anchor="middle" font-family="Verdana,Geneva,DejaVu Sans,sans-serif" font-size="11">
    <text x="$label_x" y="15" fill="#010101" fill-opacity=".3">$label</text>
    <text x="$label_x" y="14">$label</text>
    <text x="$message_x" y="15" fill="#010101" fill-opacity=".3">$message</text>
    <text x="$message_x" y="14">$message</text>
  </g>
</svg>
EOF

echo "$out_svg"
