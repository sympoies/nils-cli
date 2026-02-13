# git-summary

## Overview
git-summary prints per-author contribution summaries (added, deleted, net, commits) for a date
range, sorted by net contribution. Date ranges use local-time boundaries.

## Usage
```text
Usage:
  git-summary <command> [args]
  git-summary <from> <to>

Commands:
  all           Entire history
  today         Today only
  yesterday     Yesterday only
  this-month    1st to today
  last-month    1st to end of last month
  this-week     This Mon-Sun
  last-week     Last Mon-Sun
  <from> <to>   Custom date range (YYYY-MM-DD)
  help          Show help
```

## Commands
- `all`: Summarize the entire Git history.
- `today`: Summarize commits from today only.
- `yesterday`: Summarize commits from yesterday only.
- `this-month`: Summarize commits from the first of the month through today.
- `last-month`: Summarize commits from the entire previous month.
- `this-week`: Summarize commits for the current Monday-Sunday window.
- `last-week`: Summarize commits for the previous Monday-Sunday window.
- `<from> <to>`: Summarize a custom date range (YYYY-MM-DD). Start must be on or before end.
- `help`: Show help output.

## Exit codes
- `0`: Success and help output.
- `1`: Validation errors, Git errors, or invalid usage.

## Dependencies
- `git` is required for all commands.

## Docs

- [Docs index](docs/README.md)
