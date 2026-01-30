# semantic-commit fixtures

## staged-context fallback (no git-commit-context-json)
- Setup: temp git repo with at least one staged change.
- Command: `semantic-commit staged-context`.
- Expect:
  - stderr contains `warning: printing fallback staged diff only`.
  - stdout contains the staged patch (from `git diff --staged --no-color`).

## staged-context: no staged changes
- Setup: temp git repo with clean index.
- Command: `semantic-commit staged-context`.
- Expect: stderr contains `error: no staged changes (stage files with git add first)` and exit `2`.

## staged-context: outside git repo
- Setup: temp dir (not a git repo).
- Command: `semantic-commit staged-context`.
- Expect: stderr contains `error: must run inside a git work tree` and exit `1`.

## commit: stdin success (git-scope missing → git show fallback)
- Setup: temp git repo with staged change; ensure no `git-scope` in Codex commands dir.
- Command: pipe a valid multi-line message into `semantic-commit commit`.
- Expect:
  - exit `0`.
  - stderr contains `warning: git-scope not found; falling back to git show --stat`.
  - stdout contains `git show --stat` output for `HEAD`.

## commit: no staged changes
- Setup: temp git repo with clean index.
- Command: `semantic-commit commit --message "chore: test"`.
- Expect: stderr contains `error: no staged changes (stage files with git add first)` and exit `2`.

## commit: outside git repo
- Setup: temp dir (not a git repo).
- Command: `semantic-commit commit --message "chore: test"`.
- Expect: stderr contains `error: must run inside a git work tree` and exit `1`.

## commit: invalid header format
- Setup: temp git repo with staged change.
- Command: `semantic-commit commit --message "Feat: bad"`.
- Expect: stderr contains `error: invalid header format ...` and exit `1`.

## commit: header too long
- Setup: temp git repo with staged change.
- Command: header length > 100 chars.
- Expect: stderr contains `error: commit header exceeds 100 characters (max 100)` and exit `1`.

## commit: body requires blank line
- Setup: temp git repo with staged change.
- Command: message with line 2 non-empty.
- Expect: stderr contains `error: commit body must be separated from header by a blank line`.

## commit: body line formatting
- Setup: temp git repo with staged change.
- Command: body lines that are empty, too long, or not starting with `- [A-Z]`.
- Expect: stderr contains the corresponding `error: commit body line <n> ...` message and exit `1`.
