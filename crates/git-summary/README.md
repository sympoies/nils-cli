# git-summary

Upstream (Zsh) references:
- Script: https://github.com/graysurf/zsh-kit/blob/main/scripts/git/git-summary.zsh
- Completion: https://github.com/graysurf/zsh-kit/blob/main/scripts/_completion/_git-summary
- Docs: https://github.com/graysurf/zsh-kit/blob/main/docs/cli/git-summary.md

# git-summary parity spec

## Purpose
`git-summary` prints per-author contribution summaries (added/deleted/net/commits) over a date range,
using `git log` and local-time boundaries. Output is a fixed-width table sorted by net contribution.

## Entry points
- Command: `git-summary <command> [args]`
- Custom range: `git-summary <from> <to>` where dates are `YYYY-MM-DD`.

## Commands
- `all`: full history (no date bounds)
- `today`: today only
- `yesterday`: yesterday only
- `this-week`: current week (Mon–Sun)
- `last-week`: last full week (Mon–Sun)
- `this-month`: first day of month to today
- `last-month`: last full month
- `help`/`--help`/`-h`/empty: print help text and exit 0

## Help output (top-level)
```
Usage: git-summary <command> [args]

Commands:
  all             Entire history
  today           Today only
  yesterday       Yesterday only
  this-month      1st to today
  last-month      1st to end of last month
  this-week       This Mon–Sun
  last-week       Last Mon–Sun
  <from> <to>     Custom date range (YYYY-MM-DD)
```

## Git preconditions
- If `git` is missing: `❗ git is required but was not found in PATH.` and exit 1.
- If not a git repo: `⚠️ Not a Git repository. Run this command inside a Git project.` and exit 1.

## Date validation
- Missing date value: `❌ Missing date value.`
- Invalid format: `❌ Invalid date format: <input> (expected YYYY-MM-DD).`
- Invalid value: `❌ Invalid date value: <input>.`
- Range check: if `since > until` (lexical compare on YYYY-MM-DD) then
  `❌ Start date must be on or before end date.`

## Date boundaries and timezone
- Local timezone offset is taken from `date +%z`.
- Range bounds are built as:
  - `since`: `<YYYY-MM-DD> 00:00:00 <+ZZZZ>`
  - `until`: `<YYYY-MM-DD> 23:59:59 <+ZZZZ>`
- Git log uses `--no-merges` for all modes (including presets).

## Preset headers
The command prints a header line with a calendar emoji and then a blank line:
- all: `📅 Git summary for all commits`
- today: `📅 Git summary for today: <YYYY-MM-DD>`
- yesterday: `📅 Git summary for yesterday: <YYYY-MM-DD>`
- this-month: `📅 Git summary for this month: <start> to <today>`
- last-month: `📅 Git summary for last month: <start> to <end>`
- this-week: `📅 Git summary for this week: <start> to <end>`
- last-week: `📅 Git summary for last week: <start> to <end>`

## Author collection
- Authors are collected via `git log ... --pretty=format:"%an <%ae>"`.
- Author list is `sort | uniq`.
- Per-author logs are collected with `git log ... --author="<email>" --pretty=format:'%cd' --date=short --numstat`.

## lockfile filtering
- Log output is filtered to exclude lines matching:
  `yarn.lock | package-lock.json | pnpm-lock.yaml | .lock` (regex: `(yarn\.lock|package-lock\.json|pnpm-lock\.yaml|\.lock)$`).
- Only numstat lines (`NF==3`) contribute to add/delete counts.

## Stats computation
From the per-author log:
- `added`: sum of column 1 for `NF==3` rows (non-numeric like `-` are treated as 0).
- `deleted`: sum of column 2 for `NF==3` rows.
- Filenames containing spaces are ignored in add/delete totals because the numstat line no longer has `NF==3`.
- `commits`: count of date lines (`NF==1`).
- `first`: oldest date in range (last date line in the log output).
- `last`: newest date in range (first date line in the log output).
- `net`: `added - deleted`.

## Output table
- Header + separator:
  - Header format:
    `%-25s %-40s %8s %8s %8s %8s %12s %12s`
    with columns: Name, Email, Added, Deleted, Net, Commits, First, Last.
  - Separator line is a fixed line of hyphens matching the header width.
- Rows:
  `%-25s %-40s %8s %8s %8s %8s %12s %12s` with:
  - Name = author name
  - Email = truncated to 40 chars
  - Added/Deleted/Net/Commits/First/Last computed as above
- Rows are sorted by `Net` descending (`sort -k5 -nr`).

## Invalid usage
- If a non-preset is provided without both dates: `❌ Invalid usage. Try: git-summary help`
- If exactly one date is passed to `_git_summary`, it prints:
  `❌ Please provide both start and end dates (YYYY-MM-DD).`
# git-summary Test Fixtures

## Happy-path fixtures

### Custom range (basic)
- Setup: create repo with two authors and commits spanning `2024-01-01` to `2024-01-31`.
- Run: `git-summary 2024-01-01 2024-01-31`
- Expect:
  - Header line with column names.
  - Separator line of hyphens.
  - Rows sorted by Net desc.

### all
- Setup: any repo with commits.
- Run: `git-summary all`
- Expect:
  - `📅 Git summary for all commits` header line then blank line.
  - Table header/separator present.

### today / yesterday
- Setup: commits on today and yesterday (explicit GIT_AUTHOR_DATE/GIT_COMMITTER_DATE).
- Run: `git-summary today`, `git-summary yesterday`
- Expect:
  - Header includes exact date.
  - Table rows only include commits in that date.

### this-week / last-week
- Setup: commits on multiple days across two weeks.
- Run: `git-summary this-week`, `git-summary last-week`
- Expect:
  - Header includes `start to end` range in `YYYY-MM-DD`.
  - Rows include only commits in the computed Mon–Sun window.

### this-month / last-month
- Setup: commits across two months.
- Run: `git-summary this-month`, `git-summary last-month`
- Expect:
  - Header includes `start to end` range in `YYYY-MM-DD`.
  - Rows include only commits in the computed month window.

### Lockfile filtering
- Setup: commits that modify `yarn.lock`, `package-lock.json`, `pnpm-lock.yaml`, or `foo.lock`.
- Run: `git-summary <from> <to>` covering those commits.
- Expect:
  - Added/Deleted counts exclude those files.

## edge cases

### Invalid date format
- Run: `git-summary 2024/01/01 2024-01-31`
- Expect: `❌ Invalid date format: 2024/01/01 (expected YYYY-MM-DD).`

### Invalid date value
- Run: `git-summary 2024-02-30 2024-03-01`
- Expect: `❌ Invalid date value: 2024-02-30.`

### Start date after end date
- Run: `git-summary 2024-02-01 2024-01-31`
- Expect: `❌ Start date must be on or before end date.`

### Missing args / invalid usage
- Run: `git-summary 2024-01-01`
- Expect: `❌ Invalid usage. Try: git-summary help`

### Outside repo
- Run: `git-summary all` in a non-git directory.
- Expect: `⚠️ Not a Git repository. Run this command inside a Git project.`

### No commits in range
- Setup: range without matching commits.
- Run: `git-summary <from> <to>`
- Expect:
  - Header + separator printed.
  - No author rows.

### Binary numstat lines
- Setup: commit a binary file so numstat includes `-` values.
- Run: `git-summary <from> <to>`
- Expect:
  - Added/Deleted treat `-` as 0 (no parse crash).

### Filenames with spaces
- Setup: commit a file named `file with spaces.txt`.
- Run: `git-summary <from> <to>`
- Expect:
  - Counts remain zero for that file (numstat parsing skips space-containing paths).
