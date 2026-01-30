---
name: port-cli-to-rust
description: Port an existing Zsh CLI into a Rust crate in this workspace (rigorous plan + parallel execution + edge-case tests + pre-commit checks).
---

# Port CLI to Rust (nils-cli)

## Contract

Prereqs:

- You are in the `nils-cli` git work tree.
- The user can point to the source CLI implementation (usually a Zsh script) and any related docs/completions.
- You can run Rust + Zsh tooling locally:
  - `cargo` (with `rustfmt` + `clippy`)
  - `zsh`
- You can spawn/coordinate subagents (for parallel task execution).

Inputs:

- The CLI to port:
  - crate/bin name (kebab-case; matches the binary name)
  - source script path (e.g. `~/.config/zsh/scripts/.../<cli>.zsh`)
  - optional: completion script path and wrapper aliases to preserve
- Constraints:
  - “behavioral parity” requirements (output text, emojis, colors, exit codes, degradation paths)
  - any explicit out-of-scope items

Outputs:

- A rigorous plan file: `docs/plans/<cli>-rust-port-plan.md` (sprints + atomic tasks + dependencies + complexity + validation commands).
- Ported Rust crate: `crates/<cli>/...` and workspace wiring in `Cargo.toml`.
- Parity docs:
  - `docs/<cli>/spec.md`
  - `docs/<cli>/fixtures.md`
- Comprehensive tests covering core flows + edge cases:
  - integration tests under `crates/<cli>/tests/` (use deterministic fixtures, PATH stubs, and temp git repos as needed)
  - zsh completion tests when applicable
- Pre-delivery validation passes via:
  - `./skills/tools/testing/nils-cli-checks/scripts/nils-cli-checks.sh`
- If requested, a commit using the repo policy (`$semantic-commit` / `$semantic-commit-autostage`).

Exit codes:

- N/A (conversation/workflow skill).

Failure modes:

- Source script/docs cannot be accessed, or the request is underspecified and the user won’t confirm assumptions.
- The Zsh CLI depends on parent-shell mutation that can’t be replicated in a child process (must document wrapper-based workarounds, like `fzf-cli`).
- Tests cannot be made deterministic without stubbing external tools (resolve by PATH-stubbing, as in `crates/fzf-cli/tests/common.rs`).
- `nils-cli-checks` fails due to lint/test failures; must fix or report the blocking error and why it can’t be resolved.

## Workflow

1) Clarify minimal inputs (only if needed)

- Use the question format from:
  - `$CODEX_HOME/skills/workflows/conversation/ask-questions-if-underspecified/SKILL.md`
- Must confirm:
  - exact CLI name (binary + crate)
  - source script location
  - required subcommands/flags and exact output contract requirements
  - whether to port wrappers/completions

2) Study existing in-repo ports (patterns to copy)

- Plans:
  - `docs/plans/git-scope-rust-port-plan.md`
  - `docs/plans/fzf-cli-rust-port-plan.md`
- Parity docs:
  - `docs/git-scope/spec.md`, `docs/git-scope/fixtures.md`
  - `docs/fzf-cli/spec.md`, `docs/fzf-cli/fixtures.md`
- Test patterns:
  - `crates/git-scope/tests/common.rs` (temp git repo + NO_COLOR for stable output)
  - `crates/fzf-cli/tests/common.rs` (PATH-stubbing external tools)

3) Produce the rigorous plan (do not implement yet)

- Use: `$create-plan-rigorous`
- Output path must be: `docs/plans/<cli>-rust-port-plan.md`
- Plan requirements:
  - each task has `Location`, `Description`, `Dependencies`, `Complexity`, `Acceptance criteria`, `Validation`
  - explicitly call out parallelizable tasks (docs/tests/scaffolding) vs tasks that must be sequential
  - include “Testing Strategy”, “Risks & gotchas”, and a plausible “Rollback plan”
- Lint plan until it passes:
  - `$CODEX_HOME/skills/workflows/plan/plan-tooling/scripts/validate_plans.sh --file docs/plans/<cli>-rust-port-plan.md`
- Run a subagent plan review (required by `$create-plan-rigorous`) and incorporate fixes.

4) Execute the plan in parallel (default: Sprint 1, then iterate)

- Use: `$execute-plan-parallel`
- Invocation format:
  - `/execute-plan-parallel docs/plans/<cli>-rust-port-plan.md sprint <n>`
- Parallelization rules:
  - Only parallelize tasks that don’t overlap heavily in the same files.
  - Keep tasks atomic; merge early and often to avoid drift.
  - Require each subagent to report: files changed, acceptance criteria status, and what validation ran.

5) Implementation guidelines (parity-first)

- Prefer behavioral parity over refactors/UX changes:
  - output structure, headings, emojis, warnings, and exit codes must match the source script/spec.
  - support `--no-color` and `NO_COLOR` where applicable (ensure snapshot/stable test output).
- Degradation paths must match the script:
  - missing optional tools (e.g. `tree`, `file`, `fzf`) must print the same warnings and choose the same fallback behavior.
- If the CLI needs “shell effects” (cd/eval/alias):
  - emit a safe stdout contract (e.g. `cd <dir>`), document it in `docs/<cli>/spec.md`, and add a wrapper snippet under `wrappers/` if needed.

6) Tests: required edge-case coverage

- Every subcommand/flag must have at least one deterministic integration test.
- Add an explicit `edge_cases.rs` (or equivalent) covering:
  - unknown commands / invalid flags (exit code + stderr)
  - missing optional external tools on `PATH`
  - empty selections / aborted prompts (for interactive pickers)
  - NO_COLOR / --no-color output stability
  - git edge cases when applicable (no repo, empty repo, merge commits, parent selection, binary files)
- Prefer the in-repo patterns:
  - temp git repos (`tempfile`) + real `git` (like `crates/git-scope/tests/common.rs`)
  - PATH-stubbing and scripted fakes for interactive tools (`crates/fzf-cli/tests/common.rs`)

7) Pre-delivery checks (must pass before committing)

- Run:
  - `./skills/tools/testing/nils-cli-checks/scripts/nils-cli-checks.sh`
- If any check fails:
  - fix within scope, re-run, and only proceed once it exits `0`
  - if blocked, report the exact failing command + key error output + why it can’t be resolved

8) Commit (only after checks are green)

- Follow repo policy (do not run `git commit` directly):
  - use `$semantic-commit` (user staged) or `$semantic-commit-autostage` (Codex-owned change set)

