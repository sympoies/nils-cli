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
