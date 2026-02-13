# Crate Docs Placement Policy

## Purpose
This policy defines where documentation MUST live in the `nils-cli` workspace so contributors can
add docs without ownership drift.

## Scope
- Applies to contributor-authored Markdown docs (`*.md`) in repository root, `docs/`, and
  `crates/*/docs/`.
- Does not govern Markdown test fixtures or embedded prompt/template assets under non-docs
  directories.

## Ownership Model
- `workspace-level`: documentation owned by the whole repository, used across multiple crates, or
  defining shared governance/process.
- `crate-local`: documentation owned by exactly one crate and primarily describing that crate's
  behavior, contracts, or operations.

Contributors MUST classify each new/updated documentation file into one of these two ownership
types before choosing a path.

## Allowed Root Docs
Only the following root-level documentation categories are allowed as canonical sources:

- `/README.md` (workspace overview)
- `/DEVELOPMENT.md` (workspace required checks and developer workflow)
- `/AGENTS.md` (agent behavior policy)
- `/BINARY_DEPENDENCIES.md` (workspace shared binary prerequisites)
- `/docs/plans/*.md` (workspace planning documents)
- `/docs/specs/*.md` for `workspace-level` specifications only
- `/docs/runbooks/*.md` for `workspace-level` runbooks only

Any new root `docs/` file MUST be `workspace-level`.

## Disallowed Root Docs
The following are disallowed as canonical docs at repository root:

- `crate-local` runbooks/specs/reports under `/docs/**`
- Crate-owned files named like `docs/runbooks/<crate>-*.md` or `docs/specs/<crate>-*.md`
- Any crate-owned documentation placed directly under `/docs/` because it is "temporarily easier"

If a historical root path must remain for compatibility, it MUST be a short stub that points to the
`canonical` crate-local path and MUST NOT duplicate full canonical content.

## Compatibility Stub Lifecycle Decision
Compatibility stubs under root `docs/` are permanent redirects (no deprecation sunset date planned).

- Stubs MUST keep a `Moved to:` target and migration metadata.
- Stubs MUST remain redirect-only shims and MUST NOT carry canonical runbook/spec/report content.
- If governance changes later, the policy update MUST explicitly document a new sunset decision first.

## Canonical Crate-Local Paths
`crate-local` documentation MUST live under `crates/<crate>/docs/`.

Canonical structure:

```text
crates/<crate>/docs/README.md
crates/<crate>/docs/specs/<topic>.md
crates/<crate>/docs/runbooks/<topic>.md
crates/<crate>/docs/reports/<topic>.md
```

Current repository examples:

- `crates/memo-cli/docs/specs/memo-cli-json-contract-v1.md`
- `crates/memo-cli/docs/runbooks/memo-cli-rollout.md`
- `crates/codex-cli/docs/specs/codex-cli-diag-auth-json-contract-v1.md`

## New Documentation Contributor Requirements
When adding or moving docs, contributors MUST:

1. Classify ownership (`workspace-level` vs `crate-local`) before creating the file.
2. Place `crate-local` docs in `crates/<crate>/docs/...` using canonical paths.
3. Keep root `docs/` paths for `workspace-level` docs only.
4. Update references so README/runbooks/specs point to canonical locations.
5. For moved root docs, leave only compatibility stubs with a `Moved to` pointer.

Contributors SHOULD:

- Keep filenames deterministic and topic-focused (`<topic>.md`, version suffix only when needed).
- Avoid creating new root docs when an existing workspace-level document can be extended.

## Enforcement Reference
`DEVELOPMENT.md` required checks reference this policy. Future automation and CI checks MUST enforce
the same placement rules.
