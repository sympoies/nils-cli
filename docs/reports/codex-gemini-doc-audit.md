# codex/gemini migration doc audit

## Scope
Audit of codex/gemini migration-adjacent docs after consolidating runtime ownership into CLI adapters plus `nils-common::provider_runtime`.

## Keep/Merge/Remove map

| Path | Decision | Canonical target | Rationale |
| --- | --- | --- | --- |
| `docs/specs/codex-gemini-runtime-contract.md` | keep | self | Runtime provider matrix and compatibility rules for adapter wiring. |
| `docs/specs/codex-gemini-cli-parity-contract-v1.md` | keep | self | Canonical parity contract for both CLI lanes. |
| `crates/codex-cli/docs/specs/codex-cli-diag-auth-json-contract-v1.md` | keep | self | Codex service-consumer JSON contract is lane-specific and must remain separate. |
| `crates/gemini-cli/docs/specs/gemini-cli-diag-auth-json-contract-v1.md` | keep | self | Gemini service-consumer JSON contract is lane-specific and must remain separate. |
| `crates/codex-cli/docs/runbooks/json-consumers.md` | keep | self | Codex consumer usage remains lane-specific. |
| `crates/gemini-cli/docs/runbooks/json-consumers.md` | keep | self | Gemini consumer usage remains lane-specific. |
| `docs/runbooks/codex-core-migration.md` | remove | n/a | Root runbook path is not allowed by docs placement policy; migration ownership moved to canonical specs/reports docs. |
| `README.md` codex/gemini core references | merge | workspace crate list + parity contract link | Root workspace layout now reflects merged architecture. |
| `crates/codex-cli/README.md` core ownership text | merge | `nils-common::provider_runtime` + crate adapters | Updated ownership model after core crate removal. |
| `crates/gemini-cli/README.md` core ownership text | merge | `nils-common::provider_runtime` + crate adapters | Updated ownership model after core crate removal. |
| `release/crates-io-publish-order.txt` entry `nils-gemini-core` | remove | n/a | Removed unpublished-forward core crate from release order. |
| `crates/codex-core/docs/**` | remove | n/a | Crate removed from workspace; docs no longer canonical. |
| `crates/gemini-core/docs/**` | remove | n/a | Crate removed from workspace; docs no longer canonical. |

## Broken redirect check

- Previous stale redirect detected:
  - `docs/runbooks/codex-core-migration.md` used `Moved to: crates/codex-core/docs/runbooks/codex-core-migration.md`.
- Action:
  - Removed the root runbook and retained canonical migration records in `docs/specs/` and `docs/reports/`.

## Canonical survivors

- Runtime contract: `docs/specs/codex-gemini-runtime-contract.md`
- Parity contract: `docs/specs/codex-gemini-cli-parity-contract-v1.md`
