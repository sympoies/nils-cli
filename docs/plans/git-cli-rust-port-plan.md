# Plan: Rust git-cli parity (port of git-tools.zsh)

## Overview
This plan ports the existing Zsh `git-tools` dispatcher and its subcommands from
the pinned Zsh sources under `crates/git-cli/upstream/` (copied from
`~/.config/zsh/scripts/git/git-tools.zsh` and `~/.config/zsh/scripts/git/tools/*.zsh`) into a Rust
CLI crate inside this workspace, named `git-cli`. The goal is behavioral parity: command surface,
help/usage text, prompts/confirmations, output lines (including emojis), and exit codes should match
the Zsh implementation. The outcome includes a new `git-cli` binary, parity docs (`spec` and
`fixtures`), Zsh + Bash completions, opt-in alias/wrapper scripts, and a deterministic integration
test suite (including missing-tool fallbacks and interactive prompt flows).

## Scope
- In scope:
  - Rust `git-cli` crate + workspace wiring.
  - Command parity for groups and subcommands:
    - `utils`: `zip`, `copy-staged` (alias: `copy`), `root`, `commit-hash` (alias: `hash`)
    - `reset`: `soft`, `mixed`, `hard`, `undo`, `back-head`, `back-checkout`, `remote`
    - `commit`: `context`, `context-json` (aliases: `context_json`, `contextjson`, `json`), `to-stash` (alias: `stash`)
    - `branch`: `cleanup` (alias: `delete-merged`)
    - `ci`: `pick`
  - Parity docs:
    - `crates/git-cli/README.md`
  - Completions and opt-in alias shortcuts/wrappers:
    - `completions/zsh/_git-cli`
    - `completions/bash/git-cli`
    - `completions/zsh/aliases.zsh` and `completions/bash/aliases.bash` updates for `gx*`-style
      shortcuts (opt-in) and shell-effect wrappers (where needed).
  - Binary wrapper script (single version; dev convenience):
    - `wrappers/git-cli`
  - Comprehensive tests:
    - Integration tests under `crates/git-cli/tests/` using temp git repos and PATH-stubbing for
      external tools (clipboard helpers, `file`, etc.).
- Out of scope:
  - Porting unrelated Zsh Git helpers outside the `git-tools` family.
  - Adding new commands or changing UX beyond parity (unless explicitly documented as a limitation).
  - Re-implementing Git functionality (use the `git` CLI for parity).

## Assumptions (if any)
1. CLI interface stays structurally equivalent to `git-tools` (`git-cli GROUP COMMAND [args]`),
   with only the binary name changing from `git-tools` to `git-cli`.
2. `git-cli` shells out to `git` for all repo operations to match script behavior and edge cases.
3. Clipboard writes are best-effort (like the Zsh `set_clipboard` dependency): missing clipboard
   tools will not hard-fail the command (but will be documented).
4. Shell-effect behavior cannot be replicated by a child process; `git-cli utils root` will provide a
   safe stdout contract for alias wrappers (documented in `crates/git-cli/README.md` and implemented
   in `completions/*/aliases.*`).
5. Confirmation prompts accept only `y`/`Y` as “yes”, matching the Zsh helpers.

## Alias contract (gx*)
Opt-in aliases (in `completions/zsh/aliases.zsh` and `completions/bash/aliases.bash`) must cover the
entire `git-cli` surface:

- Base:
  - `gx` -> `git-cli`
  - `gxh` -> `git-cli help`
- Group shortcuts:
  - `gxu` -> `git-cli utils`
  - `gxr` -> `git-cli reset`
  - `gxc` -> `git-cli commit`
  - `gxb` -> `git-cli branch`
  - `gxi` -> `git-cli ci`
- utils:
  - `gxuz` -> `git-cli utils zip`
  - `gxuc` -> `git-cli utils copy-staged`
  - `gxur` -> wrapper: `eval "$(git-cli utils root --shell)"`
  - `gxuh` -> `git-cli utils commit-hash`
- reset:
  - `gxrs` -> `git-cli reset soft`
  - `gxrm` -> `git-cli reset mixed`
  - `gxrh` -> `git-cli reset hard`
  - `gxru` -> `git-cli reset undo`
  - `gxrbh` -> `git-cli reset back-head`
  - `gxrbc` -> `git-cli reset back-checkout`
  - `gxrr` -> `git-cli reset remote`
- commit:
  - `gxcc` -> `git-cli commit context`
  - `gxcj` -> `git-cli commit context-json`
  - `gxcs` -> `git-cli commit to-stash`
- branch:
  - `gxbc` -> `git-cli branch cleanup`
- ci:
  - `gxip` -> `git-cli ci pick`

## Parallelization notes
- After Task 2.1 lands, Tasks 2.3 (prompts), 2.4 (external-command utilities), and 2.5 (test harness)
  can be implemented in parallel (minimal file overlap).
- Implementation sprints can also be parallelized by group (`utils`, `reset`, `commit`, `branch`,
  `ci`) as long as shared helpers and dispatcher behavior remain stable (Tasks 2.2–2.5).
- Keep the highest-risk flows sequential to reduce blast radius: `reset undo`, `reset remote`,
  `commit to-stash`, and `ci pick`.

## Sprint 1: Parity spec + fixtures + dependency inventory
**Goal**: Make behavior explicit and define deterministic fixtures for parity testing.
**Demo/Validation**:
- Command(s): `rg -n "^# git-cli parity spec" crates/git-cli/README.md`, `rg -n "^# git-cli fixtures" crates/git-cli/README.md`
- Verify: spec enumerates every command, flags, outputs, and dependency fallback behavior.

### Task 1.1: Pin Zsh sources in-repo for traceable parity work
- **Location**:
  - `crates/git-cli/upstream/git-tools.zsh`
  - `crates/git-cli/upstream/tools/git-utils.zsh`
  - `crates/git-cli/upstream/tools/git-reset.zsh`
  - `crates/git-cli/upstream/tools/git-commit.zsh`
  - `crates/git-cli/upstream/tools/git-branch-cleanup.zsh`
  - `crates/git-cli/upstream/tools/git-pick.zsh`
- **Description**: Copy the source Zsh scripts into the repo (under `crates/git-cli/upstream/`) to pin
  a stable reference for parity and testing. Prefer fetching from `graysurf/zsh-kit` at a pinned ref
  (recorded in `crates/git-cli/README.md`), then compare to the user’s local
  `~/.config/zsh/scripts/git/git-tools.zsh` to detect any divergence that must be called out in the
  spec (and reflected in fixtures/tests).
- **Dependencies**:
  - none
- **Complexity**: 3
- **Acceptance criteria**:
  - Pinned source files exist under `crates/git-cli/upstream/` with the expected filenames.
  - `crates/git-cli/README.md` references `crates/git-cli/upstream/` as the parity baseline and
    records the upstream ref (and notes any local divergence).
  - No production code depends on these vendored scripts at runtime.
- **Validation**:
  - `ls crates/git-cli/upstream >/dev/null`

### Task 1.2: Inventory git-tools feature surface and output contract
- **Location**:
  - `crates/git-cli/upstream/git-tools.zsh`
  - `crates/git-cli/upstream/tools/git-utils.zsh`
  - `crates/git-cli/upstream/tools/git-reset.zsh`
  - `crates/git-cli/upstream/tools/git-commit.zsh`
  - `crates/git-cli/upstream/tools/git-branch-cleanup.zsh`
  - `crates/git-cli/upstream/tools/git-pick.zsh`
  - `crates/git-cli/README.md`
- **Description**: Read the pinned Zsh implementation end-to-end and produce a full inventory of
  subcommands, flags, help text, prompts, output lines, and exit codes (including “dangerous”
  operations with confirmations).
- **Dependencies**:
  - Task 1.1
- **Complexity**: 6
- **Acceptance criteria**:
  - Spec lists every group/subcommand and its argument/flag parsing.
  - Spec records user-visible strings (usage lines, warnings, success messages, abort messages).
  - Spec records exit codes for help, unknown groups/commands, parse errors, and operational errors.
  - This plan’s `Scope` command list is reconciled with the inventory (no missing subcommands).
- **Validation**:
  - `rg -n "## Commands" crates/git-cli/README.md`

### Task 1.3: External dependencies inventory and policy (Required/Optional/Eliminate)
- **Location**:
  - `crates/git-cli/README.md`
- **Description**: Inventory every external binary and sourced-script dependency used by the Zsh
  implementation, then classify each dependency with a parity policy and missing-tool behavior.
  Include deterministic testing strategy for each dependency decision.
- **Dependencies**:
  - Task 1.2
- **Complexity**: 5
- **Acceptance criteria**:
  - Spec includes a table of dependencies with one of:
    - `Required (hard fail if missing)`
    - `Optional (warn + fallback)`
    - `Eliminate (rewrite in Rust)`
  - For each dependency, spec records exact missing-tool behavior (message + exit code).
  - Spec calls out `git-scope` usage in `commit context` and the chosen handling policy.
- **Validation**:
  - `rg -n "## External dependencies" crates/git-cli/README.md`

### Task 1.4: Define fixtures and edge-case matrix for every subcommand
- **Location**:
  - `crates/git-cli/README.md`
- **Description**: Define canonical fixture scenarios with setup steps and expected output markers,
  covering all subcommands, flags, and critical edge cases (non-git repo, no staged changes, invalid
  refs, merge commits, interactive prompts, missing optional tools).
- **Dependencies**:
  - Task 1.2
  - Task 1.3
- **Complexity**: 6
- **Acceptance criteria**:
  - Fixtures cover every subcommand and every flag at least once.
  - Fixtures include interactive-input cases with explicit stdin sequences (for test harness use).
  - Fixtures include missing-tool cases for clipboard and `file` detection.
- **Validation**:
  - `rg -n "^# git-cli fixtures" crates/git-cli/README.md`

### Task 1.5: Capture upstream characterization outputs as golden fixtures
- **Location**:
  - `crates/git-cli/upstream/git-tools.zsh`
  - `crates/git-cli/README.md`
  - `crates/git-cli/tests/fixtures/upstream/README.md`
- **Description**: For each documented fixture, run the pinned upstream Zsh implementation under a
  fully controlled environment (stub `set_clipboard`, stub or pin `git-scope` output, stub `file`
  when needed) and save `stdout`, `stderr`, and `exit code` as golden fixtures. Use these golden
  fixtures as the primary parity oracle for Rust integration tests to avoid manual transcription
  drift (whitespace/newlines/stdout-vs-stderr).
- **Dependencies**:
  - Task 1.1
  - Task 1.4
- **Complexity**: 7
- **Acceptance criteria**:
  - Each fixture has a corresponding golden artifact capturing stdout/stderr/exit code.
  - Golden capture is deterministic (no network, no user git config influence).
  - Rust tests compare against golden artifacts for at least the highest-risk commands:
    `reset undo`, `reset remote`, `commit context`, `commit context-json`, `commit to-stash`, `ci pick`.
- **Validation**:
  - `ls crates/git-cli/tests/fixtures/upstream >/dev/null`

## Sprint 2: Crate scaffold + CLI surface + shared helpers
**Goal**: Create the `git-cli` crate with CLI parsing and reusable primitives for parity.
**Demo/Validation**:
- Command(s): `cargo metadata --no-deps | rg '"name": "git-cli"'`, `cargo run -p git-cli -- --help`
- Verify: help output matches Zsh usage structure and exits 0.

### Task 2.1: Create git-cli crate and workspace wiring
- **Location**:
  - `Cargo.toml`
  - `crates/git-cli/Cargo.toml`
  - `crates/git-cli/src/main.rs`
- **Description**: Add a new Rust binary crate named `git-cli`, register it as a workspace member,
  and wire a minimal `main` that prints top-level help on empty args and supports `help`.
- **Dependencies**:
  - Task 1.2
- **Complexity**: 4
- **Acceptance criteria**:
  - Workspace metadata includes `git-cli`.
  - `cargo run -p git-cli -- help` prints usage and exits 0.
- **Validation**:
  - `cargo metadata --no-deps | rg '"name": "git-cli"'`
  - `cargo run -p git-cli -- help | rg "Usage:"`

### Task 2.2: Implement group/command dispatcher parity (unknowns + group help)
- **Location**:
  - `crates/git-cli/src/main.rs`
- **Description**: Implement `git-cli GROUP help` and unknown group/command behavior mirroring the
  Zsh dispatcher, including exit codes (unknown group/command should exit 2).
- **Dependencies**:
  - Task 2.1
- **Complexity**: 5
- **Acceptance criteria**:
  - `git-cli help`, `--help`, `-h`, and empty args print top-level usage and exit 0.
  - `git-cli GROUP help` prints group usage and exits 0.
  - Unknown group/command prints an error to stderr and exits 2.
- **Validation**:
  - `cargo run -p git-cli -- help >/dev/null`
  - `cargo run -p git-cli -- nope help; test $? -eq 2`

### Task 2.3: Add shared I/O helpers (confirm prompts, menu select, stdin wiring)
- **Location**:
  - `crates/git-cli/src/prompt.rs`
  - `crates/git-cli/src/main.rs`
- **Description**: Implement reusable prompt helpers with parity semantics:
  - y/N confirmation that prints `🚫 Aborted` on decline
  - select menu used by `reset undo`
  Provide `*_with_io` variants to enable deterministic tests without a real TTY.
- **Dependencies**:
  - Task 2.1
- **Complexity**: 6
- **Acceptance criteria**:
  - Prompt helpers accept only `y`/`Y` as yes.
  - Decline prints exactly `🚫 Aborted` and returns non-zero for command-level flows.
  - Helpers are unit-tested with in-memory IO.
- **Validation**:
  - `cargo test -p git-cli --lib`

### Task 2.4: Add shared external-command utilities (run, PATH lookup, clipboard best-effort)
- **Location**:
  - `crates/git-cli/src/util.rs`
  - `crates/git-cli/src/clipboard.rs`
- **Description**: Implement shared helpers:
  - `cmd_exists` and PATH search (for missing-tool tests)
  - run external commands with captured stdout/stderr and contextual errors
  - clipboard best-effort pipeline (`pbcopy`, `wl-copy`, `xclip`) matching repo patterns
- **Dependencies**:
  - Task 2.1
  - Task 1.3
- **Complexity**: 6
- **Acceptance criteria**:
  - Clipboard helper does not fail the command when clipboard tools are missing.
  - Helpers are integration-tested via PATH-stubbed `pbcopy` (like `fzf-cli` tests).
- **Validation**:
  - `cargo test -p git-cli --lib`

### Task 2.5: Add integration test harness (env normalization + PATH stubs)
- **Location**:
  - `crates/git-cli/tests/common.rs`
  - `crates/nils-test-support/src/cmd.rs`
  - `crates/nils-test-support/src/stubs.rs`
- **Description**: Create a shared integration test harness for `git-cli`:
  - locate the built binary (`assert_cmd`-style helper or workspace pattern)
  - provide helpers to create temp git repos and local bare remotes
  - provide PATH-stubbing helpers for `pbcopy`/`wl-copy`/`xclip`, `file`, and `git-scope`
  - normalize environment to avoid user config drift (set: `HOME`, `XDG_CONFIG_HOME`,
    `GIT_CONFIG_NOSYSTEM=1`, `GIT_CONFIG_GLOBAL=/dev/null`, `GIT_PAGER=cat`, `PAGER=cat`,
    `TERM=dumb`, `TZ=UTC`, `LC_ALL=C`, and remove `GIT_TRACE*`).
- **Dependencies**:
  - Task 2.1
  - Task 1.3
- **Complexity**: 7
- **Acceptance criteria**:
  - All integration tests use the common harness (no repeated ad-hoc env setup).
  - Running `cargo test -p git-cli` is deterministic on a clean machine.
- **Validation**:
  - `cargo test -p git-cli --tests`

### Task 2.6: Add binary runner wrapper script (wrappers/git-cli)
- **Location**:
  - `wrappers/git-cli`
- **Description**: Add a single wrapper script that prefers an installed `git-cli` binary and falls
  back to `cargo run -q -p git-cli -- ...` when available (mirroring existing `wrappers/*` patterns).
- **Dependencies**:
  - Task 2.1
- **Complexity**: 2
- **Acceptance criteria**:
  - `wrappers/git-cli` exists and is executable.
  - When `git-cli` is on PATH, it `exec`s that binary; otherwise it uses `cargo run` when available.
  - Error message matches repo wrapper style when neither binary nor `cargo` are available.
- **Validation**:
  - `bash wrappers/git-cli -- help >/dev/null`

## Sprint 3: utils parity (zip/copy-staged/root/commit-hash)
**Goal**: Implement `utils` group parity with tests for outputs and error paths.
**Demo/Validation**:
- Command(s): `cargo run -p git-cli -- utils commit-hash HEAD`
- Verify: output matches `get_commit_hash` behavior for refs and annotated tags.

### Task 3.1: Implement `utils zip`
- **Location**:
  - `crates/git-cli/src/utils.rs`
  - `crates/git-cli/src/main.rs`
- **Description**: Port `git-zip`: `git archive --format zip HEAD -o backup-$SHORT_SHA.zip` in the
  current directory; errors propagate from `git`.
- **Dependencies**:
  - Task 2.2
- **Complexity**: 3
- **Acceptance criteria**:
  - Creates `backup-$SHORT_SHA.zip` matching Zsh naming.
  - Exits non-zero with an informative error when not in a git repo.
- **Validation**:
  - `cargo test -p git-cli --test utils`

### Task 3.2: Implement `utils copy-staged` (stdout/both/clipboard modes)
- **Location**:
  - `crates/git-cli/src/utils.rs`
  - `crates/git-cli/src/main.rs`
- **Description**: Port `git-copy-staged`:
  - Reads `git diff --cached --no-color`
  - Supports `--stdout` and `--both` (mutually exclusive; parse errors match script)
  - When no staged changes, prints `⚠️  No staged changes to copy` and exits 1
- **Dependencies**:
  - Task 2.4
  - Task 2.2
- **Complexity**: 6
- **Acceptance criteria**:
  - Output and exit codes match Zsh script for empty diff, unknown args, and success.
  - Clipboard mode is best-effort and prints the success summary line on success.
- **Validation**:
  - `cargo test -p git-cli --test utils`

### Task 3.3: Implement `utils root` with wrapper-friendly stdout contract
- **Location**:
  - `crates/git-cli/src/utils.rs`
  - `crates/git-cli/README.md`
  - `completions/zsh/aliases.zsh`
  - `completions/bash/aliases.bash`
- **Description**: Port `git-root` semantics with an explicit wrapper contract:
  - Default mode prints the resolved git root path and a user-friendly message.
  - Add a `--shell` mode that prints only `cd ROOT_PATH` on stdout (for `eval` wrappers) and prints
    user messaging to stderr (documented in the spec).
- **Dependencies**:
  - Task 1.1
  - Task 1.2
  - Task 2.2
- **Complexity**: 7
- **Acceptance criteria**:
  - `--shell` output is safe to `eval`, contains only a single `cd` command, and uses robust shell
    escaping (including spaces) and `cd -- ...`.
  - Spec documents limitations and wrapper usage for Zsh/Bash.
  - Aliases file defines `gxur` as an opt-in wrapper that performs the cd.
- **Validation**:
  - `cargo test -p git-cli --test utils`

### Task 3.4: Implement `utils commit-hash`
- **Location**:
  - `crates/git-cli/src/utils.rs`
  - `crates/git-cli/src/main.rs`
- **Description**: Port `get_commit_hash`: resolve `REF^{commit}` and print the full SHA to stdout.
  Error on missing ref argument should match script text and exit non-zero.
- **Dependencies**:
  - Task 2.2
- **Complexity**: 4
- **Acceptance criteria**:
  - Missing ref prints `❌ Missing git ref` and exits 1.
  - Annotated tags resolve correctly via `^{commit}`.
- **Validation**:
  - `cargo test -p git-cli --test utils`

## Sprint 4: reset parity (soft/mixed/hard/undo/back-*/remote)
**Goal**: Port reset flows with strong guardrails and deterministic prompt-driven tests.
**Demo/Validation**:
- Command(s): `cargo run -p git-cli -- reset soft 1`
- Verify: prints commit list and requires confirmation before mutating.

### Task 4.1: Implement `reset soft|mixed|hard` by-count rewinds
- **Location**:
  - `crates/git-cli/src/reset.rs`
  - `crates/git-cli/src/main.rs`
- **Description**: Port `_git_reset_by_count` behavior:
  - Validate optional `N` (positive integer) and “too many args” errors (exit 2)
  - Resolve `HEAD~N` and show `git log -n N` summary
  - Prompt for confirmation and run `git reset --MODE HEAD~N`
  - Print mode-specific preface and success/failure lines matching script
- **Dependencies**:
  - Task 2.3
  - Task 2.2
- **Complexity**: 7
- **Acceptance criteria**:
  - Text and exit codes match Zsh for invalid N, insufficient commits, decline, and success.
  - Prompts use the same wording (including emoji) as the script.
- **Validation**:
  - `cargo test -p git-cli --test reset`

### Task 4.2: Implement `reset undo` (reflog target + clean/dirty worktree flows)
- **Location**:
  - `crates/git-cli/src/reset.rs`
  - `crates/git-cli/src/main.rs`
- **Description**: Port `git-reset-undo`:
  - Validate inside git repo, resolve `HEAD@{1}` to SHA
  - Detect in-progress ops (merge/rebase/cherry-pick/revert/bisect) and require extra confirm
  - Show reflog summary lines (best-effort; fallback behavior documented)
  - If clean worktree: automatically `git reset --hard TARGET_SHA`
  - If dirty worktree: show `git status --porcelain` and present 1/2/3/4 menu for soft/mixed/hard/abort
- **Dependencies**:
  - Task 2.3
  - Task 1.1
- **Complexity**: 9
- **Acceptance criteria**:
  - All prompt text and branch behavior match the script for clean and dirty states.
  - Menu default is abort (empty input behaves as abort).
  - Tests cover: no reflog entry, clean flow, dirty flow (each menu choice), and decline confirmations.
- **Validation**:
  - `cargo test -p git-cli --test reset`

### Task 4.3: Implement `reset back-head` (checkout HEAD@{1})
- **Location**:
  - `crates/git-cli/src/reset.rs`
  - `crates/git-cli/src/main.rs`
- **Description**: Port `git-back-head`: resolve `HEAD@{1}`, print the oneline target, confirm, then
  run `git checkout HEAD@{1}` and print the success/failure messages.
- **Dependencies**:
  - Task 2.3
- **Complexity**: 6
- **Acceptance criteria**:
  - Decline prints `🚫 Aborted` and exits non-zero.
  - Checkout failures produce the same error text as the script.
- **Validation**:
  - `cargo test -p git-cli --test reset`

### Task 4.4: Implement `reset back-checkout` (previous branch from reflog)
- **Location**:
  - `crates/git-cli/src/reset.rs`
  - `crates/git-cli/src/main.rs`
- **Description**: Port `git-back-checkout`:
  - Refuse in detached HEAD with the script’s guidance text
  - Parse reflog subjects to find `checkout: moving from FROM_BRANCH to CURRENT_BRANCH`
  - Refuse when `FROM_BRANCH` looks like a commit SHA
  - Verify local branch exists before checkout, then confirm and run `git checkout FROM_BRANCH`
- **Dependencies**:
  - Task 2.3
  - Task 1.1
- **Complexity**: 8
- **Acceptance criteria**:
  - Output matches the script for all failure cases (detached, missing reflog entry, SHA-like from).
  - Successful flow checks out the branch and prints the success line.
- **Validation**:
  - `cargo test -p git-cli --test reset`

### Task 4.5: Implement `reset remote` (remote overwrite with fetch/prune/clean/upstream)
- **Location**:
  - `crates/git-cli/src/reset.rs`
  - `crates/git-cli/src/main.rs`
- **Description**: Port `git-reset-remote`:
  - Parse options: `--ref`, `-r/--remote`, `-b/--branch`, `--no-fetch`, `--prune`, `--clean`,
    `--set-upstream`, `-y/--yes`
  - Derive default `REMOTE/BRANCH` from upstream or current branch
  - Fetch (optional), verify remote-tracking ref, confirm, then `git reset --hard REF`
  - Optional `git clean -fd` flow with confirm (unless `--yes`)
  - Set upstream best-effort
- **Dependencies**:
  - Task 2.3
  - Task 1.1
- **Complexity**: 9
- **Acceptance criteria**:
  - Help output and option defaults match script.
  - Tests cover: detached HEAD refusal, missing remote ref, `--yes` bypass, and `--clean` prompt.
- **Validation**:
  - `cargo test -p git-cli --test reset`

## Sprint 5: commit parity (to-stash/context/context-json)
**Goal**: Port commit helpers with deterministic outputs and file-writing behavior.
**Demo/Validation**:
- Command(s): `cargo run -p git-cli -- commit context --stdout`
- Verify: renders Markdown sections and per-file content blocks matching Zsh output.

### Task 5.1: Implement `commit to-stash` (commit → stash conversion)
- **Location**:
  - `crates/git-cli/src/commit.rs`
  - `crates/git-cli/src/main.rs`
- **Description**: Port `git-commit-to-stash`:
  - Resolve target commit (default `HEAD`), handle merge commit first-parent prompt
  - Synthesize stash object via `git commit-tree` + `git stash store`
  - Provide fallback mode that temporarily checks out parent and applies patch (requires clean worktree)
  - Optional “drop commit from history” flow with upstream-reachability warning heuristic
- **Dependencies**:
  - Task 2.3
  - Task 1.1
- **Complexity**: 9
- **Acceptance criteria**:
  - Creates stash entry with message format matching script.
  - Root-commit refusal matches script text.
  - Tests cover: normal commit, merge commit prompt path, fallback refusal when worktree dirty, and
    decline prompts.
- **Validation**:
  - `cargo test -p git-cli --test commit`

### Task 5.2: Implement `commit context` (Markdown context + include patterns)
- **Location**:
  - `crates/git-cli/src/commit.rs`
  - `crates/git-cli/src/main.rs`
  - `crates/git-cli/README.md`
- **Description**: Port `git-commit-context`:
  - Modes: default clipboard, `--stdout`, `--both`
  - `--no-color` behavior (affects `git-scope staged` invocation or internal scope generation)
  - `--include` patterns (repeatable) for lockfile content visibility
  - Per-file section format (rename display `old -> new`, deleted-file HEAD fallback, ` ```ts ` fences)
  - Binary detection parity (numstat + optional `file` MIME probe if available)
- **Dependencies**:
  - Task 2.4
  - Task 1.2
- **Complexity**: 10
- **Acceptance criteria**:
  - Output headings and section separators match the script (including emojis).
  - Lockfile-hiding and include override match script output text.
  - Missing `git-scope` (if treated optional) has documented and tested behavior.
- **Validation**:
  - `cargo test -p git-cli --test commit`

### Task 5.3: Implement `commit context-json` (manifest + patch files + bundle output)
- **Location**:
  - `crates/git-cli/src/commit_json.rs`
  - `crates/git-cli/src/main.rs`
  - `crates/git-cli/README.md`
- **Description**: Port `git-commit-context-json`:
  - Always write `OUT_DIR/staged.patch` and `OUT_DIR/commit-context.json`
  - Modes: default clipboard, `--stdout`, `--both`
  - `--pretty` formatting (2-space indent) and stable key ordering
  - `--bundle` output separators and inclusion of patch content
  - Default out-dir: `GIT_DIR/commit-context`
- **Dependencies**:
  - Task 2.4
  - Task 1.1
- **Complexity**: 9
- **Acceptance criteria**:
  - JSON schema and values match the script (including null-vs-string behavior).
  - Bundle output matches separators and ordering from Zsh.
  - Tests verify file writes and JSON stability across runs.
- **Validation**:
  - `cargo test -p git-cli --test commit`

## Sprint 6: branch + ci parity (cleanup/pick) + completions
**Goal**: Finish remaining groups, add completions, and ensure full-suite parity coverage.
**Demo/Validation**:
- Command(s): `cargo test -p git-cli`, `zsh -f tests/zsh/completion.test.zsh`
- Verify: all commands have deterministic integration tests and completion scripts source cleanly.

### Task 6.1: Implement `branch cleanup` (merged + squash modes)
- **Location**:
  - `crates/git-cli/src/branch.rs`
  - `crates/git-cli/src/main.rs`
- **Description**: Port `git-delete-merged-branches`:
  - Options: `-b/--base`, `-s/--squash`, help output
  - Protected branches set and current/base branch protection
  - Candidate selection from `git for-each-ref --merged` and `git cherry -v` in squash mode
  - Confirmation prompt and delete with `-d` or `-D` parity rules
- **Dependencies**:
  - Task 2.3
  - Task 1.1
- **Complexity**: 8
- **Acceptance criteria**:
  - Output lists candidates and matches script wording for empty results.
  - Tests cover merged mode, squash mode, and protection rules.
- **Validation**:
  - `cargo test -p git-cli --test branch`

### Task 6.2: Implement `ci pick` (CI branch cherry-pick + push workflow)
- **Location**:
  - `crates/git-cli/src/ci.rs`
  - `crates/git-cli/src/main.rs`
- **Description**: Port `git-pick`:
  - Parse options: `-r/--remote`, `--no-fetch`, `-f/--force`, `--stay`
  - Require clean index/worktree and refuse in-progress git ops
  - Resolve base ref from remote/local/commit, resolve commit spec (single or range)
  - Create/reset CI branch `ci/TARGET/NAME`, cherry-pick commits, push (with lease on force)
  - Print cleanup instructions and switch back unless `--stay`
- **Dependencies**:
  - Task 2.3
  - Task 1.1
- **Complexity**: 9
- **Acceptance criteria**:
  - Branch naming and remote selection match the script for common cases.
  - Tests use a local bare remote to validate push without network access.
  - Failure cases print the same error text as Zsh (usage, op-in-progress, dirty tree, cherry-pick fail).
- **Validation**:
  - `cargo test -p git-cli --test ci`

### Task 6.3: Add Zsh + Bash completions and opt-in aliases for git-cli
- **Location**:
  - `completions/zsh/_git-cli`
  - `completions/bash/git-cli`
  - `completions/zsh/aliases.zsh`
  - `completions/bash/aliases.bash`
  - `tests/zsh/completion.test.zsh`
- **Description**: Add completion scripts for `git-cli` mirroring existing patterns in this repo, and
  extend alias files with the opt-in `gx*` shortcut surface (full mapping is documented in this
  plan under “Alias contract (gx*)”). Include a wrapper function for `utils root` (cd effect) for
  both shells.
- **Dependencies**:
  - Task 2.2
  - Task 3.3
- **Complexity**: 6
- **Acceptance criteria**:
  - `tests/zsh/completion.test.zsh` sources `_git-cli` successfully and verifies `_git-cli` exists.
  - Aliases do not clobber user-defined aliases/functions (match existing guard style).
  - Wrapper functions and alias contract documented in `crates/git-cli/README.md`.
- **Validation**:
  - `zsh -f tests/zsh/completion.test.zsh`

### Task 6.4: Add full edge-case integration test suite anchored to fixtures
- **Location**:
  - `crates/git-cli/tests/edge_cases.rs`
  - `crates/git-cli/tests/common.rs`
  - `crates/git-cli/README.md`
- **Description**: Implement comprehensive integration tests for every group/subcommand, including:
  - unknown group/command + exit codes
  - missing optional tools on PATH (`pbcopy`/`wl-copy`/`xclip`, `file`)
  - non-git repo behavior and invalid refs
  - interactive flows (`reset undo` menu, confirmation prompts)
  - remote-based tests using a local bare remote repo (`ci pick`, `reset remote`)
- **Dependencies**:
  - Task 1.3
  - Task 6.1
  - Task 6.2
  - Task 5.3
  - Task 1.5
- **Complexity**: 10
- **Acceptance criteria**:
  - Every documented fixture has at least one deterministic test.
  - Tests are stable in CI without network access or a real TTY.
- **Validation**:
  - `cargo test -p git-cli`

### Task 6.5: Pre-delivery checks (repo gates)
- **Location**:
  - `DEVELOPMENT.md`
  - `./.codex/skills/nils-cli-checks/scripts/nils-cli-checks.sh`
- **Description**: Run required formatting, lint, unit/integration tests, and Zsh completion tests
  via the repo’s single entrypoint. Fix any failures within scope before final delivery.
- **Dependencies**:
  - Task 6.4
- **Complexity**: 4
- **Acceptance criteria**:
  - All checks pass with exit code 0.
- **Validation**:
  - `./.codex/skills/nils-cli-checks/scripts/nils-cli-checks.sh`

## Testing Strategy
- Unit:
  - Prompt helpers (`confirm_with_io`, menu parsing) and JSON formatting helpers (stable ordering).
  - Include-pattern matching for `commit context`.
- Integration:
  - Temp git repos for all git-mutating commands (`reset`, `branch cleanup`, `commit to-stash`).
  - PATH-stubbing for clipboard commands and `file` (missing-tool + deterministic behavior).
  - Local bare remote repos for `ci pick` and `reset remote`.
- E2E/manual:
  - Manual spot-check of dangerous flows (`reset remote --clean`, `commit to-stash` drop flow) in a
    disposable repo.

## Risks & gotchas
- Parent-shell mutation: `utils root` cannot `cd` without wrappers; spec + alias wrappers must be
  explicit to avoid surprising users.
- Interactive prompts: exact wording and default choices must match; tests must feed stdin reliably.
- Git version differences: reflog subject formats and certain porcelain outputs can vary; spec should
  call out what is matched exactly vs best-effort parsing.
- `file` MIME output differs by platform; prefer an internal “binary heuristic” with `file` as an
  optional enhancement and test both paths via PATH-stubbing.
- `ci pick` pushes and branch name validation are sensitive; tests should use local bare remotes and
  avoid network-dependent assumptions.

## Rollback plan
- If parity or safety issues appear post-merge:
  - Remove `crates/git-cli/` and workspace membership, revert completion/aliases/wrapper additions.
  - Since alias scripts are opt-in, disabling sourcing immediately mitigates user impact.
  - Retain `docs/plans/git-cli-rust-port-plan.md` for audit trail.
