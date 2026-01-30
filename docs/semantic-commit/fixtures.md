# semantic-commit fixtures

## staged-context prints bundle
- Setup: temp git repo with at least one staged change.
- Command: `semantic-commit staged-context`.
- Expect:
  - exit `0`.
  - stderr is empty.
  - stdout contains `===== commit-context.json =====` and `===== staged.patch =====`.
  - stdout contains the staged patch (from `git diff --cached --no-color`).

## staged-context: summary counts + flags
- Setup: temp git repo with staged changes:
  - root text file (e.g. `README.md`),
  - lockfile (e.g. `package-lock.json`),
  - subdir text file (e.g. `src/lib.rs`),
  - binary file (e.g. `assets/logo.bin` with NUL bytes).
- Command: `semantic-commit staged-context`.
- Expect:
  - commit-context.json summary:
    - `fileCount=4`, `rootFileCount=2`, `lockfileCount=1`, `binaryFileCount=1`,
      `topLevelDirCount=2`, `insertions=3`, `deletions=0`.
  - statusCounts contains `{status:"A", count:4}`.
  - topLevelDirs contains `{name:"assets", count:1}` and `{name:"src", count:1}`.
  - file entry for `package-lock.json` has `lockfile=true`, `binary=false`.
  - file entry for `assets/logo.bin` has `binary=true` and null insertions/deletions.

## staged-context: rename captures oldPath
- Setup: temp git repo; commit `old.txt`, rename to `new.txt`, stage rename.
- Command: `semantic-commit staged-context`.
- Expect:
  - commit-context.json includes a file entry with `path="new.txt"`, `status="R"`,
    and `oldPath="old.txt"`.

## staged-context: no staged changes
- Setup: temp git repo with clean index.
- Command: `semantic-commit staged-context`.
- Expect: stderr contains `error: no staged changes (stage files with git add first)` and exit `2`.

## staged-context: outside git repo
- Setup: temp dir (not a git repo).
- Command: `semantic-commit staged-context`.
- Expect: stderr contains `error: must run inside a git work tree` and exit `1`.

## commit: fails when git-scope is missing
- Setup: temp git repo with staged change; ensure `git-scope` is not on `PATH`.
- Command: pipe a valid multi-line message into `semantic-commit commit`.
- Expect:
  - exit `1`.
  - stderr contains `error: git-scope is required`.
  - no commit is created.

## commit: fails when git-scope is not executable
- Setup: temp git repo with staged change; put a non-executable `git-scope` on `PATH`.
- Command: `semantic-commit commit --message "feat(core): add thing"`.
- Expect:
  - exit `1`.
  - stderr contains `error: git-scope is required`.
  - no commit is created.

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
