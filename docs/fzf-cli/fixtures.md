# fzf-cli fixtures

## help and unknown command
- Command: `fzf-cli help`
- Expect: prints Usage line and Commands list; exit 0.
- Command: `fzf-cli nope`
- Expect: prints `❗ Unknown command: nope` then `Run 'fzf-cli help' for usage.`; exit 1.

## file open-with flags
- Unknown flag:
  - Command: `fzf-cli file --nope`
  - Expect: stderr contains `❌ Unknown flag: --nope`; exit 2.
- Mutual exclusion:
  - Command: `fzf-cli file --vi --vscode`
  - Expect: stderr contains `❌ Flags are mutually exclusive: --vi and --vscode`; exit 2.

## directory cd emission contract
- Setup: repo with directories and files.
- Command: `fzf-cli directory`
- Expect: when cd action is chosen, stdout prints `cd <dir>` for evaluation in the parent shell.

## history parsing
- Setup: temp HISTFILE with extended Zsh history lines `: <epoch>:<dur>;<cmd>`.
- Expect: filters empty/punctuation-only commands and strips leading icon prefixes.

## process kill flags
- Setup: stub `ps` output and stub `kill` that records args.
- Default:
  - Command: `fzf-cli process`
  - Expect: prompts for kill confirmation; declining prints `🚫 Aborted.` and exits non-zero.
- Immediate:
  - Command: `fzf-cli process -k`
  - Expect: prints SIGTERM kill line and calls `kill` once per PID.
- Immediate force:
  - Command: `fzf-cli process -k -9`
  - Expect: prints SIGKILL kill line and calls `kill -9`.

## port lsof vs netstat fallback
- With lsof:
  - Setup: stub `lsof` with TCP LISTEN output including PIDs.
  - Expect: selected PIDs are deduped and passed to kill flow.
- Without lsof:
  - Setup: remove `lsof` from PATH, stub `netstat` output.
  - Expect: view-only selection does not attempt to kill processes.

## git commands outside repo
- Commands: `fzf-cli git-branch`, `fzf-cli git-tag`, `fzf-cli git-commit`
- Expect: stderr contains `❌ Not inside a Git repository. Aborting.`; exit 1.

## git-checkout stash retry
- Setup: git repo with local changes that cause checkout to fail.
- Command: `fzf-cli git-checkout`
- Expect: on checkout failure, prompts for stash; confirming stashes and retries checkout.

## git-commit snapshot extraction
- Setup: repo with a commit containing a file that no longer exists in worktree.
- Command: `fzf-cli git-commit --snapshot`
- Expect: snapshot extraction to temp file succeeds and temp file is removed after editor exits.

## def commands require delimiter env
- Command: `fzf-cli env` (or `alias`/`function`/`def`) without `FZF_DEF_DELIM` and `FZF_DEF_DELIM_END`.
- Expect: prints the two-line error/help text; exit 1.

