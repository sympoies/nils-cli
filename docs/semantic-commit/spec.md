# semantic-commit parity spec

## Overview
`semantic-commit` is a small helper CLI that ports the Codex Zsh entrypoints:
- `~/.codex/skills/semantic-commit/scripts/staged_context.sh`
- `~/.codex/skills/semantic-commit/scripts/commit_with_message.sh`

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
