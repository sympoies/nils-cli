# semantic-commit parity spec

## Overview
`semantic-commit` is a small helper CLI that ports the Codex Zsh entrypoints:
- `https://github.com/graysurf/codex-kit/blob/main/skills/tools/devex/semantic-commit/scripts/staged_context.sh`
- `https://github.com/graysurf/codex-kit/blob/main/skills/tools/devex/semantic-commit/scripts/commit_with_message.sh`

It provides a single binary with two subcommands: `staged-context` and `commit`.

## Entry point
- Command: `semantic-commit <command> [args]`
- Help: `semantic-commit help`, `semantic-commit --help`, `semantic-commit -h`, or empty args.

## Commands

### staged-context
Usage: `semantic-commit staged-context`

Purpose: Print staged change context for commit message generation.

Preconditions:
- Must run inside a Git work tree.
- Must have staged changes (index is non-empty).

Output:
- Prints a single bundle to stdout:
  - `===== commit-context.json =====` then JSON (compact).
  - blank line.
  - `===== staged.patch =====` then the staged patch (from `git diff --cached --no-color`).

Errors / guardrails:
- Help: `semantic-commit staged-context --help` prints usage and exits `0`.
- Unknown argument: prints `error: unknown argument: <arg>` to stderr, prints usage, exits `1`.
- Not in a git work tree: prints `error: must run inside a git work tree` to stderr, exits `1`.
- No staged changes: prints `error: no staged changes (stage files with git add first)` to stderr,
  exits `2`.

### commit
Usage: `semantic-commit commit [--message <text> | --message-file <path>]`

Purpose: Read a prepared commit message, validate it, run `git commit`, then print a commit summary.

Inputs (exactly one source):
- `--message <text>`: single argument string.
- `--message-file <path>`: read the entire file as the commit message.
- stdin (preferred for multi-line messages): read full stdin into the commit message.

Preconditions:
- Must run inside a Git work tree.
- Must have staged changes (index is non-empty).

Commit message validation
Validation is a hard-fail (exit `1`) and follows the source scripts:

- Message must be non-empty.
- Header is the first line:
  - Non-empty.
  - Max 100 characters.
  - Must match regex: `^[a-z][a-z0-9-]*(\\([a-z0-9._-]+\\))?: .+$`
    - Examples:
      - `feat(core): add thing`
      - `chore: update deps`
- Body is optional. If any non-empty line exists after the header, body rules apply:
  - Line 2 must be blank (one blank line after header).
  - Every subsequent line must:
    - Be non-empty (no blank lines anywhere in the body).
    - Be max 100 characters.
    - Start with `- ` followed by an uppercase letter (`^- [A-Z]`).

Errors / guardrails:
- Help: `semantic-commit commit --help` prints usage and exits `0`.
- Unknown argument: prints `error: unknown argument: <arg>` to stderr, prints usage, exits `1`.
- Both `--message` and `--message-file`: prints
  `error: use only one of --message or --message-file` to stderr, exits `1`.
- Missing value for `--message`: prints `error: --message requires a value` to stderr, prints usage,
  exits `1`.
- Missing value for `--message-file`: prints `error: --message-file requires a path` to stderr,
  prints usage, exits `1`.
- Message file missing: prints `error: message file not found: <path>` to stderr, exits `1`.
- No message provided and stdin is a TTY: prints
  `error: no commit message provided (use stdin, --message, or --message-file)` to stderr, prints
  usage, exits `1`.
- Not in a git work tree: prints `error: must run inside a git work tree` to stderr, exits `1`.
- No staged changes: prints `error: no staged changes (stage files with git add first)` to stderr,
  exits `2`.
- Missing `git-scope` on `PATH`: prints `error: git-scope is required ...` to stderr, exits `1`
  (and does not run `git commit`).
- Git commit failures: prints `error: git commit failed (exit code: <rc>)` to stderr, exits `<rc>`.

Success behavior (summary output):
- Sets `GIT_PAGER=cat` and `PAGER=cat` for subprocess calls.
- After a successful `git commit`, prints a commit summary by running:
  - `git-scope commit HEAD --no-color`
- If `git-scope commit` fails:
  - Prints `error: git-scope commit failed (exit code: <rc>)` to stderr, exits `<rc>`.

## Exit codes
- `0`: success.
- `2`: no staged changes.
- `1`: invalid usage, validation failure, or not inside a git work tree.
- Other non-zero: propagate `git commit` exit code.

## Tool resolution
- `semantic-commit` resolves external tools via `PATH`:
  - `git` is required.
  - `git-scope` is required (used for commit summary output).
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
