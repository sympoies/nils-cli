# git-lock

## Overview

git-lock saves commit hashes under labels so you can reset, diff, copy, delete, or tag a stored
commit. It operates per repository and prompts before destructive actions.

## Usage

```text
Usage:
  git-lock <command> [args]

Commands:
  lock [label] [note] [commit]  Save commit hash to lock
  unlock [label]               Reset to a saved commit
  list                         Show all locks for repo
  copy <from> <to>             Duplicate a lock label
  delete [label]               Remove a lock
  diff <l1> <l2> [--no-color]  Compare commits between two locks
  tag <label> <tag> [-m msg]   Create git tag from a lock
  help                         Show help
```

## Commands

- `lock [label] [note] [commit]`: Save a commit hash under a label. Defaults: label `default`,
  commit `HEAD`.
- `unlock [label]`: Hard reset to a locked commit. If omitted, uses the latest label.
- `list`: List locks for the current repository (newest first).
- `copy <from> <to>`: Duplicate a lock label.
- `delete [label]`: Delete a lock label. If omitted, uses the latest label.
- `diff <label1> <label2> [--no-color]`: Show `git log` between two locked commits.
- `tag <label> <tag> [-m <msg>] [--push]`: Create an annotated tag at a locked commit. Use
  `--push` to push the tag to `origin` and delete the local tag.
- `help`: Show help output.

## Exit codes

- `0`: Success and help output.
- `1`: Operational errors or aborted confirmations.

## Dependencies

- `git` is required for all commands.

## Environment

- `ZSH_CACHE_DIR`: Base directory for lock storage. Locks are stored under
  `$ZSH_CACHE_DIR/git-locks`. If unset, defaults to `/git-locks`.
- `NO_COLOR`: Disable color for `diff` output.

## Docs

- [Docs index](docs/README.md)
