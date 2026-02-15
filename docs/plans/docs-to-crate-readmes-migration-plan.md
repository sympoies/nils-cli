# Plan: Migrate `docs/` crate folders into per-crate `README.md`

## Overview
This plan migrates crate-specific documentation currently stored under `docs/CRATE/` into
`crates/CRATE/README.md`, then removes the old `docs/` files (keeping `docs/plans/`). While doing
so, it replaces machine-local references (for example `~/.config/zsh/...`, `~/.agents/...`,
`/Users/terry/...`) with stable GitHub permalinks to the upstream repos:
`graysurf/zsh-kit` and `graysurf/codex-kit`.

## Scope
- In scope:
  - Create `README.md` for each crate that currently has docs under `docs/`.
  - Move/merge each crate’s `spec.md` + `fixtures.md` (and any crate docs README) into that crate’s
    `README.md`.
  - Replace doc references to local paths with GitHub links to upstream sources.
  - Update repo references (`README.md`, `docs/plans/*.md`) that point at the removed `docs/` files.
  - Remove `docs/*` files and crate folders, keeping only `docs/plans/`.
- Out of scope:
  - CLI behavior changes, new features, refactors, or test rewrites not required by path/link fixes.

## Assumptions (if any)
1. `docs/plans/` remains the canonical home for plan files in this repo.
2. Upstream repos use the `main` branch:
   - `https://github.com/graysurf/zsh-kit` (default branch `main`)
   - `https://github.com/graysurf/codex-kit` (default branch `main`)
3. New crate READMEs can be longer and include spec/fixture sections verbatim (parity docs remain
   first-class artifacts; only the location changes).
4. It is acceptable to replace “snapshot” files currently stored under `docs/CRATE/source/` with
   GitHub links instead of keeping local copies.

## Inventory (current state → target)

### Crate docs folders to migrate
- `docs/api-gql/{spec.md,fixtures.md}` → `crates/api-gql/README.md`
- `docs/api-rest/{spec.md,fixtures.md}` → `crates/api-rest/README.md`
- `docs/api-test/{spec.md,fixtures.md}` → `crates/api-test/README.md`
- `docs/api-testing/{overview.md,usage.md}` → `crates/api-testing-core/README.md`
- `docs/codex-cli/{README.md,spec.md,fixtures.md}` → `crates/codex-cli/README.md`
- `docs/fzf-cli/{spec.md,fixtures.md}` → `crates/fzf-cli/README.md`
- `docs/git-lock/{spec.md,fixtures.md,source/*}` → `crates/git-lock/README.md` (replace `source/*` with upstream links)
- `docs/git-scope/{spec.md,fixtures.md}` → `crates/git-scope/README.md`
- `docs/git-summary/{spec.md,fixtures.md,source/*}` → `crates/git-summary/README.md` (replace `source/*` with upstream links)
- `docs/image-processing/{spec.md,fixtures.md}` → `crates/image-processing/README.md`
- `docs/semantic-commit/{spec.md,fixtures.md}` → `crates/semantic-commit/README.md`

### Non-crate docs files to remove or relocate
- `docs/completions-strategy.md` → merge into repo root `README.md` (then delete)
- `docs/zsh-cli-reference.md` → delete (replace any plan references with upstream GitHub links)
- `docs/notes/coverage-gap.md` → `notes/coverage-gap.md` (then delete `docs/notes/`)

## Link rewrite rules (local → GitHub)

### zsh-kit
- `~/.config/zsh/scripts/PATH` → `https://github.com/graysurf/zsh-kit/blob/main/scripts/PATH`
- `~/.config/zsh/scripts/_completion/FILE` → `https://github.com/graysurf/zsh-kit/blob/main/scripts/_completion/FILE`
- `~/.config/zsh/scripts/_features/PATH` → `https://github.com/graysurf/zsh-kit/blob/main/scripts/_features/PATH`
- `~/.config/zsh/docs/cli/FILE` → `https://github.com/graysurf/zsh-kit/blob/main/docs/cli/FILE`

### codex-kit
- `/Users/terry/.config/codex-kit/PATH` (or `~/.config/codex-kit/PATH`) →
  `https://github.com/graysurf/codex-kit/blob/main/PATH`
- `$AGENTS_HOME/skills/tools/devex/semantic-commit/scripts/FILE` →
  `https://github.com/graysurf/codex-kit/blob/main/skills/tools/devex/semantic-commit/scripts/FILE`

## Sprint 1: Create per-crate READMEs (migrate docs content)
**Goal**: Each crate previously documented under `docs/CRATE/` has a `crates/CRATE/README.md`
that contains the relevant spec/fixtures content and uses upstream GitHub links for source refs.
**Demo/Validation**:
- Command(s): `find crates -maxdepth 2 -name README.md -print | sort`
- Verify: The output includes README files for all crates listed in “Crate docs folders to migrate”.

### Task 1.1: Migrate `docs/api-gql/*` into `crates/api-gql/README.md`
- **Location**:
  - `crates/api-gql/README.md`
- **Description**: Create a crate-local README by merging `docs/api-gql/spec.md` and
  `docs/api-gql/fixtures.md` (preserve headings; ensure the README reads well as a single document).
- **Dependencies**:
  - none
- **Complexity**: 2
- **Acceptance criteria**:
  - `crates/api-gql/README.md` exists and contains both the parity spec and fixtures content.
  - The README has no references to `docs/api-gql/`.
- **Validation**:
  - `test -f crates/api-gql/README.md`
  - `rg -n \"api-gql parity spec\" crates/api-gql/README.md`
  - `rg -n \"api-gql fixtures\" crates/api-gql/README.md`
  - `! rg -n \"docs/api-gql/\" crates/api-gql/README.md`

### Task 1.2: Migrate `docs/api-rest/*` into `crates/api-rest/README.md`
- **Location**:
  - `crates/api-rest/README.md`
- **Description**: Create a crate-local README by merging `docs/api-rest/spec.md` and
  `docs/api-rest/fixtures.md`, and update any cross-links that currently point at `docs/api-testing/*`
  to point at `crates/api-testing-core/README.md`.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 3
- **Acceptance criteria**:
  - `crates/api-rest/README.md` exists and contains both the parity spec and fixtures content.
  - Cross-links to API testing docs point at `crates/api-testing-core/README.md`.
  - The README has no references to `docs/api-rest/`.
- **Validation**:
  - `test -f crates/api-rest/README.md`
  - `rg -n \"api-rest parity spec\" crates/api-rest/README.md`
  - `rg -n \"api-rest fixtures\" crates/api-rest/README.md`
  - `rg -n \"crates/api-testing-core/README.md\" crates/api-rest/README.md`
  - `! rg -n \"docs/api-rest/\" crates/api-rest/README.md`

### Task 1.3: Migrate `docs/api-test/*` into `crates/api-test/README.md`
- **Location**:
  - `crates/api-test/README.md`
- **Description**: Create a crate-local README by merging `docs/api-test/spec.md` and
  `docs/api-test/fixtures.md` into a single document.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 2
- **Acceptance criteria**:
  - `crates/api-test/README.md` exists and contains both the parity spec and fixtures content.
  - The README has no references to `docs/api-test/`.
- **Validation**:
  - `test -f crates/api-test/README.md`
  - `rg -n \"api-test parity spec\" crates/api-test/README.md`
  - `rg -n \"api-test fixtures\" crates/api-test/README.md`
  - `! rg -n \"docs/api-test/\" crates/api-test/README.md`

### Task 1.4: Migrate `docs/api-testing/*` into `crates/api-testing-core/README.md`
- **Location**:
  - `crates/api-testing-core/README.md`
- **Description**: Merge `docs/api-testing/overview.md` and `docs/api-testing/usage.md` into a
  crate-local README. Keep the doc structure, and ensure it links to the per-CLI READMEs for
  `api-rest`, `api-gql`, and `api-test` using repo-local paths.
- **Dependencies**:
  - Task 1.1
  - Task 1.2
  - Task 1.3
- **Complexity**: 4
- **Acceptance criteria**:
  - `crates/api-testing-core/README.md` exists and contains the overview + usage content.
  - Links reference `crates/api-rest/README.md`, `crates/api-gql/README.md`, and
    `crates/api-test/README.md` (not `docs/api-testing/*`).
- **Validation**:
  - `test -f crates/api-testing-core/README.md`
  - `rg -n \"API testing\" crates/api-testing-core/README.md`
  - `rg -n \"crates/api-rest/README.md\" crates/api-testing-core/README.md`
  - `rg -n \"crates/api-gql/README.md\" crates/api-testing-core/README.md`
  - `rg -n \"crates/api-test/README.md\" crates/api-testing-core/README.md`

### Task 1.5: Migrate `docs/codex-cli/*` into `crates/codex-cli/README.md` (replace local paths)
- **Location**:
  - `crates/codex-cli/README.md`
- **Description**: Merge `docs/codex-cli/README.md`, `docs/codex-cli/spec.md`, and
  `docs/codex-cli/fixtures.md` into one README. Replace references to local Zsh paths under
  `~/.config/zsh/scripts/_features/codex/` with GitHub links to `graysurf/zsh-kit`.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 5
- **Acceptance criteria**:
  - `crates/codex-cli/README.md` exists and includes the former docs README, spec, and fixtures.
  - The README contains no `~/.config/zsh/` references; upstream links point at `zsh-kit` GitHub URLs.
- **Validation**:
  - `test -f crates/codex-cli/README.md`
  - `rg -n \"codex-cli parity spec\" crates/codex-cli/README.md`
  - `rg -n \"codex-cli fixtures\" crates/codex-cli/README.md`
  - `! rg -n \"~/.config/zsh\" crates/codex-cli/README.md`
  - `rg -n \"github.com/graysurf/zsh-kit\" crates/codex-cli/README.md`

### Task 1.6: Migrate `docs/fzf-cli/*` into `crates/fzf-cli/README.md` (replace local paths)
- **Location**:
  - `crates/fzf-cli/README.md`
- **Description**: Merge `docs/fzf-cli/spec.md` and `docs/fzf-cli/fixtures.md` into a crate-local
  README. Replace the local source reference to `~/.config/zsh/scripts/fzf-tools.zsh` with a GitHub
  link to `graysurf/zsh-kit/blob/main/scripts/fzf-tools.zsh`.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 3
- **Acceptance criteria**:
  - `crates/fzf-cli/README.md` exists and includes both spec + fixtures.
  - The README contains no `~/.config/zsh/scripts/fzf-tools.zsh` references.
  - The README links to the upstream `fzf-tools.zsh` file in `zsh-kit`.
- **Validation**:
  - `test -f crates/fzf-cli/README.md`
  - `rg -n \"fzf-cli parity spec\" crates/fzf-cli/README.md`
  - `rg -n \"github.com/graysurf/zsh-kit/blob/main/scripts/fzf-tools.zsh\" crates/fzf-cli/README.md`

### Task 1.7: Migrate `docs/git-lock/*` into `crates/git-lock/README.md` (replace snapshots with links)
- **Location**:
  - `crates/git-lock/README.md`
- **Description**: Merge `docs/git-lock/spec.md` and `docs/git-lock/fixtures.md` into a crate-local
  README. Replace the local snapshot references under `docs/git-lock/source/*` with GitHub links to:
  - Zsh script: `graysurf/zsh-kit/blob/main/scripts/git/git-lock.zsh`
  - Completion: `graysurf/zsh-kit/blob/main/scripts/_completion/_git-lock`
  - Docs: `graysurf/zsh-kit/blob/main/docs/cli/git-lock.md`
- **Dependencies**:
  - Task 1.1
- **Complexity**: 4
- **Acceptance criteria**:
  - `crates/git-lock/README.md` exists and includes spec + fixtures content.
  - The README references upstream GitHub links for the Zsh sources and does not reference
    `docs/git-lock/source/`.
- **Validation**:
  - `test -f crates/git-lock/README.md`
  - `rg -n \"git-lock parity spec\" crates/git-lock/README.md`
  - `rg -n \"github.com/graysurf/zsh-kit/blob/main/scripts/git/git-lock.zsh\" crates/git-lock/README.md`
  - `rg -n \"github.com/graysurf/zsh-kit/blob/main/scripts/_completion/_git-lock\" crates/git-lock/README.md`
  - `rg -n \"github.com/graysurf/zsh-kit/blob/main/docs/cli/git-lock.md\" crates/git-lock/README.md`
  - `! rg -n \"docs/git-lock/source/\" crates/git-lock/README.md`

### Task 1.8: Migrate `docs/git-scope/*` into `crates/git-scope/README.md`
- **Location**:
  - `crates/git-scope/README.md`
- **Description**: Merge `docs/git-scope/spec.md` and `docs/git-scope/fixtures.md` into a crate-local
  README. If any fixtures/spec reference local Zsh paths, replace them with GitHub links to
  `graysurf/zsh-kit`.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 3
- **Acceptance criteria**:
  - `crates/git-scope/README.md` exists and contains both spec + fixtures.
  - No machine-local references remain (`~/.config/zsh`, `/Users/`).
- **Validation**:
  - `test -f crates/git-scope/README.md`
  - `rg -n \"git-scope parity spec\" crates/git-scope/README.md`
  - `! rg -n \"~/.config/zsh|/Users/\" crates/git-scope/README.md`

### Task 1.9: Migrate `docs/git-summary/*` into `crates/git-summary/README.md` (replace snapshots with links)
- **Location**:
  - `crates/git-summary/README.md`
- **Description**: Merge `docs/git-summary/spec.md` and `docs/git-summary/fixtures.md` into a
  crate-local README. Replace the local snapshot references under `docs/git-summary/source/*` with
  GitHub links to:
  - Zsh script: `graysurf/zsh-kit/blob/main/scripts/git/git-summary.zsh`
  - Completion: `graysurf/zsh-kit/blob/main/scripts/_completion/_git-summary`
  - Docs: `graysurf/zsh-kit/blob/main/docs/cli/git-summary.md`
- **Dependencies**:
  - Task 1.1
- **Complexity**: 4
- **Acceptance criteria**:
  - `crates/git-summary/README.md` exists and includes spec + fixtures content.
  - The README references upstream GitHub links and does not reference `docs/git-summary/source/`.
- **Validation**:
  - `test -f crates/git-summary/README.md`
  - `rg -n \"git-summary parity spec\" crates/git-summary/README.md`
  - `rg -n \"github.com/graysurf/zsh-kit/blob/main/scripts/git/git-summary.zsh\" crates/git-summary/README.md`
  - `rg -n \"github.com/graysurf/zsh-kit/blob/main/scripts/_completion/_git-summary\" crates/git-summary/README.md`
  - `rg -n \"github.com/graysurf/zsh-kit/blob/main/docs/cli/git-summary.md\" crates/git-summary/README.md`
  - `! rg -n \"docs/git-summary/source/\" crates/git-summary/README.md`

### Task 1.10: Migrate `docs/image-processing/*` into `crates/image-processing/README.md` (codex-kit links)
- **Location**:
  - `crates/image-processing/README.md`
- **Description**: Merge `docs/image-processing/spec.md` and `docs/image-processing/fixtures.md` into
  a crate-local README, replacing references to local `~/.config/codex-kit/...` paths with GitHub
  links into `graysurf/codex-kit` (for example `skills/tools/media/image-processing/...`).
- **Dependencies**:
  - Task 1.1
- **Complexity**: 3
- **Acceptance criteria**:
  - `crates/image-processing/README.md` exists and includes spec + fixtures.
  - The README contains no `~/.config/codex-kit` references.
  - The README includes at least one `github.com/graysurf/codex-kit` link for upstream reference.
- **Validation**:
  - `test -f crates/image-processing/README.md`
  - `rg -n \"image-processing parity spec\" crates/image-processing/README.md`
  - `! rg -n \"~/.config/codex-kit\" crates/image-processing/README.md`
  - `rg -n \"github.com/graysurf/codex-kit\" crates/image-processing/README.md`

### Task 1.11: Migrate `docs/semantic-commit/*` into `crates/semantic-commit/README.md` (codex-kit links)
- **Location**:
  - `crates/semantic-commit/README.md`
- **Description**: Merge `docs/semantic-commit/spec.md` and `docs/semantic-commit/fixtures.md` into a
  crate-local README. Replace references to `~/.agents/skills/...` scripts with GitHub links into
  `graysurf/codex-kit` for the semantic-commit skill scripts.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 4
- **Acceptance criteria**:
  - `crates/semantic-commit/README.md` exists and includes spec + fixtures.
  - The README contains no `~/.agents/` references.
  - The README links to `graysurf/codex-kit` semantic-commit scripts.
- **Validation**:
  - `test -f crates/semantic-commit/README.md`
  - `rg -n \"semantic-commit parity spec\" crates/semantic-commit/README.md`
  - `! rg -n \"~/.agents\" crates/semantic-commit/README.md`
  - `rg -n \"github.com/graysurf/codex-kit/blob/main/skills/tools/devex/semantic-commit/scripts\" crates/semantic-commit/README.md`

## Sprint 2: Update repo references and remove old `docs/` files
**Goal**: No repository docs or plans reference removed `docs/` crate folders or machine-local
paths; old docs are deleted and all validation commands still run.
**Demo/Validation**:
- Command(s): `rg -n \"docs/(api-|git-|fzf-|codex-cli|image-processing|semantic-commit)\" README.md docs/plans crates || true`
- Verify: The grep output contains no references to the removed `docs/CRATE/` paths.

### Task 2.1: Update repo root `README.md` to point at crate READMEs
- **Location**:
  - `README.md`
- **Description**: Replace links that point at `docs/*` with links to the new crate-local READMEs:
  - `docs/fzf-cli/spec.md` → `crates/fzf-cli/README.md`
  - `docs/codex-cli/README.md` → `crates/codex-cli/README.md`
  - `docs/api-testing/usage.md` → `crates/api-testing-core/README.md`
  - Merge `docs/completions-strategy.md` content into `README.md`, then remove the docs link.
- **Dependencies**:
  - Task 1.4
  - Task 1.5
  - Task 1.6
- **Complexity**: 3
- **Acceptance criteria**:
  - Root README contains no references to removed `docs/*` files.
  - Root README retains the completions/wrappers guidance (now inlined).
- **Validation**:
  - `! rg -n \"docs/fzf-cli/spec.md\" README.md`
  - `! rg -n \"docs/codex-cli/README.md\" README.md`
  - `! rg -n \"docs/api-testing/usage.md\" README.md`
  - `! rg -n \"docs/completions-strategy.md\" README.md`
  - `rg -n \"completions\" README.md`

### Task 2.2: Update `docs/plans/*.md` to use GitHub links (zsh-kit and codex-kit)
- **Location**:
  - `docs/plans/api-testing-clis-rust-port-plan.md`
  - `docs/plans/codex-cli-rust-port-plan.md`
  - `docs/plans/fzf-cli-rust-port-plan.md`
  - `docs/plans/git-lock-rust-port-plan.md`
  - `docs/plans/git-scope-rust-port-plan.md`
  - `docs/plans/git-summary-rust-port-plan.md`
  - `docs/plans/image-processing-rust-port-plan.md`
  - `docs/plans/nils-term-progress-plan.md`
  - `docs/plans/plan-tooling-cli-consolidation-plan.md`
  - `docs/plans/rust-cli-repo-setup-plan.md`
  - `docs/plans/semantic-commit-rust-port-plan.md`
  - `docs/plans/test-coverage-70-plan.md`
- **Description**: Replace machine-local source references with GitHub links:
  - `~/.config/zsh/...` and `/Users/terry/.config/zsh/...` → `graysurf/zsh-kit` links.
  - `~/.config/codex-kit/...` and `/Users/terry/.config/codex-kit/...` → `graysurf/codex-kit` links.
  - `~/.agents/...` → `graysurf/codex-kit` links when pointing at tracked skill sources.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 6
- **Acceptance criteria**:
  - None of the listed plan files contain `~/.config/zsh`, `~/.config/codex-kit`, `~/.agents/`, or
    `/Users/terry/`.
  - Updated links resolve to the correct upstream repos (`zsh-kit` vs `codex-kit`).
- **Validation**:
  - `! rg -n \"~/.config/zsh\" docs/plans/api-testing-clis-rust-port-plan.md docs/plans/codex-cli-rust-port-plan.md docs/plans/fzf-cli-rust-port-plan.md docs/plans/git-lock-rust-port-plan.md docs/plans/git-scope-rust-port-plan.md docs/plans/git-summary-rust-port-plan.md docs/plans/image-processing-rust-port-plan.md docs/plans/nils-term-progress-plan.md docs/plans/plan-tooling-cli-consolidation-plan.md docs/plans/rust-cli-repo-setup-plan.md docs/plans/semantic-commit-rust-port-plan.md docs/plans/test-coverage-70-plan.md`
  - `! rg -n \"~/.config/codex-kit\" docs/plans/api-testing-clis-rust-port-plan.md docs/plans/codex-cli-rust-port-plan.md docs/plans/fzf-cli-rust-port-plan.md docs/plans/git-lock-rust-port-plan.md docs/plans/git-scope-rust-port-plan.md docs/plans/git-summary-rust-port-plan.md docs/plans/image-processing-rust-port-plan.md docs/plans/nils-term-progress-plan.md docs/plans/plan-tooling-cli-consolidation-plan.md docs/plans/rust-cli-repo-setup-plan.md docs/plans/semantic-commit-rust-port-plan.md docs/plans/test-coverage-70-plan.md`
  - `! rg -n \"~/.agents\" docs/plans/api-testing-clis-rust-port-plan.md docs/plans/codex-cli-rust-port-plan.md docs/plans/fzf-cli-rust-port-plan.md docs/plans/git-lock-rust-port-plan.md docs/plans/git-scope-rust-port-plan.md docs/plans/git-summary-rust-port-plan.md docs/plans/image-processing-rust-port-plan.md docs/plans/nils-term-progress-plan.md docs/plans/plan-tooling-cli-consolidation-plan.md docs/plans/rust-cli-repo-setup-plan.md docs/plans/semantic-commit-rust-port-plan.md docs/plans/test-coverage-70-plan.md`
  - `! rg -n \"/Users/terry\" docs/plans/api-testing-clis-rust-port-plan.md docs/plans/codex-cli-rust-port-plan.md docs/plans/fzf-cli-rust-port-plan.md docs/plans/git-lock-rust-port-plan.md docs/plans/git-scope-rust-port-plan.md docs/plans/git-summary-rust-port-plan.md docs/plans/image-processing-rust-port-plan.md docs/plans/nils-term-progress-plan.md docs/plans/plan-tooling-cli-consolidation-plan.md docs/plans/rust-cli-repo-setup-plan.md docs/plans/semantic-commit-rust-port-plan.md docs/plans/test-coverage-70-plan.md`
  - `rg -n \"github.com/graysurf/(zsh-kit|codex-kit)\" docs/plans/api-testing-clis-rust-port-plan.md docs/plans/codex-cli-rust-port-plan.md docs/plans/fzf-cli-rust-port-plan.md docs/plans/git-lock-rust-port-plan.md docs/plans/git-scope-rust-port-plan.md docs/plans/git-summary-rust-port-plan.md docs/plans/image-processing-rust-port-plan.md docs/plans/nils-term-progress-plan.md docs/plans/plan-tooling-cli-consolidation-plan.md docs/plans/rust-cli-repo-setup-plan.md docs/plans/semantic-commit-rust-port-plan.md docs/plans/test-coverage-70-plan.md`

### Task 2.3: Update `docs/plans/*.md` references to migrated crate docs paths
- **Location**:
  - `docs/plans/api-report-from-cmd-subcommands-plan.md`
  - `docs/plans/api-testing-clis-rust-port-plan.md`
  - `docs/plans/codex-cli-rust-port-plan.md`
  - `docs/plans/fzf-cli-rust-port-plan.md`
  - `docs/plans/git-lock-rust-port-plan.md`
  - `docs/plans/git-scope-rust-port-plan.md`
  - `docs/plans/git-summary-rust-port-plan.md`
  - `docs/plans/image-processing-rust-port-plan.md`
  - `docs/plans/semantic-commit-rust-port-plan.md`
- **Description**: Replace plan references to `docs/CRATE/spec.md`, `docs/CRATE/fixtures.md`,
  and `docs/CRATE/source/*` with:
  - `crates/CRATE/README.md` for spec/fixtures content
  - GitHub links for upstream Zsh sources (instead of `docs/CRATE/source/*`)
- **Dependencies**:
  - Task 1.1
  - Task 2.2
- **Complexity**: 7
- **Acceptance criteria**:
  - No plan files reference `docs/CRATE/spec.md` or `docs/CRATE/fixtures.md` for migrated crates.
  - Plans referencing Zsh snapshots use GitHub links (not repo-local snapshot files).
- **Validation**:
  - `! rg -n \"docs/(api-gql|api-rest|api-test|api-testing|codex-cli|fzf-cli|git-lock|git-scope|git-summary|image-processing|semantic-commit)/\" docs/plans/api-report-from-cmd-subcommands-plan.md docs/plans/api-testing-clis-rust-port-plan.md docs/plans/codex-cli-rust-port-plan.md docs/plans/fzf-cli-rust-port-plan.md docs/plans/git-lock-rust-port-plan.md docs/plans/git-scope-rust-port-plan.md docs/plans/git-summary-rust-port-plan.md docs/plans/image-processing-rust-port-plan.md docs/plans/semantic-commit-rust-port-plan.md`
  - `rg -n \"crates/(api-gql|api-rest|api-test|api-testing-core|codex-cli|fzf-cli|git-lock|git-scope|git-summary|image-processing|semantic-commit)/README.md\" docs/plans/api-report-from-cmd-subcommands-plan.md docs/plans/api-testing-clis-rust-port-plan.md docs/plans/codex-cli-rust-port-plan.md docs/plans/fzf-cli-rust-port-plan.md docs/plans/git-lock-rust-port-plan.md docs/plans/git-scope-rust-port-plan.md docs/plans/git-summary-rust-port-plan.md docs/plans/image-processing-rust-port-plan.md docs/plans/semantic-commit-rust-port-plan.md`

### Task 2.4: Relocate or remove remaining non-plan docs files
- **Location**:
  - `README.md`
  - `notes/coverage-gap.md`
- **Description**: Remove the remaining non-plan docs by relocating what is still needed and
  deleting old references.
  - Create `notes/coverage-gap.md` by moving `docs/notes/coverage-gap.md`.
  - Delete `docs/zsh-cli-reference.md` and update any plan references to point to upstream
    `graysurf/zsh-kit` docs instead.
  - Delete `docs/completions-strategy.md` after its content is merged into `README.md` (Task 2.1).
- **Dependencies**:
  - Task 2.1
  - Task 2.2
- **Complexity**: 4
- **Acceptance criteria**:
  - `notes/coverage-gap.md` exists (if coverage-gap is still referenced by plans).
  - `docs/zsh-cli-reference.md` and `docs/completions-strategy.md` are removed.
- **Validation**:
  - `test -f notes/coverage-gap.md`
  - `test ! -f docs/zsh-cli-reference.md`
  - `test ! -f docs/completions-strategy.md`

### Task 2.5: Delete migrated `docs/CRATE/` folders (keep `docs/plans/`)
- **Location**:
  - `docs/api-gql/fixtures.md`
  - `docs/api-gql/spec.md`
  - `docs/api-rest/fixtures.md`
  - `docs/api-rest/spec.md`
  - `docs/api-test/fixtures.md`
  - `docs/api-test/spec.md`
  - `docs/api-testing/overview.md`
  - `docs/api-testing/usage.md`
  - `docs/codex-cli/README.md`
  - `docs/codex-cli/fixtures.md`
  - `docs/codex-cli/spec.md`
  - `docs/fzf-cli/fixtures.md`
  - `docs/fzf-cli/spec.md`
  - `docs/git-lock/fixtures.md`
  - `docs/git-lock/spec.md`
  - `docs/git-lock/source/_git-lock`
  - `docs/git-lock/source/git-lock.md`
  - `docs/git-lock/source/git-lock.zsh`
  - `docs/git-scope/fixtures.md`
  - `docs/git-scope/spec.md`
  - `docs/git-summary/fixtures.md`
  - `docs/git-summary/spec.md`
  - `docs/git-summary/source/_git-summary`
  - `docs/git-summary/source/git-summary.md`
  - `docs/git-summary/source/git-summary.zsh`
  - `docs/image-processing/fixtures.md`
  - `docs/image-processing/spec.md`
  - `docs/semantic-commit/fixtures.md`
  - `docs/semantic-commit/spec.md`
  - `docs/notes/coverage-gap.md`
  - `docs/completions-strategy.md`
  - `docs/zsh-cli-reference.md`
- **Description**: Remove the crate docs folders that have been migrated into crate READMEs:
  `docs/api-gql/`, `docs/api-rest/`, `docs/api-test/`, `docs/api-testing/`, `docs/codex-cli/`,
  `docs/fzf-cli/`, `docs/git-lock/`, `docs/git-scope/`, `docs/git-summary/`,
  `docs/image-processing/`, and `docs/semantic-commit/`. Keep `docs/plans/` untouched.
- **Dependencies**:
  - Task 2.3
  - Task 2.4
- **Complexity**: 4
- **Acceptance criteria**:
  - Only `docs/plans/` remains under `docs/` (and any empty dirs removed).
- **Validation**:
  - `find docs -maxdepth 1 -mindepth 1 -print | sort`
  - `test -d docs/plans`

### Task 2.6: Repo-wide validation and regression checks
- **Location**:
  - workspace
- **Description**: Ensure the repo contains no stale references and the workspace checks still pass.
  Run the standard repo checks plus a doc reference scan.
- **Dependencies**:
  - Task 2.5
- **Complexity**: 5
- **Acceptance criteria**:
  - No references remain to deleted docs paths.
  - Standard lint/tests pass.
- **Validation**:
  - `! rg -n \"docs/(api-|git-|fzf-|codex-cli|image-processing|semantic-commit)\" . --glob '!docs/plans/docs-to-crate-readmes-migration-plan.md'`
  - `./.agents/skills/nils-cli-checks/scripts/nils-cli-checks.sh`

## Testing Strategy
- Unit: unchanged (documentation-only changes should not affect unit tests).
- Integration: unchanged; still run `cargo test --workspace` via the repo checks script.
- E2E/manual: confirm links in `README.md` and a couple of representative crate READMEs open the
  correct GitHub pages for upstream reference.

## Risks & gotchas
- Removing `docs/CRATE/source/*` snapshots may reduce offline parity reference; mitigate by linking
  to stable GitHub sources and keeping the Rust tests as the parity guard.
- Existing plan files contain many concrete validation commands; path updates must keep commands
  runnable and meaningful after the migration.
- Some references may exist outside `docs/` (for example in scripts or tests); use repo-wide `rg`
  scans before deleting docs.

## Rollback plan
- If the migration causes confusion or breaks workflows, revert the documentation commit(s) and keep
  `docs/CRATE/` in place temporarily.
- If only links are wrong: re-add `docs/CRATE/source/*` snapshots for the affected crates as a
  short-term fallback, then correct the GitHub link mapping.
