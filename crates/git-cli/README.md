# git-cli

Upstream (Zsh) references:
- Repo: https://github.com/graysurf/zsh-kit
- Pinned sources (vendored for parity work): `crates/git-cli/upstream/`
- Upstream ref (pinned): `zsh-kit@83f0ef41b9ba16ac6ea0987e83685f35621f7c1e`
- Local divergence vs `~/.config/zsh/scripts/git/*`: none detected (byte-identical)

Notes:
- This directory is created in Sprint 1 to hold the parity spec + fixtures + vendored upstream sources.
- The Rust crate + workspace wiring lands in Sprint 2 (see `docs/plans/git-cli-rust-port-plan.md`).
- Vendored Zsh sources are **reference-only** and must not be used by the Rust binary at runtime.

# git-cli parity spec

## Purpose
`git-cli` is a Rust port of the Zsh `git-tools` dispatcher and its subcommands, targeting behavioral
parity:
- Same command surface (groups/subcommands + aliases)
- Same prompts/confirmations and user-visible output lines (including emojis)
- Same exit codes and stdout/stderr conventions

Naming note:
- The upstream dispatcher is named `git-tools`; this port’s binary is `git-cli`.
- When an upstream string contains `git-tools ...` usage, the Rust port should print `git-cli ...`
  instead. Other tool names (`git-reset-remote`, `git-pick`, etc.) remain as-is for parity.

## Conventions

### Streams
- Errors are printed to stderr when upstream uses `print -u2`.
- Most informational output is printed to stdout.

### Exit codes
- `0`: success and `--help` flows.
- `1`: operational errors and user-declined confirmations (`🚫 Aborted`).
- `2`: usage/parse errors and unknown group/command in the dispatcher.

## Alias contract (gx*)

Opt-in shell aliases live in `completions/zsh/aliases.zsh` and `completions/bash/aliases.bash`.
Source them from your shell init to enable the shortcuts. They must not clobber user-defined aliases
or functions.

`git-cli utils root` cannot change the parent shell; the `gxur` wrapper must use:
`eval "$(git-cli utils root --shell)"`.

Alias mapping:
- `gx` -> `git-cli`
- `gxh` -> `git-cli help`
- `gxu` -> `git-cli utils`
- `gxr` -> `git-cli reset`
- `gxc` -> `git-cli commit`
- `gxb` -> `git-cli branch`
- `gxi` -> `git-cli ci`
- `gxuz` -> `git-cli utils zip`
- `gxuc` -> `git-cli utils copy-staged`
- `gxur` -> wrapper: `eval "$(git-cli utils root --shell)"`
- `gxuh` -> `git-cli utils commit-hash`
- `gxrs` -> `git-cli reset soft`
- `gxrm` -> `git-cli reset mixed`
- `gxrh` -> `git-cli reset hard`
- `gxru` -> `git-cli reset undo`
- `gxrbh` -> `git-cli reset back-head`
- `gxrbc` -> `git-cli reset back-checkout`
- `gxrr` -> `git-cli reset remote`
- `gxcc` -> `git-cli commit context`
- `gxcj` -> `git-cli commit context-json`
- `gxcs` -> `git-cli commit to-stash`
- `gxbc` -> `git-cli branch cleanup`
- `gxip` -> `git-cli ci pick`

## Commands
This section inventories the upstream Zsh behavior (vendored under `crates/git-cli/upstream/`) and
defines the parity contract for the Rust port.

Scope check:
- The command surface matches `docs/plans/git-cli-rust-port-plan.md` (no missing subcommands).

### Top-level dispatcher

#### Help output (top-level)
Upstream: `_git_tools_usage` (prints `git-tools ...`); Rust port should print the same content with
`git-cli ...`:

```text
Usage:
  git-cli <group> <command> [args]

Groups:
  utils    zip | copy-staged | root | commit-hash
  reset    soft | mixed | hard | undo | back-head | back-checkout | remote
  commit   context | context-json | to-stash
  branch   cleanup
  ci       pick

Help:
  git-cli help
  git-cli <group> help

Examples:
  git-cli utils zip
  git-cli reset hard 3
```

#### Group usage output
Upstream: `_git_tools_group_usage <group>` (prints `git-tools ...`); Rust port should print:

```text
Usage: git-cli utils <command> [args]
  zip | copy-staged | root | commit-hash
```

```text
Usage: git-cli reset <command> [args]
  soft | mixed | hard | undo | back-head | back-checkout | remote
```

```text
Usage: git-cli commit <command> [args]
  context | context-json | to-stash
```

```text
Usage: git-cli branch <command> [args]
  cleanup
```

```text
Usage: git-cli ci <command> [args]
  pick
```

#### Dispatcher behavior
- `git-cli` / `git-cli help` / `git-cli --help` / `git-cli -h`:
  - prints top-level usage and exits `0`.
- `git-cli <group>` / `git-cli <group> help` / `git-cli <group> --help` / `git-cli <group> -h`:
  - prints group usage for known groups and exits `0`.
  - unknown group:
    - stderr: `Unknown group: <group>`
    - prints top-level usage
    - exits `2`.
- `git-cli <group> <unknown-cmd> ...`:
  - stderr: `Unknown <group> command: <cmd>`
  - prints group usage
  - exits `2`.

### utils

#### zip
- Invocation: `git-cli utils zip`
- Upstream implementation: `git-zip` (shells out)
- Behavior:
  - Runs: `git archive --format zip HEAD -o "backup-<short-sha>.zip"`
  - Propagates the `git archive` exit code.

#### copy-staged (alias: copy)
- Invocation: `git-cli utils copy-staged [--stdout|--both]` (`copy` is an alias)
- Upstream implementation: `git-copy-staged`
- Flags:
  - `--stdout|-p|--print`: print staged diff to stdout; no “copied” status line.
  - `--both`: print to stdout and copy to clipboard.
  - `--help|-h`: prints:
    ```text
    Usage: git-copy-staged [--stdout|--both]
      --stdout   Print staged diff to stdout (no status message)
      --both     Print to stdout and copy to clipboard
    ```
- Errors and messages:
  - Conflicting mode flags:
    - stderr: `❗ Only one output mode is allowed: --stdout or --both`
    - exits `1`.
  - Unknown arg:
    - stderr: `❗ Unknown argument: <arg>`
    - stderr: `Usage: git-copy-staged [--stdout|--both]`
    - exits `1`.
  - No staged diff:
    - stdout: `⚠️  No staged changes to copy`
    - exits `1`.
  - Clipboard/both success:
    - stdout: `✅ Staged diff copied to clipboard`
    - exits `0`.

#### root
- Invocation: `git-cli utils root [--shell]`
- Upstream implementation: `git-root` (shell-effect; `cd` + prints root)
- Flags:
  - `--shell`: prints `cd -- <root>` to stdout (safe for `eval`) and prints the user-facing message
    to stderr.
- Behavior:
  - Not in a repo:
    - stderr: `❌ Not in a git repository`
    - exits `1`.
  - Success prints a leading blank line, then:
    - stdout: `📁 Jumped to Git root: <absolute-path>`
    - exits `0`.
  - `--shell` success prints:
    - stdout: `cd -- <shell-escaped-path>`
    - stderr: `📁 Jumped to Git root: <absolute-path>`
    - exits `0`.
- Parity note: the Rust binary cannot `cd` the parent shell; wrappers/aliases must implement the
  shell-effect contract (see plan’s “Alias contract (gx*)”), typically via
  `eval "$(git-cli utils root --shell)"`.

#### commit-hash (alias: hash)
- Invocation: `git-cli utils commit-hash <ref>` (`hash` is an alias)
- Upstream implementation: `get_commit_hash`
- Behavior:
  - Missing ref:
    - stderr: `❌ Missing git ref`
    - exits `1`.
  - Otherwise prints the resolved commit SHA for `<ref>^{commit}` (annotated tags supported) and
    exits with `git rev-parse`’s status.

### reset

#### soft / mixed / hard
- Invocation:
  - `git-cli reset soft [N]`
  - `git-cli reset mixed [N]`
  - `git-cli reset hard [N]` (dangerous)
- Upstream implementation: `_git_reset_by_count` via `git-reset-soft|mixed|hard`
- Args:
  - `N`: positive integer commit count (default: `1`)
- Usage/parse errors (exit `2`):
  - Missing mode: stderr `❌ Missing reset mode.`
  - Too many args: stderr `❌ Too many arguments.` + `Usage: git-reset-<mode> [N]`
  - Invalid N: stderr `❌ Invalid commit count: <arg> (must be a positive integer).` + usage line
  - Unknown mode: stderr `❌ Unknown reset mode: <mode>`
- Operational errors (exit `1`):
  - Cannot resolve target: stderr `❌ Cannot resolve HEAD~<N> (not enough commits?).`
- Confirmation prompt and abort:
  - Decline prints: `🚫 Aborted` and exits `1`.
- Success messages (exit `0`):
  - soft:  `✅ Reset completed. Your changes are still staged.`
  - mixed: `✅ Reset completed. Your changes are now unstaged.`
  - hard:  `✅ Hard reset completed. HEAD moved back to HEAD~<N>.`

#### undo
- Invocation: `git-cli reset undo`
- Upstream implementation: `git-reset-undo`
- Key behavior and strings:
  - Not in a repo: `❌ Not a git repository.`
  - In-progress operation detection:
    - prints `🛡️  Detected an in-progress Git operation:` + list
    - prompt: `❓ Still run git-reset-undo (move HEAD back)? [y/N] `
  - Reflog display (best-effort):
    - prints `🧾 Current HEAD@{0} (last action):` and `🧾 Target  HEAD@{1} ...`
    - may print `ℹ️  Reflog display unavailable here; reset target is still the resolved SHA: <sha>`
  - If last action is not `reset:*`:
    - prints `⚠️  The last action does NOT look like a reset operation.`
    - prompt: `❓ Still proceed to move HEAD back to the previous HEAD position? [y/N] `
  - Clean tree fast-path:
    - prints `✅ Working tree clean. Proceeding with: git reset --hard <sha>`
    - on success: `✅ Repository reset back to previous HEAD: <sha>`
  - Dirty tree menu:
    - prompt: `❓ Select [1/2/3/4] (default: 4): `
    - default/abort prints `🚫 Aborted`

#### back-head
- Invocation: `git-cli reset back-head`
- Upstream implementation: `git-back-head`
- Behavior:
  - Prints a summary + prompt:
    - `⏪ This will move HEAD back to the previous position (HEAD@{1}):`
    - `❓ Proceed with 'git checkout HEAD@{1}'? [y/N] `
  - Decline prints `🚫 Aborted` and exits `1`.

#### back-checkout
- Invocation: `git-cli reset back-checkout`
- Upstream implementation: `git-back-checkout`
- Behavior highlights:
  - Detached HEAD refusal:
    - `❌ You are in a detached HEAD state. This function targets branch-to-branch checkouts.`
  - Confirmation prompt:
    - `❓ Proceed with 'git checkout <branch>'? [y/N] `
  - Decline prints `🚫 Aborted` and exits `1`.

#### remote
- Invocation: `git-cli reset remote [options]`
- Upstream implementation: `git-reset-remote`
- Help (`--help|-h`) prints `git-reset-remote: ...` header, usage examples, and options; exits `0`.
- Options:
  - `--ref <remote/branch>` (must contain `/`; else exits `2`)
  - `-r, --remote <name>`
  - `-b, --branch <name>`
  - `--no-fetch`
  - `--prune`
  - `--clean` (may prompt before `git clean -fd` unless `-y`)
  - `--set-upstream`
  - `-y, --yes` (skip confirmations)
- Success string:
  - `✅ Done. '<current-branch>' now matches '<remote>/<branch>'.`

### commit

#### context
- Invocation: `git-cli commit context [--stdout|--both] [--no-color] [--include <path/glob>]`
- Upstream implementation: `git-commit-context`
- Help (`--help|-h`) prints usage/options for `git-commit-context`; exits `0`.
- Notable behaviors:
  - No staged changes: stderr `⚠️  No staged changes to record` and exits `1`.
  - Default output is copied to clipboard via `set_clipboard` (best-effort).
  - Output is a Markdown document with these stable section headings:
    - `# Commit Context`
    - `## 📂 Scope and file tree:`
    - `## 📄 Git staged diff:`
    - `## 📚 Staged file contents (index version):`

#### context-json (aliases: context_json, contextjson, json)
- Invocation: `git-cli commit context-json [--stdout|--both] [--pretty] [--bundle] [--out-dir <path>]`
- Upstream implementation: `git-commit-context-json`
- Help (`--help|-h`) prints usage/options for `git-commit-context-json`; exits `0`.
- Notable behaviors:
  - Writes:
    - `<out-dir>/commit-context.json`
    - `<out-dir>/staged.patch`
  - No staged changes: stderr `⚠️  No staged changes to record` and exits `1`.
  - `--bundle` prints/copies a combined output with markers:
    - `===== commit-context.json =====`
    - `===== staged.patch =====`

#### to-stash (alias: stash)
- Invocation: `git-cli commit to-stash [commit]`
- Upstream implementation: `git-commit-to-stash`
- Behavior highlights:
  - Not in a repo: `❌ Not a git repository.`
  - Aborts print: `🚫 Aborted`
  - Success (stash created):
    - `✅ Stash created: ...` (or `✅ Stash created (fallback): ...`)
  - Optional history rewrite prompts:
    - `❓ Drop commit from history now? [y/N] `
    - `❓ Final confirmation: run 'git reset --hard <parent>'? [y/N] `

### branch

#### cleanup (alias: delete-merged)
- Invocation: `git-cli branch cleanup [-b|--base <ref>] [-s|--squash]`
- Upstream implementation: `git-delete-merged-branches`
- Help (`-h|--help`) prints usage/options; exits `0`.
- Prompts:
  - `❓ Proceed with deleting these branches? [y/N] `
  - Decline prints `🚫 Aborted` and exits `1`.

### ci

#### pick
- Invocation: `git-cli ci pick [options] <target> <commit-or-range> <name>`
- Upstream implementation: `git-pick`
- Help (`-h|--help`) prints usage/options; exits `0`.
- Options:
  - `-r, --remote <name>`
  - `--no-fetch`
  - `-f, --force`
  - `--stay`
- Parse/usage errors exit `2` (e.g., missing args).

## External dependencies

This table inventories dependencies used by the upstream Zsh implementation and records the Rust
port’s parity policy.

Legend (policy):
- **Required**: hard fail if missing.
- **Optional**: warn + fallback (continue).
- **Eliminate**: implement in Rust (no runtime dependency).

| Dependency | Used by (upstream) | Policy (Rust) | Missing-tool behavior (Rust) | Test strategy |
| --- | --- | --- | --- | --- |
| `git` | all commands | Required | stderr: `❗ git is required but was not found in PATH.`; exit `1` | PATH-stub in integration tests |
| `git-scope` | `commit context` | Required (initial parity) | stderr: `❗ git-scope is required but was not found in PATH.`; exit `1` | PATH-stub + golden fixture |
| Clipboard tool (`pbcopy`/`xclip`/`xsel`) | `utils copy-staged`, `commit context`, `commit context-json` (clipboard/both) | Optional | stderr: `⚠️  No clipboard tool found (requires pbcopy, xclip, or xsel)`; fallback: skip copy; exit `0` | PATH-stub + golden fixture (missing-clipboard mode) |
| `file` | `commit context` (binary sniff) | Optional | no warning; skip MIME probe (binary detection via numstat only) | Golden fixtures for stub + missing `file` |
| `date` | `commit context-json` (`generatedAt`) | Eliminate | N/A | Inject deterministic time in tests |
| `mktemp` | `commit context` temp file | Eliminate | N/A | Rust uses in-memory buffers in tests |
| Zsh module `zsh/zutil` | `branch cleanup`, `ci pick`, `reset remote` option parsing | Eliminate | N/A | Rust uses clap; unit tests for parsing |
| `sed` / `wc` / `tr` / `grep` / `head` / `basename` | various helpers | Eliminate | N/A | Rust parsing unit tests |

### `git-scope` policy note (required in spec)
- Upstream `commit context` shells out to `git-scope staged` and strips ANSI codes.
- The Rust port treats `git-scope` as **required** for initial parity (missing → hard fail, exit `1`),
  with a future option to eliminate the external dependency by extracting/reusing the `git-scope`
  implementation as a library.

# git-cli fixtures

These fixtures define deterministic scenarios for parity testing. Sprint 1 focuses on documenting the
fixture matrix and capturing upstream characterization outputs as golden artifacts under
`crates/git-cli/tests/fixtures/upstream/`.

## Fixture environment (controlled)
Golden captures run the vendored upstream Zsh scripts (`crates/git-cli/upstream/`) under a controlled
environment:
- Stub `set_clipboard` to avoid touching the real system clipboard.
- Stub `git-scope` output (or pin it) to avoid drift.
- Stub `date -u` to make `generatedAt` deterministic for `commit context-json`.
- Stub or remove `file` to cover both binary-detection and missing-tool behavior.

## Canonical fixture matrix

Each fixture specifies:
- **Setup**: deterministic repo state (commits/branches/staged changes/remotes)
- **Run**: the `git-cli ...` invocation
- **stdin**: explicit input for prompts (when interactive)
- **Expect**: output markers + exit code + any side effects

### Dispatcher (top-level)

#### F001: Top-level help
- Setup: none
- Run: `git-cli help`
- Expect:
  - prints the top-level usage block (see parity spec)
  - exit `0`

### utils

#### F010: utils zip (happy path)
- Setup:
  - repo with at least 1 commit
- Run: `git-cli utils zip`
- Expect:
  - creates `backup-<short-sha>.zip` in CWD
  - exit `0`

#### F011: utils copy-staged (clipboard success)
- Setup:
  - repo with staged textual change
  - clipboard stub enabled (success)
- Run: `git-cli utils copy-staged --both`
- Expect:
  - stdout includes the staged diff
  - stdout contains: `✅ Staged diff copied to clipboard`
  - exit `0`

#### F012: utils copy-staged (--stdout)
- Setup:
  - repo with staged textual change
- Run: `git-cli utils copy-staged --stdout`
- Expect:
  - stdout is the staged diff (no trailing “copied” status line)
  - exit `0`

#### F013: utils root
- Setup:
  - repo nested at least one directory deep
- Run: `git-cli utils root`
- Expect:
  - stdout contains:
    - a leading blank line
    - `📁 Jumped to Git root: <absolute-path>`
  - exit `0`

#### F014: utils commit-hash (missing ref)
- Setup: none
- Run: `git-cli utils commit-hash`
- Expect:
  - stderr: `❌ Missing git ref`
  - exit `1`

### reset

#### F020: reset soft (abort)
- Setup:
  - repo with at least 2 commits
- Run: `git-cli reset soft 1`
- stdin: `n\n`
- Expect:
  - stdout includes `🧾 Commits to be rewound:`
  - stdout includes `🚫 Aborted`
  - exit `1`

#### F021: reset mixed (invalid count)
- Setup: none
- Run: `git-cli reset mixed 0`
- Expect:
  - stderr includes `❌ Invalid commit count: 0 (must be a positive integer).`
  - exit `2`

#### F022: reset hard (confirm)
- Setup:
  - repo with at least 2 commits
- Run: `git-cli reset hard 1`
- stdin: `y\n`
- Expect:
  - stdout includes `❓ Are you absolutely sure? [y/N] `
  - stdout includes `✅ Hard reset completed. HEAD moved back to HEAD~1.`
  - exit `0`

#### F023: reset undo (dirty tree menu; abort)
- Setup:
  - repo with at least 2 commits
  - working tree dirty (tracked change)
- Run: `git-cli reset undo`
- stdin: `\n` (default)
- Expect:
  - stdout includes `Choose how to proceed:`
  - stdout includes `🚫 Aborted`
  - exit `1`

#### F024: reset back-head (abort)
- Setup:
  - repo with reflog history (at least one checkout/reset action)
- Run: `git-cli reset back-head`
- stdin: `n\n`
- Expect:
  - stdout includes `❓ Proceed with 'git checkout HEAD@{1}'? [y/N] `
  - stdout includes `🚫 Aborted`
  - exit `1`

#### F025: reset back-checkout (detached HEAD refusal)
- Setup:
  - repo in detached HEAD state
- Run: `git-cli reset back-checkout`
- Expect:
  - stdout includes `❌ You are in a detached HEAD state. This function targets branch-to-branch checkouts.`
  - exit `1`

#### F026: reset remote (full flags; yes mode)
- Setup:
  - repo with remote `origin` and remote-tracking branch `origin/main`
  - untracked files present (to exercise `--clean`)
- Run: `git-cli reset remote --ref origin/main -r origin -b main --no-fetch --prune --clean --set-upstream -y`
- Expect:
  - stdout includes `✅ Done. '<branch>' now matches 'origin/main'.`
  - exit `0`

### commit

#### F030: commit context (--both, lockfile hidden + include)
- Setup:
  - repo with staged changes including:
    - a lockfile (e.g., `yarn.lock`)
    - a small text file
  - `git-scope` stubbed/pinned
  - `file` stubbed to force one file to be treated as binary (`charset=binary`)
- Run: `git-cli commit context --both --no-color --include yarn.lock`
- Expect:
  - stdout starts with `# Commit Context`
  - output includes:
    - `## 📂 Scope and file tree:`
    - `## 📄 Git staged diff:`
    - `## 📚 Staged file contents (index version):`
    - `[Binary file content hidden]`
  - exit `0`

#### F031: commit context (missing clipboard tool; best-effort)
- Setup:
  - repo with staged changes
  - clipboard stub set to “missing tool”
- Run: `git-cli commit context`
- Expect:
  - stderr includes: `⚠️  No clipboard tool found (requires pbcopy, xclip, or xsel)`
  - command still completes (exit `0`)

#### F032: commit context (missing `file` tool)
- Setup:
  - repo with staged binary change (so `git diff --numstat` reports `-`)
  - `file` missing from `PATH`
- Run: `git-cli commit context --stdout`
- Expect:
  - output includes: `[Binary file content hidden]`
  - exit `0`

#### F033: commit context-json (--stdout bundle + out-dir)
- Setup:
  - repo with staged changes
  - `date -u` stubbed to a fixed timestamp
- Run: `git-cli commit context-json --stdout --pretty --bundle --out-dir ./out/commit-context`
- Expect:
  - stdout includes `===== commit-context.json =====`
  - stdout includes `===== staged.patch =====`
  - JSON contains fixed `generatedAt`
  - exit `0`

#### F034: commit to-stash (non-merge; keep history)
- Setup:
  - repo with at least 2 commits
- Run: `git-cli commit to-stash HEAD`
- stdin:
  - `y\n` (create stash)
  - `n\n` (do not drop commit)
- Expect:
  - stdout includes `🧾 Convert commit → stash`
  - stdout includes `✅ Stash created:`
  - stdout includes `✅ Done. Commit kept; stash saved.`
  - exit `0`

### branch

#### F040: branch cleanup (squash mode; abort)
- Setup:
  - repo with:
    - base branch `main`
    - feature branch merged (or squash-applied) into `main`
- Run: `git-cli branch cleanup --base main --squash`
- stdin: `n\n`
- Expect:
  - stdout includes `🧹 Branches to delete`
  - stdout includes `🚫 Aborted`
  - exit `1`

### ci

#### F050: ci pick (local remote; force + stay)
- Setup:
  - repo with a local bare remote `origin`
  - at least 1 commit to cherry-pick
- Run: `git-cli ci pick --remote origin --no-fetch --force --stay main HEAD ci-test`
- Expect:
  - stdout includes `🌿 CI branch: ci/main/ci-test`
  - stdout includes `✅ Pushed: origin/ci/main/ci-test`
  - exit `0`
