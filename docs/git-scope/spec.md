# git-scope parity spec

## Overview
`git-scope` is a Git status/commit inspection CLI that renders a change list plus a directory tree.
It supports working tree and historical commit views, with optional file-content printing.
Output is emoji-heavy and colorized unless `--no-color` or `NO_COLOR` is set.

## Commands
- `git-scope tracked [prefix...]` : list all tracked files (optionally filtered by prefix).
- `git-scope staged` : list staged changes.
- `git-scope unstaged` : list unstaged changes.
- `git-scope all` : list staged + unstaged changes (unique union).
- `git-scope untracked` : list untracked files (respecting `.gitignore`).
- `git-scope commit <commit-ish> [--parent <n>]` : inspect a historical commit.

## Common flags
- `-p`, `--print` : print file contents after the tree (worktree/index as appropriate).
- `--no-color` : disable ANSI colors (also via `NO_COLOR`).

## Output sections (worktree/index)
- `📄 Changed files:` followed by lines:
  - `  ➔ [KIND] path` (tracked uses `-` as KIND; rename shows `src -> dest`).
- `📂 Directory tree:` rendered via `tree --fromfile`.
- `📦 Printing file contents:` section when `-p` is used.

## Tree rendering behavior
- If no files to render: `⚠️ No files to render as tree`.
- If `tree` missing: `⚠️  tree is not installed. Install it to see the directory tree.`
- If `tree --fromfile` unsupported: `⚠️  tree does not support --fromfile. Please upgrade tree to enable directory tree output.`

## File-content printing behavior
- Worktree: `📄 <path> (working tree)` or `(binary file in working tree)`.
- Index: `📄 <path> (index)` or `(binary file in index)`.
- Fallback to HEAD when file missing in worktree/index:
  - `📄 <path> (from HEAD)`
  - `📄 <path> (deleted in index; from HEAD)`
  - Binary variants replace the label text accordingly.
- Missing file: `❗ File not found: <path>` or `❗ File not found in index: <path>`.

## Commit mode behavior
- Header:
  - `🔖 <short-hash> <subject>`
  - `👤 <author> <email>`
  - `📅 <date>`
- Commit message section:
  - `📝 Commit Message:` followed by indented body (first line and subsequent lines prefixed with spaces).
- File list:
  - `📄 Changed files:` then lines:
    - `  ➤ [KIND] path  [+A / -D]` (with totals on a separate line).
  - `📊 Total: +N / -M` after file list.
- Merge commits:
  - Uses `--parent/-P` to select parent index.
  - Invalid values emit warnings and fall back to parent #1.
  - If no file changes vs selected parent: prints a `ℹ️` line and exits that section.

## Color behavior
- Default colors map A/M/D/U/- to fixed ANSI color codes (matches Zsh mapping).
- `--no-color` or `NO_COLOR` disables ANSI sequences for all sections.

## Errors / guardrails
- Outside git repo: `⚠️ Not a Git repository. Run this command inside a Git project.`
- Unknown subcommand: `⚠️ Unknown subcommand: '<sub>'`.
