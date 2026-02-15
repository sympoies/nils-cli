# New CLI Crate Development Standard

## Purpose
This runbook defines the mandatory standard for adding a new CLI crate in this workspace.

Priority model:
1. Preserve repository CLI quality/parity expectations from `AGENTS.md`.
2. For service-consumed commands, provide a stable, service-consumable JSON contract.
3. Keep the crate publish-ready under current workspace release rules.

## Canonical Sources
Use these as the source of truth to avoid policy drift:

- Global CLI priorities and completion/wrapper expectations:
  - `AGENTS.md`
- Required checks and coverage policy:
  - `DEVELOPMENT.md`
  - `./.agents/skills/nils-cli-checks/scripts/nils-cli-checks.sh`
- Publishing workflow and order:
  - `scripts/publish-crates.sh`
  - `release/crates-io-publish-order.txt`
  - `.agents/skills/nils-cli-release/SKILL.md`
- `agent-docs` command semantics and registration patterns:
  - `crates/agent-docs/README.md`
- JSON contract details:
  - `docs/specs/cli-service-json-contract-guideline-v1.md`

## Applicability
Apply this standard when you add or substantially redesign any CLI crate under `crates/` that is
intended for user/service consumption.

If a crate is intentionally internal-only, keep this standard for UX/testing quality, but mark the
crate explicitly as non-publishable (`publish = false`) and document the reason.

## Required Workflow
1. Create crate scaffold and workspace wiring.
2. Define command contract (flags, exit codes, text output, JSON output).
3. Implement behavior with parity/consistency to workspace conventions.
4. Add tests for both human-readable and JSON contracts.
5. Verify publish-readiness metadata and release order.
6. Run required repository checks before delivery.

## Crate Scaffold Rules
For a new publishable CLI crate:

- `Cargo.toml` must include:
  - `version = "0.3.0"` (or current workspace release version).
  - `edition.workspace = true`
  - `license.workspace = true`
  - `description = "CLI crate for nils-<name> in the nils-cli workspace."`
  - `repository = "https://github.com/graysurf/nils-cli"`
  - at least one `[[bin]]` target.
- Crate must be listed in workspace `members` in root `Cargo.toml`.
- Dependencies should use workspace/shared conventions when available (`[workspace.dependencies]`,
  local `nils-*` crates with explicit `version` + `path` + `package`).
- Add a crate README that documents commands, options, output modes, and dependencies.

For internal-only helper crates:

- Add `publish = false` and explain the reason in README.
- Do not add the crate to `release/crates-io-publish-order.txt`.

## Documentation Placement Rules
Documentation created for a new crate MUST follow `docs/specs/crate-docs-placement-policy.md`.

- Contributors MUST classify each new or updated Markdown file as `workspace-level` or
  `crate-local` before deciding the path.
- `crate-local` docs MUST be placed under `crates/<crate>/docs/...`.
- `crate-local` docs SHOULD use canonical paths:
  - `crates/<crate>/docs/README.md`
  - `crates/<crate>/docs/specs/<topic>.md`
  - `crates/<crate>/docs/runbooks/<topic>.md`
  - `crates/<crate>/docs/reports/<topic>.md`
- Crate-owned docs MUST NOT be added under root `docs/`.

### Workspace-Level Exceptions (Root `docs/` Allowed)
Root `docs/` is an exception path and MUST be used only when the document is `workspace-level`.

Allowed exception types:
- Repository-wide governance or process standards used across the workspace.
- Cross-crate contracts/specifications consumed by multiple crates or external services.
- Shared workspace operations runbooks (release/CI/tooling) not owned by a single crate.

Qualification criteria (both MUST pass):
- Ownership MUST be workspace-owned (not a single crate team/module).
- Scope MUST be cross-crate or repository-governance; otherwise treat the doc as `crate-local`.

## Output Contracts
Every user-facing CLI command surface must have explicit output behavior.

### Human-Readable Contract (Required)
- Default mode should be optimized for terminal use (clear sections/messages).
- `stdout` is reserved for primary command output.
- `stderr` is reserved for warnings/errors/debug/progress.
- Exit codes must be stable and documented.
- Honor `NO_COLOR=1` where colorized output exists.

### JSON Contract (Required For Service-Consumed Commands)
- JSON output must be opt-in (`--json` or `--format json`).
- JSON responses must use a versioned envelope.
- JSON mode must avoid prose-only error signaling.
- JSON payloads must never expose secret/token material.
- Full field-level requirements, examples, compatibility rules, and error envelope schema are
  defined in `docs/specs/cli-service-json-contract-guideline-v1.md`.

### JSON Compatibility Rules
- Additive fields are allowed within the same schema version.
- Renaming/removing required fields is breaking and requires a new schema version.
- Keep old schema behavior available until consumers migrate.
- Contract tests are mandatory for required keys/types and representative failure paths.

## Command and UX Rules
- Use clap-based parsing with stable help text.
- Root CLI parser must include `#[command(version)]` so `-V, --version` is always available.
- Usage errors return `64` unless command-specific legacy contract requires otherwise.
- Keep warning/error prefix conventions consistent with neighboring crates.
- If completion aliases are provided, keep `completions/zsh/` and `completions/bash/` synchronized.

## Testing and Validation Rules
Minimum testing for new CLI crates:

1. Unit tests for core parsing/formatting/edge-case logic.
2. Integration tests for CLI behavior and exit codes.
3. JSON contract tests:
   - required top-level fields.
   - stable error envelope fields.
   - no secret leakage.
4. Completion tests if completions/aliases were changed.

Preferred single entrypoint:

```bash
./.agents/skills/nils-cli-checks/scripts/nils-cli-checks.sh
```

Pre-commit docs placement audit (required):

```bash
bash scripts/ci/docs-placement-audit.sh --strict
```

For exact command set and coverage threshold, follow `DEVELOPMENT.md`.

## Publish Readiness Checklist
Before claiming a new publishable CLI crate is ready:

1. Cargo metadata matches workspace conventions.
2. README exists and includes command/output documentation.
3. Crate appears in root workspace `members`.
4. Publish order file includes the crate at a dependency-safe position:
   - `release/crates-io-publish-order.txt`
5. Publish dry-run succeeds:

```bash
scripts/publish-crates.sh --dry-run --crate <crate-package-name>
```

If crate is non-publishable (`publish = false`), verify it is excluded from publish order.

## Agent-Docs Integration
This document should be required in `project-dev` context.

Use the standard `resolve + add` pattern from `crates/agent-docs/README.md`.
Project registration for this document:

```bash
agent-docs add \
  --target project \
  --context project-dev \
  --scope project \
  --path docs/runbooks/new-cli-crate-development-standard.md \
  --required \
  --when always \
  --notes "New CLI crate standard (human output + JSON contract + publish-ready)"
```

Then verify strict resolve:

```bash
agent-docs resolve --context project-dev --strict --format checklist
```

## Review Checklist (PR Gate)
- [ ] Human-readable output behavior is documented and tested.
- [ ] JSON contract is versioned, documented, and tested.
- [ ] Error envelope is machine-consumable in JSON mode.
- [ ] No sensitive fields leak in JSON output.
- [ ] Publish-readiness items are complete (or crate is explicitly internal-only).
- [ ] Required repository checks pass.
