# Workspace Shared Crate Boundary v1

## Purpose

This spec records finalized shared-crate boundaries from canonical audit artifacts so extraction ownership stays deterministic across
execution lanes.

Audit inputs:

- `bash scripts/dev/workspace-shared-crate-audit.sh --format tsv`
- `$AGENT_HOME/out/workspace-shared-audit/crate-matrix.tsv`
- `$AGENT_HOME/out/workspace-shared-audit/hotspots-index.tsv`
- `$AGENT_HOME/out/workspace-shared-audit/task-lanes.tsv`

## Boundary Decisions

### What belongs in `nils-common` (`extract`)

- Domain-neutral filesystem primitives (`write_atomic`, `sha256_file`, timestamp persistence, secret timestamp path derivation).
- Provider-runtime path resolution and profile-driven auth-file/secret-dir/cache-dir rules.
- Shared auth persistence substrate that identifies identity-matched secrets and synchronizes auth snapshots without provider-specific UX copy.
- Structured errors that caller crates map back to existing output/exit contracts.

### What stays in `nils-term` (`keep-local`)

- Progress bars/spinners and TTY rendering policy.
- ANSI/presentation behavior that is terminal UX specific rather than runtime-domain neutral.
- CLI surface decisions for progress visibility and one-line rendering modes.

### What stays crate-local (`keep-local` / `defer`)

- Provider-specific warning/error text, JSON envelope wording, and exit-code mapping.
- Parity-sensitive secret-dir UX behavior where Codex/Gemini semantics intentionally diverge (`defer` until characterization proves safe merge).
- Command composition and product-specific policy (auth prompts, sync messaging, diagnostics output).

## Hotspot Lane Matrix

| Theme | Audit signals | Decision | Execution lane ID | Notes |
| --- | --- | --- | --- | --- |
| process/env/no-color primitives | `manual_process_probe`, `manual_env_mutation`, `manual_no_color_logic` | `extract` to `nils-common` with crate-local adapters | `S2T2` | Keep UX text and exit semantics in crate-local adapters. |
| provider auth persistence + atomic fs | `manual_atomic_fs` | `extract` to `nils-common::provider_runtime` substrate | `S2T3` | Shared sync substrate + timestamp path rules; keep provider JSON/text copy local. |
| parity-sensitive secret-dir routing | `manual_secret_dir_resolution` | `keep-local` (`defer` full unification pending explicit parity evidence) | `S2T3` | Do not force full Codex/Gemini secret-dir unification without explicit parity evidence. |
| redundant local wrappers post-extraction | `dependency_present` + wrapper shims | `keep-local` only if still contract-relevant, otherwise delete | `S2T4` | Wrapper removal depends on process/env and provider-runtime substrate landing. |

## Keep/Remove Rules for Runtime Helpers

- `extract` when helper logic is domain-neutral and used by multiple crates.
- `keep-local` when helper exists only to preserve user-visible contract fidelity.
- `defer` when migration risks parity-sensitive behavior without characterization coverage.
- `remove` when no live caller remains after extraction and wrapper no longer provides contract value.
- Current baseline: `crates/codex-cli/src/fs.rs`, `crates/gemini-cli/src/fs.rs`, and `crates/git-cli/src/util.rs` are removed; callers
  consume `nils-common` primitives directly.

## Non-goals

- Moving provider-specific message wording into `nils-common`.
- Merging Codex and Gemini command-level UX into one behavior surface.
- Treating `nils-term` as a generic runtime helper crate.
- Keeping compatibility-only wrappers once shared helpers are canonical.

## Validation

```bash
test -f docs/specs/workspace-shared-crate-boundary-v1.md
bash scripts/dev/workspace-shared-crate-audit.sh --format tsv
rg -n 'What belongs|What stays crate-local|Non-goals' README.md crates/nils-common/README.md crates/nils-test-support/README.md
rg -n 'keep-local|extract|nils-common|nils-term' docs/specs/workspace-shared-crate-boundary-v1.md
```
