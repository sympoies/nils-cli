# git-lock

Upstream (Zsh) references:
- Script: https://github.com/graysurf/zsh-kit/blob/main/scripts/git/git-lock.zsh
- Completion: https://github.com/graysurf/zsh-kit/blob/main/scripts/_completion/_git-lock
- Docs: https://github.com/graysurf/zsh-kit/blob/main/docs/cli/git-lock.md

# git-lock parity spec

## Overview
`git-lock` is a lightweight commit-locking CLI that stores per-repo lock files under
`$ZSH_CACHE_DIR/git-locks`. It lets you save a commit hash under a label, list or diff labels, reset
back to a saved commit, copy/delete labels, and create tags. Output is emoji-heavy and includes
confirmation prompts for destructive actions.

## Commands
- `git-lock lock [label] [note] [commit]` : save a commit hash under a label (defaults: label
  `default`, commit `HEAD`).
- `git-lock unlock [label]` : reset hard to the commit stored by a label (defaults to latest label).
- `git-lock list` : list all locks for the current repo, newest-first.
- `git-lock copy <from> <to>` : duplicate a lock label (source may fall back to latest).
- `git-lock delete [label]` : delete a lock label (defaults to latest label).
- `git-lock diff <label1> <label2> [--no-color]` : show `git log` between two saved commits.
- `git-lock tag <label> <tag> [-m <msg>] [--push]` : create an annotated tag at a locked commit.
- `git-lock help` / `--help` / `-h` : show usage and command list.

## Lock file format and storage
- Directory: `$ZSH_CACHE_DIR/git-locks`.
- Per-repo label file: `<repo>-<label>.lock`.
- Latest marker: `<repo>-latest` containing the label name.
- File contents:
  - Line 1: `<commit-hash> # <optional note>`
  - Line 2: `timestamp=YYYY-MM-DD HH:MM:SS`

## Output / prompts
- Lock success:
  - `🔐 [<repo>:<label>] Locked: <hash>` and optional `# <note>`
  - `    at <timestamp>`
- Unlock flow:
  - `🔐 Found [<repo>:<label>] → <hash>` plus optional `# <note>` and `commit message: <subject>`
  - Prompt: `⚠️  Hard reset to [<label>]? [y/N] `
  - Cancelled prompt prints `🚫 Aborted`.
  - Success prints `⏪ [<repo>:<label>] Reset to: <hash>`.
- List output:
  - `🔐 git-lock list for [<repo>]:` followed by per-label blocks:
    - ` - 🏷️  tag:     <label>` with `⭐ (latest)` when label matches latest marker.
    - `   🧬 commit:  <hash>`
    - Optional `📄 message`, `📝 note`, `📅 time` lines.
- Copy output:
  - `📋 Copied git-lock [<repo>:<src>] → [<repo>:<dst>]` with metadata lines.
  - Overwrite prompt: `⚠️  Target git-lock [<repo>:<dst>] already exists. Overwrite? [y/N] `
- Delete output:
  - Prints candidate block before prompt.
  - Prompt: `⚠️  Delete this git-lock? [y/N] `
  - Success: `🗑️  Deleted git-lock [<repo>:<label>]` and optional `🧼 Removed latest marker`.
- Diff output:
  - Header lines:
    - `🧮 Comparing commits: [<repo>:<label1>] → [<label2>]`
    - `   🔖 <label1>: <hash1>`
    - `   🔖 <label2>: <hash2>`
  - Runs `git log --oneline --graph --decorate` with `--color=never` when `--no-color` or
    `NO_COLOR` is set.
- Tag output:
  - Warn if tag exists, prompt `❓ Overwrite it? [y/N] `, delete then recreate.
  - Success:
    - `🏷️  Created tag [<tag>] at commit [<hash>]`
    - `📝 Message: <msg>`
  - `--push` prints `🚀 Pushed tag [<tag>] to origin` then `🧹 Deleted local tag [<tag>]`.

## Errors / guardrails
- Outside git repo: `❗ Not a Git repository. Run this command inside a Git project.`
- Missing/invalid commit: `❌ Invalid commit: <value>`.
- Missing latest label:
  - Unlock: `❌ No recent git-lock found for <repo>`
  - Delete: `❌ No label provided and no latest git-lock exists`
- Missing lock label:
  - Unlock: `❌ No git-lock named '<label>' found for <repo>`
  - Delete: `❌ git-lock [<label>] not found`
  - Copy: `❌ Source git-lock [<repo>:<label>] not found`
  - Diff: `❌ git-lock [<label>] not found for [<repo>]`
- Diff usage errors:
  - `❗ Too many labels provided (expected 2)` and usage line.
  - Missing second label prints `❗ Second label not provided or found`.
- Unknown command:
  - `❗ Unknown command: '<cmd>'` followed by `Run 'git-lock help' for usage.`
# git-lock fixtures

## Lock default label + HEAD
- Setup:
  - Init temp repo with at least one commit.
  - Set `ZSH_CACHE_DIR` to a temp dir.
- Command:
  - `git-lock lock`
- Expected markers:
  - `🔐 [<repo>:default] Locked:`
  - `at YYYY-MM-DD HH:MM:SS`
  - Lock file created at `$ZSH_CACHE_DIR/git-locks/<repo>-default.lock`.

## Lock with explicit label + note + commit
- Setup:
  - Two commits in repo; capture `HEAD~1` hash.
- Command:
  - `git-lock lock wip "before refactor" HEAD~1`
- Expected markers:
  - Output includes `# before refactor`.
  - Lock file first line includes `<hash> # before refactor`.

## Unlock uses latest marker
- Setup:
  - Create two locks; ensure `<repo>-latest` points to the second label.
- Command:
  - `git-lock unlock` then respond `n` to prompt.
- Expected markers:
  - `🔐 Found [<repo>:<latest>] → <hash>`
  - `⚠️  Hard reset to [<label>]? [y/N]`
  - `🚫 Aborted`.

## Unlock missing latest
- Setup:
  - Ensure no `<repo>-latest` file.
- Command:
  - `git-lock unlock`
- Expected markers:
  - `❌ No recent git-lock found for <repo>`

## List ordering + latest marker
- Setup:
  - Create two locks with distinct timestamps (or adjust file timestamps).
- Command:
  - `git-lock list`
- Expected markers:
  - Newest lock appears first.
  - Latest label shows `⭐ (latest)`.

## Copy label overwrite prompt
- Setup:
  - Existing labels `a` and `b`.
- Command:
  - `git-lock copy a b` then respond `n`.
- Expected markers:
  - `⚠️  Target git-lock [<repo>:b] already exists. Overwrite? [y/N]`
  - `🚫 Aborted`.

## Delete latest cleanup
- Setup:
  - Latest marker points to label `wip`.
- Command:
  - `git-lock delete wip` then respond `y`.
- Expected markers:
  - `🗑️  Deleted git-lock [<repo>:wip]`
  - `🧼 Removed latest marker`.

## Diff no-color
- Setup:
  - Two labels pointing at different commits.
- Command:
  - `git-lock diff a b --no-color`
- Expected markers:
  - `🧮 Comparing commits: [<repo>:a] → [b]`
  - `git log` output without ANSI color sequences.

## Tag overwrite prompt
- Setup:
  - Existing tag `v1.0.0`.
- Command:
  - `git-lock tag <label> v1.0.0` then respond `n`.
- Expected markers:
  - `⚠️  Git tag [v1.0.0] already exists.`
  - `❓ Overwrite it? [y/N] `
  - `🚫 Aborted`.

## Not a git repo
- Setup:
  - Run in a temp directory without `.git`.
- Command:
  - `git-lock list`
- Expected markers:
  - `❗ Not a Git repository. Run this command inside a Git project.`
