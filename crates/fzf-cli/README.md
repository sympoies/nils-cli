# fzf-cli

## Overview
fzf-cli is a Rust CLI that provides interactive pickers for files, Git metadata, processes, ports,
and shell definitions, all powered by `fzf`.

## Usage
```text
Usage:
  fzf-cli <command> [args]

Commands:
  file         Search and preview files
  directory    Browse directories and pick files
  git-status   Interactive git status viewer
  git-commit   Browse commits and open changed files
  git-checkout Pick and checkout a previous commit
  git-branch   Browse and checkout branches
  git-tag      Browse and checkout tags
  process      Browse and kill running processes
  port         Browse listening ports and owners
  history      Search command history
  env          Browse environment variables
  alias        Browse shell aliases
  function     Browse shell functions
  def          Browse env, alias, and function definitions
  help         Display help message

Help:
  fzf-cli help
  fzf-cli --help
```

## Commands

### file
- `file [--vi|--vscode] [-- <query...>]`: Search files with preview and open the selection.

### directory
- `directory [--vi|--vscode] [-- <query...>]`: Pick a directory, then pick a file to open. Use
  `ctrl-d` to emit `cd <path>` to stdout.

### git-status
- `git-status [query...]`: Interactive `git status -s` viewer with diff previews.

### git-commit
- `git-commit [--snapshot] [query...]`: Browse commits and open changed files. `--snapshot` opens
  file snapshots from the selected commit by default.

### git-checkout
- `git-checkout [query...]`: Pick a commit and checkout (prompts before checkout).

### git-branch
- `git-branch [query...]`: Browse branches and checkout (prompts before checkout).

### git-tag
- `git-tag [query...]`: Browse tags and checkout (prompts before checkout).

### process
- `process [-k|--kill] [-9|--force] [query...]`: Browse processes and optionally kill selected PIDs.

### port
- `port [-k|--kill] [-9|--force] [query...]`: Browse listening ports and optionally kill owning PIDs.

### history
- `history [query...]`: Browse shell history and print the selected command to stdout.

### env
- `env [query...]`: Browse environment variables.

### alias
- `alias [query...]`: Browse shell aliases.

### function
- `function [query...]`: Browse shell functions.

### def
- `def [query...]`: Browse env, alias, and function definitions.

## Environment
- `FZF_FILE_OPEN_WITH`: Default opener for `file`, `directory`, `git-commit` (`vi` or `vscode`).
- `FZF_FILE_MAX_DEPTH`: Max directory depth for `file` and `directory` (default: `10`).
- `FZF_PREVIEW_WINDOW`: Preview window layout for `directory` (default: `right:50%:wrap`).
- `FZF_DEF_DELIM` and `FZF_DEF_DELIM_END`: Required delimiters for `env`, `alias`, `function`, `def`.
- `FZF_DEF_DOC_CACHE_ENABLED`: Enable definition doc caching.
- `FZF_DEF_DOC_CACHE_EXPIRE_MINUTES`: Cache TTL in minutes (default: `10`).
- `FZF_DEF_DOC_SEPARATOR_PAD`: Padding lines between definition docs (default: `2`).

## Dependencies
- `fzf` is required for all commands.
- `git` is required for `git-*` commands.
- `lsof` is optional for `port` (falls back to `netstat`).
- `code` is required for `--vscode` (falls back to `vi`).
