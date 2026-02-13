# Crate Doc Migration Inventory

Last updated: 2026-02-13  
Policy source: `docs/specs/crate-docs-placement-policy.md`

This report is the workspace migration checklist for classifying root docs and moving crate-owned docs to canonical crate-local paths.

## Checklist Status Legend

- `[ ]` Not started
- `[~]` In progress
- `[x]` Completed
- `Retained` = workspace-level canonical path stays under root `docs/`.

## Root Docs Classification (Policy-Aligned)

| Checklist | Root docs path | Root docs classification | Owner crate | Final canonical path | Risk level | Migration order | Status |
|---|---|---|---|---|---|---|---|
| [x] | `docs/specs/crate-docs-placement-policy.md` | workspace-level spec | `workspace` | `docs/specs/crate-docs-placement-policy.md` | Low | - | Retained |
| [x] | `docs/specs/cli-service-json-contract-guideline-v1.md` | workspace-level spec | `workspace` | `docs/specs/cli-service-json-contract-guideline-v1.md` | Low | - | Retained |
| [x] | `docs/runbooks/new-cli-crate-development-standard.md` | workspace-level runbook | `workspace` | `docs/runbooks/new-cli-crate-development-standard.md` | Low | - | Retained |
| [x] | `docs/runbooks/provider-onboarding.md` | workspace-level runbook | `workspace` | `docs/runbooks/provider-onboarding.md` | Low | - | Retained |
| [x] | `docs/runbooks/crates-io-status-script-runbook.md` | workspace-level runbook | `workspace` | `docs/runbooks/crates-io-status-script-runbook.md` | Low | - | Retained |
| [x] | `docs/runbooks/wrappers-mode-usage.md` | workspace-level runbook | `workspace` | `docs/runbooks/wrappers-mode-usage.md` | Low | - | Retained |
| [x] | `docs/plans/*.md` | workspace-level planning docs | `workspace` | `docs/plans/*.md` | Low | - | Retained |
| [x] | `docs/runbooks/codex-cli-json-consumers.md` | crate-local runbook compatibility stub (root canonical disallowed) | `codex-cli` | `crates/codex-cli/docs/runbooks/json-consumers.md` | Medium | 1 | Completed (redirect stub) |
| [x] | `docs/runbooks/image-processing-llm-svg.md` | crate-local runbook compatibility stub (root canonical disallowed) | `image-processing` | `crates/image-processing/docs/runbooks/llm-svg-workflow.md` | High | 2 | Completed (redirect stub) |

## Migrated Candidates (Execution Checklist)

| Checklist | Current path | Owner crate | Final canonical path | Risk level | Migration order | Status | Notes |
|---|---|---|---|---|---|---|---|
| [x] | `docs/runbooks/codex-cli-json-consumers.md` | `codex-cli` | `crates/codex-cli/docs/runbooks/json-consumers.md` | Medium | 1 | Completed | Root path retained as compatibility redirect stub (`Moved to ...`), high-impact references updated. |
| [x] | `docs/runbooks/image-processing-llm-svg.md` | `image-processing` | `crates/image-processing/docs/runbooks/llm-svg-workflow.md` | High | 2 | Completed | Root path retained as compatibility redirect stub (`Moved to ...`), high-impact references updated. |

## Retained Workspace-Level Docs

| Checklist | Path | Why retained at root docs | Status |
|---|---|---|---|
| [x] | `docs/specs/crate-docs-placement-policy.md` | Governance spec for whole workspace | Retained |
| [x] | `docs/specs/cli-service-json-contract-guideline-v1.md` | Shared JSON contract guideline across CLIs/services | Retained |
| [x] | `docs/runbooks/new-cli-crate-development-standard.md` | Workspace contributor/development standard | Retained |
| [x] | `docs/runbooks/provider-onboarding.md` | Cross-crate provider onboarding process | Retained |
| [x] | `docs/runbooks/crates-io-status-script-runbook.md` | Workspace publish-status operation runbook | Retained |
| [x] | `docs/runbooks/wrappers-mode-usage.md` | Cross-wrapper execution-mode behavior for many crates | Retained |
| [x] | `docs/plans/*.md` | Workspace planning artifacts | Retained |

## Crate Docs Baseline Gap Summary

Current baseline snapshot:
- Workspace crates total: `26`
- `crates/*/docs/README.md` present: `26`
- Crates with `crates/<crate>/docs/` subtree: `26`

Priority checklist for baseline gap closure:

| Checklist | Baseline gap item | Owner crate | Final canonical path | Risk level | Migration order | Status |
|---|---|---|---|---|---|---|
| [x] | Create crate docs index needed before migration Task 2.1 | `codex-cli` | `crates/codex-cli/docs/README.md` | High | P0-1 | Completed |
| [x] | Create crate docs index + runbooks dir needed before migration Task 2.2 | `image-processing` | `crates/image-processing/docs/README.md` | High | P0-2 | Completed |
| [x] | Existing crate docs subtree lacked index | `memo-cli` | `crates/memo-cli/docs/README.md` | Medium | P1-1 | Completed |
| [x] | Add missing crate docs index for remaining crates | `agent-docs`, `agent-provider-claude`, `agent-provider-codex`, `agent-provider-gemini`, `agent-runtime-core`, `agentctl`, `api-gql`, `api-rest`, `api-test`, `api-testing-core`, `cli-template`, `fzf-cli`, `git-cli`, `git-lock`, `git-scope`, `git-summary`, `macos-agent`, `nils-common`, `nils-term`, `nils-test-support`, `plan-tooling`, `screen-record`, `semantic-commit` | `crates/<crate>/docs/README.md` | Medium | P1-2 | Completed |

## High-Impact Reference Hotspots (Post-Move Update Queue)

| Checklist | File | Status |
|---|---|---|
| [x] | `crates/codex-cli/README.md` | Updated to canonical crate-local path |
| [x] | `docs/plans/codex-cli-service-consumable-diag-auth-plan.md` | Updated to canonical crate-local path |
| [x] | `docs/plans/codex-auth-login-save-plan.md` | Updated to canonical crate-local path |
| [x] | `docs/plans/image-processing-from-svg-llm-tooling-migration-plan.md` | Updated to canonical crate-local path |

## Compatibility Stub Lifecycle Status

Decision: compatibility stubs at root `docs/runbooks/` are permanent redirects (no sunset date planned).

| Checklist | Legacy root path | Stub status | Redirect target | Deprecation/Sunset note |
|---|---|---|---|---|
| [x] | `docs/runbooks/codex-cli-json-consumers.md` | Redirect stub active | `crates/codex-cli/docs/runbooks/json-consumers.md` | No deprecation sunset scheduled; keep as redirect-only shim. |
| [x] | `docs/runbooks/image-processing-llm-svg.md` | Redirect stub active | `crates/image-processing/docs/runbooks/llm-svg-workflow.md` | No deprecation sunset scheduled; keep as redirect-only shim. |

## Policy Consistency Notes

- Root `docs/` is workspace-level by default; crate-owned canonical content lives under `crates/<crate>/docs/...`.
- Compatibility stubs are redirect-only files and MUST NOT duplicate canonical runbook/spec/report content.
- `scripts/ci/docs-placement-audit.sh --strict` enforces docs index presence and root runbook placement governance.
