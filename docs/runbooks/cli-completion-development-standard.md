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
Use this checklist for every existing CLI migration:

1. confirm the CLI is completion-required (or explicitly excluded) in coverage matrix
2. ensure clap model fully expresses subcommands/flags/value candidates
3. add completion export path (e.g., `completion <shell>`) powered by `clap_complete`
4. generate/update `completions/zsh/_<cli>` and `completions/bash/<cli>`
5. keep shell adapters thin (alias wiring and optional dynamic hooks only)
6. if dynamic values are needed, add `CompleteEnv` (or crate-local `__complete` only where justified)
7. enforce no-legacy completion policy (no `COMPLETION_MODE` gates or legacy completion functions)
8. sync aliases in both alias files when alias-bearing CLIs are touched
9. run completion checks + required repo checks; record rollout status in PR notes
