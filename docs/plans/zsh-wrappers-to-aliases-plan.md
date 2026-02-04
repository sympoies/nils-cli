# Plan: Zsh wrappers -> aliases (git-scope + codex-cli + fzf-cli)

## Overview
Replace selected wrapper scripts in `wrappers/` with Zsh aliases (and a small number of Zsh functions
where required) so that, after `brew install`, users can enable short commands with a single `source`
line and still get completion via the existing `compdef` mappings in `completions/zsh/`.

Initial scope: `git-scope` (`gs*` aliases for all subcommands), `codex-cli` (`cx*` only), and `fzf-cli`
(`ff*` naming scheme).

## Scope
- In scope:
  - Add a Zsh aliases snippet (opt-in via `source`) providing aliases for:
    - `git-scope` (`gs*` aliases for all subcommands)
    - `codex-cli` (`cx*` only; drop legacy `codex-*` and `crl/crla`)
    - `fzf-cli` (`ff*` aliases, with function wrappers only where needed for parent-shell effects)
  - Remove wrapper scripts that exist solely to provide these alias entrypoints.
  - Update Zsh completion registrations so alias names trigger the same completion as the underlying binary.
  - Update docs to describe the new setup for Homebrew and release tarball installs.
  - Add/adjust Zsh tests to ensure the aliases snippet is sourceable and stays in sync with completion.
- Out of scope:
  - Any changes to Rust CLI behavior/output.
  - Adding Bash/Fish aliases/completions.
  - Converting other wrapper scripts in `wrappers/` (api-rest/api-gql/api-test/plan-tooling/git-summary/git-lock, etc.).

## Assumptions (if any)
1. Users are OK adding one line to `~/.zshrc` to opt into aliases after `brew install`.
2. Completion must work via `compdef` (not by requiring `setopt complete_aliases`).
3. Alias snippet must not overwrite user-defined aliases/functions (define only when not already defined).
4. Homebrew packaging is managed outside this repo (formula/tap changes will be done there).
5. For `fzf-cli directory` and `fzf-cli history`, a function wrapper is preferred over a plain alias to
   preserve the documented `eval` contract (parent-shell `cd` / command execution).

## Naming rules
### git-scope
- Use the `gs` prefix:
  - `gs` is the base entrypoint (`git-scope`).
  - `gs*` names map to fixed `git-scope <subcommand>` entrypoints (e.g., `gss` = `git-scope staged`).

### codex-cli
- Keep the existing “cx” naming scheme as the canonical short form:
  - `cx` + `{a|g|d|c|s}` group + `{subcommand}` letter(s).
  - Example: `cxau` = `codex-cli auth use`.
- Do not support legacy long names (e.g. `codex-use`) or `crl/crla`; keep `cx*` only.

### fzf-cli
- Use the `ff` prefix:
  - `ff` maps to `fzf-cli`.
  - `ff*` maps to a specific `fzf-cli <subcommand>` dispatcher command.
  - Prefer readable abbreviations that avoid collisions (`ffdef`, `fffn`, `ffal`, etc.).

## Alias inventory
### git-scope
- `gs` → `git-scope`
- `gst` → `git-scope tracked`
- `gss` → `git-scope staged`
- `gsu` → `git-scope unstaged`
- `gsa` → `git-scope all`
- `gsun` → `git-scope untracked`
- `gsc` → `git-scope commit`
- `gsh` → `git-scope help`

### codex-cli
- `cx` → `codex-cli`
- `cxau` → `codex-cli auth use`
- `cxar` → `codex-cli auth refresh`
- `cxaa` → `codex-cli auth auto-refresh`
- `cxac` → `codex-cli auth current`
- `cxas` → `codex-cli auth sync`
- `cxst` → `codex-cli starship`
- `cxdr` → `codex-cli diag rate-limits`
- `cxdra` → `codex-cli diag rate-limits --async`
- `cxcs` → `codex-cli config show`
- `cxct` → `codex-cli config set`
- `cxgp` → `codex-cli agent prompt`
- `cxga` → `codex-cli agent advice`
- `cxgk` → `codex-cli agent knowledge`
- `cxgc` → `codex-cli agent commit`

### fzf-cli
- `ff` → `fzf-cli`
- `fff` → `fzf-cli file`
- `ffd` → `fzf-cli directory` (prefer a function wrapper to support `cd` via `eval`)
- `ffgs` → `fzf-cli git-status`
- `ffgc` → `fzf-cli git-commit`
- `ffgco` → `fzf-cli git-checkout`
- `ffgb` → `fzf-cli git-branch`
- `ffgt` → `fzf-cli git-tag`
- `ffp` → `fzf-cli process`
- `ffpo` → `fzf-cli port`
- `ffh` → `fzf-cli history` (prefer a function wrapper to support `eval`)
- `ffenv` → `fzf-cli env`
- `ffal` → `fzf-cli alias`
- `fffn` → `fzf-cli function`
- `ffdef` → `fzf-cli def`

## Sprint 1: Add aliases + completion support
**Goal**: Land an opt-in aliases file for the full inventory and ensure completion works for all alias names.
**Demo/Validation**:
- Command(s):
  - `zsh -f tests/zsh/completion.test.zsh`
  - `plan-tooling validate --file docs/plans/zsh-wrappers-to-aliases-plan.md`
- Verify:
  - The aliases snippet can be sourced without errors.
  - Completion is registered for all alias names listed in this plan.

### Task 1.1: Audit current wrapper + completion behavior
- **Location**:
  - `wrappers/gs`
  - `wrappers/gsc`
  - `wrappers/gst`
  - `wrappers/git-scope`
  - `wrappers/cx`
  - `wrappers/cxaa`
  - `wrappers/cxac`
  - `wrappers/cxar`
  - `wrappers/cxas`
  - `wrappers/cxau`
  - `wrappers/cxcs`
  - `wrappers/cxct`
  - `wrappers/cxdr`
  - `wrappers/cxga`
  - `wrappers/cxgc`
  - `wrappers/cxgk`
  - `wrappers/cxgp`
  - `wrappers/crl`
  - `wrappers/crla`
  - `wrappers/codex-cli`
  - `wrappers/codex-use`
  - `wrappers/codex-refresh-auth`
  - `wrappers/codex-auto-refresh`
  - `wrappers/codex-rate-limits`
  - `wrappers/codex-rate-limits-async`
  - `wrappers/codex-starship`
  - `wrappers/fzf-cli`
  - `completions/zsh/_git-scope`
  - `completions/zsh/_codex-cli`
  - `completions/zsh/_fzf-cli`
- **Description**: Confirm current behavior and completion registration, and lock in the alias mapping
  for all entries in the alias inventory (including the special-case completion behavior for wrapper
  names via `invoked_as` overrides in completion scripts).
- **Dependencies**: none
- **Complexity**: 2
- **Acceptance criteria**:
  - Wrapper scripts are confirmed to be simple `exec` shims (no extra behavior to preserve) for the
    alias entrypoints.
  - Completion files are confirmed to register `compdef` for the alias names in the inventory (or are
    updated in Task 1.3/1.5 to do so).
- **Validation**:
  - `ls wrappers | rg \"^(gs|gsc|gst|cx|cx..|crl|crla|codex-|fzf-cli)$\"`
  - `rg -n \"#compdef git-scope\" completions/zsh/_git-scope`
  - `rg -n \"#compdef codex-cli\" completions/zsh/_codex-cli`
  - `rg -n \"#compdef fzf-cli\" completions/zsh/_fzf-cli`

### Task 1.2: Add a Zsh aliases snippet for the full inventory
- **Location**:
  - `completions/zsh/aliases.zsh` (new)
- **Description**: Add an opt-in Zsh snippet defining all aliases in this plan. For `fzf-cli directory`
  and `fzf-cli history`, prefer function wrappers that can pass through `"$@"` and `eval` the emitted
  shell command to preserve parent-shell behavior. Avoid clobbering existing user aliases/functions:
  define only when not already defined.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 4
- **Acceptance criteria**:
  - The file defines exactly the aliases in the inventory section.
  - Sourcing the file in `zsh -f` does not error.
  - Aliases/functions are not redefined if the user already has them defined.
- **Validation**:
  - `zsh -f -c 'source completions/zsh/aliases.zsh && alias gs && alias cx && alias ff'`

### Task 1.3: Update `git-scope` completion registration for `gs*`
- **Location**:
  - `completions/zsh/_git-scope`
- **Description**: Register completion for the `gs*` alias names by adding them to the `#compdef` header
  and the explicit `compdef` call. Extend the existing `invoked_as` override handling so `gst/gss/gsu/gsa/gsun/gsc/gsh`
  complete as if the corresponding `git-scope <subcommand>` were already selected.
- **Dependencies**:
  - Task 1.2
- **Complexity**: 2
- **Acceptance criteria**:
  - `completions/zsh/_git-scope` registers completion for every `gs*` alias in the inventory.
  - `invoked_as` overrides cover each `gs*` alias that maps to a fixed subcommand.
- **Validation**:
  - `rg -n \"#compdef git-scope\" completions/zsh/_git-scope`
  - `rg -n \"compdef _git-scope\" completions/zsh/_git-scope`

### Task 1.4: Update `fzf-cli` completion registration for `ff*`
- **Location**:
  - `completions/zsh/_fzf-cli`
- **Description**: Register completion for `ff*` alias names by adding them to the `#compdef` header and
  the explicit `compdef` call, so completion triggers for aliases without requiring
  `setopt complete_aliases`. Add `invoked_as` override handling so `ff*` aliases that map to a fixed
  dispatcher subcommand complete as if that subcommand were already selected (e.g., `fff` behaves like
  `fzf-cli file` completion).
- **Dependencies**:
  - Task 1.2
- **Complexity**: 2
- **Acceptance criteria**:
  - `completions/zsh/_fzf-cli` registers `compdef _fzf-cli` for every `ff*` name in the alias inventory.
  - `invoked_as` overrides cover `ff*` aliases that map to a fixed dispatcher subcommand.
- **Validation**:
  - `rg -n \"#compdef fzf-cli\" completions/zsh/_fzf-cli`
  - `rg -n \"compdef _fzf-cli\" completions/zsh/_fzf-cli`

### Task 1.5: Update `codex-cli` completion registration for `cx*` only
- **Location**:
  - `completions/zsh/_codex-cli`
- **Description**: Remove completion registrations and `invoked_as` overrides for legacy long names
  (e.g. `codex-use`, `codex-rate-limits`, `crl/crla`). Ensure `compdef` covers all `cx*` aliases in
  the inventory, including `cxst` and `cxdra`, and ensure `invoked_as` overrides map fixed-entrypoint
  aliases to the correct subcommand group for completion behavior.
- **Dependencies**:
  - Task 1.2
- **Complexity**: 2
- **Acceptance criteria**:
  - `completions/zsh/_codex-cli` registers completion only for `codex-cli`, `cx`, and the `cx*` aliases
    defined by this plan.
  - `invoked_as` overrides cover each `cx*` alias that maps to fixed subcommands (including `cxst`).
- **Validation**:
  - `rg -n \"codex-use|codex-rate-limits|crl|crla\" completions/zsh/_codex-cli` returns no matches.
  - `rg -n \"cxst\" completions/zsh/_codex-cli` returns matches.
  - `rg -n \"cxdra\" completions/zsh/_codex-cli` returns matches.

### Task 1.6: Add/adjust Zsh tests to cover the aliases snippet
- **Location**:
  - `tests/zsh/completion.test.zsh`
  - (or) `tests/zsh/aliases.test.zsh` (new)
- **Description**: Ensure the aliases snippet remains sourceable and contains the expected alias names.
  This protects against accidental regressions (syntax errors, renamed file, missing aliases).
- **Dependencies**:
  - Task 1.2
- **Complexity**: 2
- **Acceptance criteria**:
  - CI runs a Zsh test that sources the aliases snippet and asserts representative names exist:
    - `gs/gst/gss/gsu/gsa/gsun/gsc/gsh`
    - `cx/cxau/cxst/cxdr/cxdra`
    - `ff/fff/ffgs/ffdef`
- **Validation**:
  - `zsh -f tests/zsh/completion.test.zsh`

## Sprint 2: Remove wrapper scripts for alias entrypoints
**Goal**: Delete wrapper scripts that exist only to provide the alias entrypoints now covered by the Zsh snippet.
**Demo/Validation**:
- Command(s):
  - `./.codex/skills/nils-cli-checks/scripts/nils-cli-checks.sh`
- Verify:
  - The wrapper scripts listed for removal are gone.
  - Docs no longer instruct users to add `wrappers/` to `PATH` for these entrypoints.

### Task 2.1: Remove `gs/gsc/gst` wrapper scripts
- **Location**:
  - `wrappers/gs`
  - `wrappers/gsc`
  - `wrappers/gst`
- **Description**: Delete the short-name wrapper scripts so the project uses aliases for these
  entrypoints. Keep `wrappers/git-scope` intact for developers who still want the “cargo run fallback”
  behavior during local development.
- **Dependencies**:
  - Task 1.2
- **Complexity**: 1
- **Acceptance criteria**:
  - The three wrapper files are removed from the repo.
  - No tests or docs require these wrapper scripts.
- **Validation**:
  - `test ! -f wrappers/gs && test ! -f wrappers/gsc && test ! -f wrappers/gst`

### Task 2.2: Remove `codex-cli` alias-entrypoint wrapper scripts
- **Location**:
  - `wrappers/cx`
  - `wrappers/cxaa`
  - `wrappers/cxac`
  - `wrappers/cxar`
  - `wrappers/cxas`
  - `wrappers/cxau`
  - `wrappers/cxcs`
  - `wrappers/cxct`
  - `wrappers/cxdr`
  - `wrappers/cxga`
  - `wrappers/cxgc`
  - `wrappers/cxgk`
  - `wrappers/cxgp`
  - `wrappers/crl`
  - `wrappers/crla`
  - `wrappers/codex-use`
  - `wrappers/codex-refresh-auth`
  - `wrappers/codex-auto-refresh`
  - `wrappers/codex-rate-limits`
  - `wrappers/codex-rate-limits-async`
  - `wrappers/codex-starship`
- **Description**: Delete the codex alias-entrypoint wrapper scripts so the project uses the Zsh aliases
  snippet for `cx*` names and explicitly drops support for legacy `codex-*` and `crl/crla` entrypoints.
  Keep `wrappers/codex-cli` intact for developers who still want the “cargo run fallback” behavior
  during local development.
- **Dependencies**:
  - Task 1.2
- **Complexity**: 2
- **Acceptance criteria**:
  - The listed wrapper files are removed from the repo.
  - No tests or docs require these wrapper scripts.
- **Validation**:
  - `ls wrappers | rg \"^(cx|crl|crla|codex-)\"` returns no matches.

### Task 2.3: Update README/DEVELOPMENT to describe aliases (not wrappers)
- **Location**:
  - `README.md`
  - `DEVELOPMENT.md`
- **Description**: Update docs to describe the new recommended UX:
  - Keep zsh completions as-is via `fpath` + `compinit`.
  - Add one `source` line to opt into aliases after Homebrew install (and for tarball installs).
  - Remove references to adding `wrappers/` to `PATH` for these alias entrypoints.
- **Dependencies**:
  - Task 2.2
- **Complexity**: 2
- **Acceptance criteria**:
  - README mentions the alias snippet path and the exact `source ...` line for Homebrew.
  - DEVELOPMENT no longer lists removed wrapper scripts under “Wrapper scripts”.
- **Validation**:
  - `rg -n \"wrappers/gs|wrappers/gsc|wrappers/gst\" README.md DEVELOPMENT.md` returns no matches.
  - `rg -n \"wrappers/cx|wrappers/crl|wrappers/codex-\" README.md DEVELOPMENT.md` returns no matches.
  - `rg -n \"nils-cli-aliases\\.zsh\" README.md DEVELOPMENT.md` returns matches.

## Sprint 3: Homebrew + release distribution
**Goal**: Ensure the aliases snippet is shipped and Homebrew users can enable it with one `source` line.
**Demo/Validation**:
- Command(s):
  - (Repo) `./.codex/skills/nils-cli-checks/scripts/nils-cli-checks.sh`
  - (Release artifact spot-check) `tar -tf dist/*.tar.gz | rg \"completions/zsh/nils-cli-aliases\\.zsh\"`
- Verify:
  - Release tarballs contain `completions/zsh/aliases.zsh`.
  - Homebrew formula installs the aliases snippet to a stable path and prints a caveat with the exact `source` line.

### Task 3.1: Ensure release artifacts ship the aliases snippet
- **Location**:
  - `completions/zsh/aliases.zsh`
  - `.github/workflows/release.yml`
- **Description**: Confirm the release packaging includes the aliases snippet. If the file lives under
  `completions/`, the existing `cp -R completions "${out_dir}/"` in the release workflow should include it.
- **Dependencies**:
  - Task 1.2
- **Complexity**: 1
- **Acceptance criteria**:
  - A release tarball contains `completions/zsh/aliases.zsh`.
- **Validation**:
  - Locally run the packaging step (or inspect a CI artifact) and confirm the file is present.

### Task 3.2: Update the Homebrew formula to install and document aliases
- **Location**:
  - Homebrew tap repo (external): `Formula/nils-cli.rb` (or equivalent)
- **Description**: Install the aliases snippet to `pkgshare` (or `etc`) and print a caveat telling users
  how to enable it.
  - Example desired caveat:
    - `source \"$(brew --prefix nils-cli)/share/nils-cli/aliases.zsh\"`
  - Recommend opt-in to avoid clobbering existing `gs` aliases.
- **Dependencies**:
  - Task 3.1
- **Complexity**: 3
- **Acceptance criteria**:
  - After `brew install`, the file exists at the documented path.
  - The formula prints an accurate caveat (or the tap README documents the line).
- **Validation**:
  - `brew reinstall --build-from-source nils-cli` (or test tap install) and verify the installed file path.

### Task 3.3: Manual UX verification (alias + completion)
- **Location**:
  - User shell environment (local)
- **Description**: Verify that aliases behave and complete as expected.
- **Dependencies**:
  - Task 3.2
- **Complexity**: 2
- **Acceptance criteria**:
  - `gs --help` behaves like `git-scope --help`.
  - `gss --help` behaves like `git-scope staged --help`.
  - `gsa --help` behaves like `git-scope all --help`.
  - Tab completion works for representative `gs*` aliases without requiring `setopt complete_aliases`.
  - `cx --help` behaves like `codex-cli --help`.
  - `cxau --help` behaves like `codex-cli auth use --help`.
  - `cxst --help` behaves like `codex-cli starship --help`.
  - `cxdra --help` behaves like `codex-cli diag rate-limits --help` (with `--async` implied by alias).
  - `ff --help` behaves like `fzf-cli --help`.
  - Tab completion works for representative codex aliases (`cx`, `cxau`, `cxst`) and fzf aliases (`ff`, `fff`, `ffgs`).
- **Validation**:
  - New zsh session:
    - Ensure `compinit` is enabled and `_git-scope` is installed.
    - `source \"$(brew --prefix nils-cli)/share/nils-cli/aliases.zsh\"`.
    - Type `gs ` then press Tab; repeat for `gss ` and `gsc `.
    - Type `cx ` then press Tab; repeat for `cxst ` and `cxdra `.
    - Type `ff ` then press Tab; repeat for `fff ` and `ffgs `.

## Testing Strategy
- Unit: none (no Rust behavior changes).
- Integration:
  - `zsh -f tests/zsh/completion.test.zsh` (and/or new alias-focused test) ensures Zsh assets are sourceable.
- E2E/manual:
  - In a clean Zsh session: source the aliases file, then verify completion works for `gs`, `cx`, and `ff`.

## Risks & gotchas
- **Alias conflicts**: `gs` is a common alias name; the snippet should avoid overwriting existing aliases.
- **Non-interactive shells**: aliases are not available unless sourced; wrapper removal may affect scripts.
- **Completion ordering**: completion requires `compinit` and completion files in `completions/zsh/` installed/available.
- **Homebrew caveats**: brew can install files but cannot auto-edit `~/.zshrc`; the formula must print instructions.
- **Parent-shell effects**: `fzf-cli directory` and `fzf-cli history` require `eval` to fully match the original UX; prefer function wrappers for `ffd` and `ffh`.

## Rollback plan
- Reintroduce `wrappers/gs`, `wrappers/gsc`, `wrappers/gst` as scripts if alias-only proves too limiting.
- Reintroduce codex alias-entrypoint wrappers if needed (`wrappers/cx*`, `wrappers/codex-*`, `wrappers/crl*`).
- Keep the aliases file as an opt-in UX improvement even if wrappers return.
