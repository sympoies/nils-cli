# semantic-commit

## Overview
semantic-commit validates commit messages and commits staged changes. It can also emit staged
change context for message generation.

## Usage
```text
Usage:
  semantic-commit <command> [args]

Commands:
  staged-context  Print staged change context for commit message generation
  commit          Commit staged changes with a prepared commit message
  help            Display help message

Help:
  semantic-commit help
  semantic-commit --help
```

## Commands

### staged-context
- `staged-context`: Emit a bundle to stdout containing `commit-context.json` and `staged.patch`.

### commit
- `commit [--message <text> | --message-file <path>]`: Validate and commit staged changes. When no
  flags are provided, the message is read from stdin.

## Commit Message Validation
- Header must be non-empty, `<= 100` characters, and use a lowercase type.
- Header format: `type(scope): subject` or `type: subject`.
- If a body exists, line 2 must be blank and each body line must start with `- ` followed by an
  uppercase letter and be `<= 100` characters.

## Exit codes
- `0`: success and help output.
- `1`: usage errors, validation failures, or operational errors.
- `2`: no staged changes.

## Dependencies
- `git` is required.
- `git-scope` is required to print the commit summary after a successful commit.
