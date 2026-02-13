# git-cli

## Overview
git-cli is a Rust CLI that groups Git workflow helpers behind a dispatcher. It provides five command
groups: utils, reset, commit, branch, and ci.

## Usage
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
```

## Commands

### utils
- `zip`: Create `backup-<short-sha>.zip` from `HEAD` using `git archive`.
- `copy-staged` (`copy`): Copy staged diff to the clipboard. Use `--stdout` to print, `--both` to
  print and copy.
- `root`: Print the repository root. Use `--shell` to output `cd -- <path>` for `eval`.
- `commit-hash` (`hash`): Resolve a ref to a commit SHA.

### reset
- `soft|mixed|hard [N]`: Rewind `HEAD` by N commits (default: 1) with confirmations and summaries.
- `undo`: Move `HEAD` back to the previous reflog entry with safety checks.
- `back-head`: Checkout `HEAD@{1}` (previous position).
- `back-checkout`: Checkout the previously checked-out branch (requires non-detached `HEAD`).
- `remote`: Reset the current branch to a remote-tracking ref.
  Options: `--ref <remote/branch>`, `--remote <name>`, `--branch <name>`, `--no-fetch`, `--prune`,
  `--set-upstream`, `--clean`, `-y/--yes`.

### commit
- `context`: Build a Markdown commit context from staged changes.
  Options: `--stdout`, `--both`, `--no-color` (or `NO_COLOR`), `--include <path/glob>` (repeatable).
- `context-json`: Write `commit-context.json` and `staged.patch` (default:
  `<git-dir>/commit-context`).
  Options: `--stdout`, `--both`, `--pretty`, `--bundle`, `--out-dir <path>`.
- `to-stash`: Create a stash from a commit and optionally rewrite history via prompts.

### branch
- `cleanup` (`delete-merged`): Delete merged local branches.
  Options: `-b/--base <ref>`, `-s/--squash`.

### ci
- `pick`: Create and push a `ci/<target>/<name>` branch with cherry-picked commits.
  Options: `-r/--remote <name>`, `--no-fetch`, `-f/--force`, `--stay`.

## Shell aliases (optional)
- Zsh aliases live in `completions/zsh/aliases.zsh`.
- Bash aliases live in `completions/bash/aliases.bash`.
- `gxur` should be implemented via: `eval "$(git-cli utils root --shell)"`.

## Exit codes
- `0`: Success and help output.
- `1`: Operational errors or aborted confirmations.
- `2`: Usage/parse errors.

## Dependencies
- `git` is required for all commands.
- `git-scope` is required for `commit context`.
- Clipboard tools are optional: `pbcopy`, `wl-copy`, `xclip`, or `xsel`. Missing clipboard tools
  emit a warning and continue.

## Docs

- [Docs index](docs/README.md)
