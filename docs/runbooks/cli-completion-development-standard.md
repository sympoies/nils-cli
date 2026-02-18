# CLI Completion Development Standard

## Purpose
This runbook defines the workspace standard for CLI completion architecture, implementation boundaries, migration safety, and validation gates.

## Canonical Sources
- Global completion obligations and alias families:
  - `AGENTS.md`
- Required checks and coverage policy:
  - `DEVELOPMENT.md`
  - `./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh`
- Workspace completion assets and tests:
  - `completions/zsh/`
  - `completions/bash/`
  - `tests/zsh/completion.test.zsh`
- Per-CLI migration contract template:
  - `docs/specs/completion-contract-template.md`
- New crate implementation standard:
  - `docs/runbooks/new-cli-crate-development-standard.md`

## Completion Quality Contract
For every completion-required CLI, baseline completion quality must include:

1. subcommands and nested subcommands
2. long and short flags
3. declared value candidates where available (for example enum-like values)
4. context-aware filtering by cursor position (avoid global "show everything" candidate dumps)

This matches modern Rust CLI expectations (e.g., ripgrep/bat/fd style behavior):
- zsh usually shows richer descriptions
- bash usually shows more compact candidates

## Canonical Completion Architecture (Clap-First)
Every completion-required CLI must follow this layered architecture:

1. clap command model as source of truth (`Parser`/`Subcommand`/`Args`)
2. `clap_complete` generated completion baseline for zsh/bash
3. thin repo adapters only for alias wiring and optional dynamic extensions

### Clap command model contract
The CLI crate must keep completion-relevant metadata in clap definitions:

- command tree and flags from clap types
- value candidates via `ValueEnum` / `PossibleValue` / parser constraints
- path/command hints via `ValueHint` when applicable

Do not keep command/flag truth in shell scripts.

### Generation and export contract
Each completion-required CLI must expose a user-facing export path for completions, for example:

- `<cli> completion <shell>`
- `<cli> --generate-completion <shell>`

Implementation should be powered by `clap_complete` generation APIs.

Repository distribution contract:

- generated zsh assets live in `completions/zsh/_<cli>`
- generated bash assets live in `completions/bash/<cli>`
- generated outputs must be deterministic and committed alongside CLI surface changes

### Thin adapter and dynamic extension contract
Shell adapters may only:

- register completion for canonical command names and required aliases
- apply minor shell-specific formatting/wiring
- apply deterministic fail-closed behavior when generated completion cannot be loaded

Dynamic/runtime value completion is optional and must extend (not replace) clap-generated baseline:

- preferred dynamic path: `clap_complete::env::CompleteEnv`
- alternative dynamic path (when needed): crate-local hidden completion command (e.g., `__complete`)

### Shared adapter helper contract
Workspace adapters may share common helper scripts for zsh/bash to reduce duplicated loader and registration logic while preserving thin adapter behavior.

Canonical helper files:

- `completions/zsh/_completion-adapter-common.zsh`
- `completions/bash/completion-adapter-common.bash`

Helper contract:

- generated loading helper: fetches `<cli> completion <shell>`, renames generated function symbols per adapter, strips known self-registration blocks when needed, caches success/failure state, and validates that the generated function is callable
- registration helper: registers one completion entrypoint for canonical command and alias names (`compdef`/`complete`) without mutating alias definitions
- no-legacy helper: enforces fail-closed behavior on generated-load failure (empty/no candidates) and does not route to any legacy completion stack

Adapter integration requirements:

- adapters stay responsible for CLI-specific alias rewrite semantics (for example `cxgp -> agent prompt`)
- adapters may source shared helpers from their own completion directory; on helper lookup failure, adapters must still fail closed and must not add legacy fallback code paths
- helper usage must keep completion behavior contract-compatible with existing adapter expectations

### Shell compatibility caveats
- zsh helper loading should prefer the adapter source path (`functions_source[...]`) and then `fpath` lookup to support both direct `source` and autoloaded completion contexts
- bash helper loading should resolve helper paths from `${BASH_SOURCE[0]}` so colocated helper files are found when completion scripts are sourced from arbitrary working directories
- when generated scripts include shell/version-guard wrappers that conflict with renamed generated function names, helpers may strip those wrappers before `eval`, but only in deterministic, documented ways
- `zsh -n` and `bash -n` checks verify syntax only; runtime behavior still requires completion tests (`tests/zsh/completion.test.zsh`) to protect alias wiring and no-legacy invariants

## Contract Boundaries: Rust vs Shell
Rust responsibilities:

- command/subcommand/flag/value completion truth
- completion export behavior and generated content
- dynamic value providers (if used)
- completion contract tests

Shell responsibilities:

- completion hook registration (`compdef` / `complete`)
- alias registration and alias-to-command mapping
- generated-script loading and thin adapter wiring only

Any logic that can drift from clap parsing belongs in Rust, not shell adapters.

## Alias Sync Policy (zsh/bash)
When a CLI ships aliases, alias definitions are a dual-shell contract:

- Zsh aliases: `completions/zsh/aliases.zsh`
- Bash aliases: `completions/bash/aliases.bash`

Rules:

- add/remove/rename aliases in both files in the same change
- keep alias families aligned with workspace policy:
  - `gs*` for `git-scope`
  - `gx*` for `git-cli`
  - `cx*` for `codex-cli` (no `codex-*`/`crl*`)
  - `fx*` for `fzf-cli`
- ensure completion registration covers aliases that require command completion behavior

## No Legacy Completion Mode Policy
Completion implementations must not maintain dual completion stacks (`clap` + legacy shell tree).

Rules:

- do not add `<CLI_NAME_UPPER>_COMPLETION_MODE` toggles for completion behavior
- do not keep legacy completion dispatch functions alongside clap-generated completion paths
- if generated completion quality is wrong, fix clap metadata and/or thin adapter logic in-place
- generated-load failure must fail closed (empty/no candidates) rather than routing to a legacy path

### Required no-legacy enforcement metadata
Every completion-required CLI migration must declare and validate this metadata contract in both:

- the CLI row in `docs/reports/completion-coverage-matrix.md`
- the crate-local migration contract copied from `docs/specs/completion-contract-template.md`

Canonical metadata tuple:

- `completion_mode=clap-first`
- `legacy_completion_mode_toggles=forbidden` (legacy completion mode toggles are disallowed, including `<CLI_NAME_UPPER>_COMPLETION_MODE`)
- `legacy_completion_dispatch=forbidden`
- `generated_load_failure=fail-closed`

Validation expectation:

- migration contracts must include explicit evidence for no-legacy metadata checks
- validation must include `COMPLETION_MODE` and legacy completion mode grep checks for touched completion paths

## Testing Requirements and Required Checks Linkage
Completion work must satisfy completion-specific checks and repository gates.

Mandatory completion-focused validation when completion code changes:

1. `zsh -f tests/zsh/completion.test.zsh`
2. `zsh -n completions/zsh/_<cli>`
3. `bash -n completions/bash/<cli>`
4. export smoke check from CLI (example pattern):
   - `<cli> completion zsh | rg -- "--help|--version|--"`

Mandatory repository checks:

- preferred single entrypoint:
  - `./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh`
- docs-only fast path (when all changed files are docs):
  - `./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh --docs-only`
- for non-doc changes, coverage must remain `>= 85.00%` per `DEVELOPMENT.md`

Completion changes are not deliverable until required checks pass.

## Rollout Checklist: Migrating an Existing CLI
Use this checklist for every existing CLI migration. The migration is only complete when the
per-CLI contract is filled, validation evidence is captured, and acceptance criteria are checked.

1. confirm the CLI is completion-required (or explicitly excluded) in
   `docs/reports/completion-coverage-matrix.md`, and for `required` CLIs ensure the matrix row has
   explicit no-legacy enforcement metadata
2. create the per-CLI migration contract from `docs/specs/completion-contract-template.md`:
   - `mkdir -p crates/<crate>/docs/reports`
   - `cp docs/specs/completion-contract-template.md crates/<crate>/docs/reports/<cli>-completion-migration-contract.md`
3. fill the contract `command graph`, `value providers`, `alias map`, `no-legacy enforcement metadata`,
   and `no-legacy invariants` sections before changing code so scope and invariants are explicit
4. ensure clap model fully expresses subcommands/flags/value candidates
5. add completion export path (e.g., `completion <shell>`) powered by `clap_complete`
6. generate/update `completions/zsh/_<cli>` and `completions/bash/<cli>`
7. keep shell adapters thin (alias wiring and optional dynamic hooks only)
8. if dynamic values are needed, add `CompleteEnv` (or crate-local `__complete` only where justified)
9. enforce no-legacy completion policy and metadata values (no `COMPLETION_MODE` gates or legacy completion functions)
10. sync aliases in both alias files when alias-bearing CLIs are touched and update the contract
    `alias map` with any rewrite semantics
11. run contract validation commands and required repository checks; record output in the contract,
    including no-legacy metadata verification evidence
12. mark contract acceptance criteria complete and link the contract path in PR notes
