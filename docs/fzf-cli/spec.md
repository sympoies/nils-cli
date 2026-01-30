# fzf-cli parity spec

## Overview
`fzf-cli` is a Rust port of the Zsh `fzf-tools` dispatcher defined in `~/.config/zsh/scripts/fzf-tools.zsh`.
It provides a single entry point with multiple interactive subcommands powered by `fzf`.
Most subcommands shell out to external tools (`fzf`, `git`, `ps`, `lsof`, `netstat`, `kill`, editors)
to preserve behavior and output parity with the Zsh implementation.

## Entry point
- Command: `fzf-cli <command> [args]`
- Help: `fzf-cli help`, `fzf-cli --help`, `fzf-cli -h`, or empty args.

## Commands (dispatcher)
- `file` : search files and open selection in an editor
- `directory` : two-step directory picker then file picker; supports an emit-cd flow
- `git-status` : interactive `git status -s` viewer with diff preview
- `git-commit` : browse commits and open changed files (worktree or snapshot)
- `git-checkout` : pick a commit and checkout (with optional stash retry)
- `git-branch` : browse and checkout branches
- `git-tag` : browse and checkout tags
- `process` : browse processes and optionally kill selected PIDs
- `port` : browse listening ports (via `lsof`) and optionally kill owning PIDs
- `history` : browse Zsh history and return the selected command
- `env` : browse environment variables with preview (block mode)
- `alias` : browse indexed alias definitions with preview (block mode)
- `function` : browse indexed function definitions with preview (block mode)
- `def` : browse env+alias+function in a single list (block mode)

## Help output and guardrails
- Help output format matches the Zsh dispatcher (Usage, blank line, Commands list).
- Unknown command prints:
  - `❗ Unknown command: <cmd>`
  - `Run 'fzf-cli help' for usage.`
  and exits with code 1.

## Shared prompts (confirmation)
`fzf-cli` uses the same confirmation rules as the script:
- A confirmation prompt succeeds only when the user inputs `y` or `Y`.
- Declining prints `🚫 Aborted.` and exits non-zero.

## File opener behavior (file/directory/git-commit)
### Open-with selection
- Flags: `--vi` and `--vscode` (mutually exclusive).
- Default opener comes from `FZF_FILE_OPEN_WITH` (`vi` default).
- `--` ends option parsing; remaining args are treated as query tokens.
- Unknown `--flag` errors:
  - stderr: `❌ Unknown flag: --flag`
  - exit code: 2
- Mutual exclusion errors:
  - stderr: `❌ Flags are mutually exclusive: --vi and --vscode`
  - exit code: 2

### VSCode open behavior
- Uses `code --goto` when `--vscode` is selected.
- For git-root workspaces, VSCode is launched with the workspace root and `--goto` path.
- If VSCode open fails (including missing `code`), it prints:
  - `❌ Failed to open in VSCode; falling back to vi`
  and falls back to `vi`.

## Shell interop limitations (directory/history)
Some Zsh behaviors are not possible for a child process to apply to the parent shell session.
`fzf-cli` provides an output contract to enable wrappers to preserve parity:

### directory
- The directory picker supports a “cd action” (triggered by the same key flow as the script).
- When the user chooses the cd action, `fzf-cli directory` prints a `cd ...` shell command on stdout
  and exits 0.
- When the user chooses an open-file action, it opens the file and prints nothing.

Recommended wrapper:
```zsh
fzf-directory() { eval "$(fzf-cli directory -- "$@")"; }
```

### history
- `fzf-cli history` prints the selected command on stdout (with the same icon-prefix stripping as the
  script) and exits 0.

Recommended wrapper:
```zsh
fzf-history() { eval "$(fzf-cli history -- "$@")"; }
```

## process
- Flags:
  - `-k`, `--kill`: kill immediately with SIGTERM (no prompts)
  - `-9`, `--force`: use SIGKILL (SIGKILL requires either `-k -9` or interactive confirmation)
- Default flow (no `-k`): prompt `Kill PID(s): ...? [y/N] ` then prompt `Force SIGKILL (-9)? [y/N] `.
- Kill messages:
  - SIGTERM: `☠️  Killing PID(s) with SIGTERM: ...`
  - SIGKILL: `☠️  Killing PID(s) with SIGKILL: ...`

## port
- Uses `lsof -nP -iTCP -sTCP:LISTEN` when `lsof` is available.
- Without `lsof`, falls back to `netstat` in view-only mode (no kill dispatch).

## git-branch and git-tag
- Outside a Git repository:
  - stderr: `❌ Not inside a Git repository. Aborting.`
  - exit code: 1
- Prompts for confirmation before `git checkout`.
- Branch checkout success: `✅ Checked out to <branch>`
- Tag checkout success: `✅ Checked out to tag <tag> (commit <hash>)`
- Tag resolution failure: `❌ Could not resolve tag '<tag>' to a commit hash.`

## git-checkout
- Uses the commit picker and confirms before checkout:
  - `🚚 Checkout to commit <hash>? [y/N] `
- On checkout failure:
  - prints `⚠️  Checkout to '<hash>' failed. Likely due to local changes.`
  - prompts `📦 Stash your current changes and retry checkout? [y/N] `
- On stash confirm:
  - runs `git stash push -u -m "<stash_msg>"`
  - prints `📦 Changes stashed: <stash_msg>`

## git-commit
- Requires Git repository; otherwise prints the same abort message as git-branch.
- Supports `--snapshot`:
  - When enabled, the default open action opens a snapshot temp file extracted via `git show`.
- Worktree file open behavior:
  - When selected file exists: opens it.
  - When missing: prints `❌ File no longer exists in working tree: <path>` then prompts
    `🧾 Open snapshot from <commit> instead? [y/N] `
- Snapshot extraction:
  - Tries `git show <commit>:<path>`, then `git show <commit>^:<path>`
  - Failure prints `❌ Failed to extract snapshot: <commit>:<path> (or <commit>^:<path>)`
- When opening all files in VSCode:
  - If `open-changed-files` exists, it is used; otherwise falls back to `code --new-window`.

## env/alias/function/def (block preview mode)
- Requires both env vars:
  - `FZF_DEF_DELIM`
  - `FZF_DEF_DELIM_END`
- If either is missing:
  - prints:
    - `❌ Error: FZF_DEF_DELIM or FZF_DEF_DELIM_END is not set.`
    - `💡 Please export FZF_DEF_DELIM and FZF_DEF_DELIM_END before running.`
  - exits 1
- After selection, prints the selected block and attempts to copy it to clipboard (best-effort).

## Definition indexing and caching
- First-party files scanned:
  - `${ZDOTDIR:-$HOME/.config/zsh}/.zshrc`
  - `${ZDOTDIR:-$HOME/.config/zsh}/.zprofile`
  - zsh files under `${ZDOTDIR:-$HOME/.config/zsh}/{scripts,bootstrap,tools}`
- Optional persistent doc cache:
  - `FZF_DEF_DOC_CACHE_ENABLED=true` enables persistent cache.
  - `FZF_DEF_DOC_CACHE_EXPIRE_MINUTES` controls TTL (default 10 minutes).

