# Crate Doc Migration Summary

Date: 2026-02-13

## Overview

This workspace finished the crate-doc migration governance plan by moving crate-owned canonical docs into `crates/<crate>/docs/...`, preserving root compatibility stubs as redirect-only shims, and enforcing placement in local/CI checks.

## Before / After Examples

This section captures concrete before and after path mappings used by maintainers.

- Before: `docs/runbooks/codex-cli-json-consumers.md` (canonical content at root)
- After: `crates/codex-cli/docs/runbooks/json-consumers.md` (canonical), with root redirect stub

- Before: `docs/runbooks/image-processing-llm-svg.md` (canonical content at root)
- After: `crates/image-processing/docs/runbooks/llm-svg-workflow.md` (canonical), with root redirect stub

- Before: missing crate docs indexes across workspace crates
- After: every crate has `crates/<crate>/docs/README.md` and crate README link to `docs/README.md`

## Enforcement Model

- Policy: `docs/specs/crate-docs-placement-policy.md`
- Inventory/checklist: `docs/reports/crate-doc-migration-inventory.md`
- Automation: `scripts/ci/docs-placement-audit.sh --strict`
- Required docs context: `AGENT_DOCS.toml` (`project-dev` requires `crate-docs-placement-policy.md`)
- Pipeline integration: `./.codex/skills/nils-cli-checks/scripts/nils-cli-checks.sh` and `.github/workflows/ci.yml`

## Contributor Required Steps (Pre-Commit)

Mandatory pre-commit checks for docs placement:

```bash
bash scripts/ci/docs-placement-audit.sh --strict
./.codex/skills/nils-cli-checks/scripts/nils-cli-checks.sh
```

When adding or moving Markdown docs:

1. Classify the file as `workspace-level` or `crate-local`.
2. Put crate-local canonical docs under `crates/<crate>/docs/...`.
3. Update the crate docs index (`crates/<crate>/docs/README.md`) and relevant README links.
4. If preserving a legacy root path, keep only a short redirect stub (`Moved to:`).

## Maintainer Notes

- Root compatibility stubs are treated as permanent redirect files unless policy is explicitly revised.
- Any new root `docs/runbooks/*.md` file that is not an approved workspace-level runbook or stub should fail strict audit.
