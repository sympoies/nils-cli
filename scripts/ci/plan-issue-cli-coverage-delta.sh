#!/usr/bin/env bash
set -euo pipefail

baseline="${PLAN_ISSUE_COVERAGE_BASELINE:-72.78}"
minimum="${PLAN_ISSUE_COVERAGE_MINIMUM:-73.28}"
agent_home="${AGENT_HOME:-$HOME/.agents}"
out_dir="${agent_home}/out/plan-issue-cli-coverage"
summary_path="${out_dir}/summary.txt"

mkdir -p "${out_dir}"

cargo llvm-cov --package nils-plan-issue-cli --summary-only | tee "${summary_path}"

python3 - "${summary_path}" "${baseline}" "${minimum}" <<'PY' | tee -a "${summary_path}"
import pathlib
import re
import sys

summary_path = pathlib.Path(sys.argv[1])
baseline = float(sys.argv[2])
minimum = float(sys.argv[3])
text = summary_path.read_text(encoding="utf-8")
match = re.search(
    r"^TOTAL\s+\d+\s+\d+\s+[0-9.]+%\s+\d+\s+\d+\s+[0-9.]+%\s+\d+\s+\d+\s+([0-9.]+)%",
    text,
    re.MULTILINE,
)
if not match:
    raise SystemExit(f"unable to parse TOTAL line coverage from {summary_path}")

final = float(match.group(1))
delta = final - baseline

print(f"coverage baseline: {baseline:.2f}%")
print(f"coverage final: {final:.2f}%")
print(f"coverage delta: {delta:.2f}pp")
print(f"coverage minimum: {minimum:.2f}%")

if final < minimum:
    raise SystemExit(
        f"line coverage {final:.2f}% is below required {minimum:.2f}%"
    )
print(f"line coverage gate passed: {final:.2f}%")
PY
