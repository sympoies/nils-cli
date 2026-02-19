#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
skill_root="$(cd "${script_dir}/.." && pwd)"
entrypoint="${skill_root}/scripts/nils-cli-docs-plan-cleanup.sh"

if [[ ! -f "${skill_root}/SKILL.md" ]]; then
  echo "error: missing SKILL.md" >&2
  exit 1
fi
if [[ ! -f "$entrypoint" ]]; then
  echo "error: missing scripts/nils-cli-docs-plan-cleanup.sh" >&2
  exit 1
fi

bash "$entrypoint" --help >/dev/null

tmp_root="$(mktemp -d)"
trap 'rm -rf "$tmp_root"' EXIT

setup_repo() {
  local repo="$1"
  mkdir -p "$repo"
  (
    cd "$repo"
    git init >/dev/null
    mkdir -p docs/plans docs/reports docs/specs docs/runbooks

    cat > docs/plans/a-plan.md <<'EOF'
# A plan
EOF
    cat > docs/plans/b-plan.md <<'EOF'
# B plan
EOF
    cat > docs/reports/a-summary.md <<'EOF'
Plan source: docs/plans/a-plan.md
EOF
    cat > docs/reports/retained.md <<'EOF'
Retain me because another markdown file references this doc.
Source: docs/plans/a-plan.md
EOF
    cat > docs/reports/mixed.md <<'EOF'
Links:
- docs/plans/a-plan.md
- docs/plans/b-plan.md
EOF
    cat > docs/specs/a-policy.md <<'EOF'
Policy derived from docs/plans/a-plan.md
EOF
    cat > README.md <<'EOF'
Migration note: docs/plans/a-plan.md
EOF
    cat > docs/runbooks/report-consumer.md <<'EOF'
Uses docs/reports/retained.md during validation.
EOF
  )
}

repo_one="${tmp_root}/repo-one"
setup_repo "$repo_one"

cat > "${repo_one}/keep.txt" <<'EOF'
# preserve this one
b-plan
EOF

dry_run_output="$(bash "$entrypoint" --project-path "$repo_one" --keep-plans-file "${repo_one}/keep.txt")"
echo "$dry_run_output" | grep -Fq "=== docs-plan-cleanup-report:v1 ==="
echo "$dry_run_output" | grep -Fq "[plan_md_to_clean]"
echo "$dry_run_output" | grep -Fq "count: 1"
echo "$dry_run_output" | grep -Fq -- "- docs/plans/a-plan.md"
echo "$dry_run_output" | grep -Fq "[plan_related_md_to_clean]"
echo "$dry_run_output" | grep -Fq -- "- docs/reports/a-summary.md"
echo "$dry_run_output" | grep -Fq "[plan_related_md_kept_referenced_elsewhere]"
echo "$dry_run_output" | grep -Fq -- "- docs/reports/retained.md"
echo "$dry_run_output" | grep -Fq "referenced_by: docs/runbooks/report-consumer.md"
echo "$dry_run_output" | grep -Fq "[plan_related_md_to_rehome]"
echo "$dry_run_output" | grep -Fq -- "- docs/specs/a-policy.md"
echo "$dry_run_output" | grep -Fq "[plan_related_md_manual_review]"
echo "$dry_run_output" | grep -Fq -- "- docs/reports/mixed.md"
echo "$dry_run_output" | grep -Fq "[non_docs_md_referencing_removed_plan]"
echo "$dry_run_output" | grep -Fq -- "- README.md"
echo "$dry_run_output" | grep -Fq "[execution]"
echo "$dry_run_output" | grep -Fq "status: skipped (dry-run)"

bash "$entrypoint" --project-path "$repo_one" --keep-plan b-plan --execute >/dev/null
[[ ! -f "${repo_one}/docs/plans/a-plan.md" ]]
[[ -f "${repo_one}/docs/plans/b-plan.md" ]]
[[ ! -f "${repo_one}/docs/reports/a-summary.md" ]]
[[ -f "${repo_one}/docs/reports/retained.md" ]]
[[ -f "${repo_one}/docs/reports/mixed.md" ]]
[[ -f "${repo_one}/docs/specs/a-policy.md" ]]
[[ -f "${repo_one}/docs/runbooks/report-consumer.md" ]]

repo_two="${tmp_root}/repo-two"
setup_repo "$repo_two"

bash "$entrypoint" --project-path "$repo_two" --keep-plan b-plan --execute --delete-important >/dev/null
[[ ! -f "${repo_two}/docs/specs/a-policy.md" ]]
[[ -f "${repo_two}/docs/reports/retained.md" ]]

echo "ok: project skill tests passed"
