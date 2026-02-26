# plan-issue-cli docs

## Purpose

Crate-local documentation for `nils-plan-issue-cli`.

`Task Decomposition` is the crate's documented runtime-truth execution table for plan/sprint orchestration. Specs define `Owner` as a
dispatch alias, document `group + auto` single-lane normalization to `per-sprint`, and treat task-spec/subagent prompts as derived artifacts
(not a second issue-body dispatch table). `start-sprint` validates drift against plan-derived lanes and does not rewrite issue rows in
runtime-truth mode.

Current runtime ownership:

- `plan-tooling split-prs` emits grouping primitives only.
- `plan-issue-cli` materializes runtime `Owner/Branch/Worktree/Notes` metadata from plan content, grouping results, and prefixes.
- markdown table cell canonicalization is provided by `nils-common::markdown`.

## Specs

- [plan-issue CLI contract v2](specs/plan-issue-cli-contract-v2.md)
- [plan-issue CLI contract v1](specs/plan-issue-cli-contract-v1.md)
- [plan-issue state machine and gates v1](specs/plan-issue-state-machine-v1.md)
- [plan-issue gate matrix v1](specs/plan-issue-gate-matrix-v1.md)
