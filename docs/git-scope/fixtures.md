# git-scope fixtures

## tracked prefix filtering
- Setup: git repo with multiple files under `scripts/`, `docs/`, and root.
- Command: `git-scope tracked ./scripts`.
- Expect: output contains `-\tscripts/git/git-scope.zsh` and only files under `scripts/`.

## staged print sources
- Setup: repo with files `only_staged.txt`, `only_unstaged.txt`, `both.txt`.
- Stage `only_staged.txt` and `both.txt` (with different index/worktree content).
- Command: `git-scope staged -p`.
- Expect: prints `📄 both.txt (index)` and includes index content; no worktree content.

## all print sources
- Setup: same as staged fixture.
- Command: `git-scope all -p`.
- Expect: staged-only prints index only; unstaged-only prints worktree only; both prints both.

## untracked listing
- Setup: create new untracked file not in `.gitignore`.
- Command: `git-scope untracked`.
- Expect: file appears with `U` status.

## commit basic
- Setup: commit with file edits.
- Command: `git-scope commit HEAD`.
- Expect: header metadata, commit message section, per-file stats, total summary, tree output.

## commit with print
- Setup: commit with text file in history.
- Command: `git-scope commit HEAD -p`.
- Expect: `📦 Printing file contents` and `📄 <file> (from HEAD)` or `(working tree)`.

## merge parent selection
- Setup: create merge commit with two parents and divergent changes.
- Command: `git-scope commit <merge> --parent 2`.
- Expect: `ℹ️  Merge commit with 2 parents — showing diff against parent #2`.
- Invalid: `--parent 9` should warn and fall back to parent #1.

## no-color mode
- Setup: any repo with changes.
- Command: `NO_COLOR=1 git-scope staged`.
- Expect: output contains no ANSI escape codes.

## tree missing
- Setup: run with PATH missing `tree`.
- Command: `PATH=/nope git-scope staged` (with git in PATH overridden appropriately).
- Expect: `⚠️  tree is not installed...` warning and no tree output.

## tree unsupported
- Setup: simulate `tree` without `--fromfile` (or mock a stub that exits non-zero).
- Expect: `⚠️  tree does not support --fromfile...` warning.

## binary file printing
- Setup: commit or staged binary file.
- Command: `git-scope staged -p`.
- Expect: `📄 <file> (binary file in index)` and placeholder line.
