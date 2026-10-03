# devlog

Maintain and query a repository development log.

A devlog is a directory of `YYYY-MM.md` month files plus a `README.md` index,
holding an append-only, newest-first narrative of notable work. The conventions
live in that index; this crate is what makes them enforceable rather than
advisory.

## Why a CLI

The conventions are format-critical and previously had no owner:

- a month file whose name is not `YYYY-MM.md` is invisible to a glob-based
  search and absent from the index, and no audit notices;
- the index drifts from the tracked month files with nothing to detect it;
- entries insert newest-first under the month heading, which is positional;
- section labels must be `###` headings, because `MD036` in this workspace's
  Markdown lint baseline rejects bold labels;
- a URL written bare fails `MD034` in that same baseline, so the entry cannot
  be committed into the repository it describes.

`devlog check` reports the first four. `devlog new` produces entries that
satisfy all five by construction.

## Commands

| Command | Purpose |
| --- | --- |
| `devlog new` | Add a month entry, or an isolated fragment when enabled. |
| `devlog search <term>` | Literal, case-insensitive search across month files and pending fragments. |
| `devlog check` | Report structural problems and default-branch fragment edits or deletions. |
| `devlog fix` | Repair the structural problems that have one correct repair. |
| `devlog index` | Rewrite the README month index and pending-fragment links. |
| `devlog fold` | Move fragments dated before today into deterministic month files. |
| `devlog merge <base> <ours> <theirs>` | Union month entries for a local Git merge driver, overwriting ours. |
| `devlog completion <bash\|zsh>` | Export the shell completion script. |

### Enabling fragment layout in a repository

The default month layout keeps its existing file and output bytes. Enable the
fragment writer by exporting this setting in the repository's development
shell or automation environment:

```bash
export DEVLOG_LAYOUT=fragments
```

`DEVLOG_LAYOUT=months`, an empty value, or an unset variable selects the month
writer. Other values are rejected. There is no repository config file; apply
this environment setting to every author and the repository's CI check job.
Existing month files require no conversion.

Initialize the log directory with a `README.md` containing a `## Months`
section, then run `devlog index`. With the setting enabled, `devlog new` writes
only `pending/YYYY-MM-DD-slug.md` under the log directory. It never modifies
the month file or README, including for the first entry of a new month. The
default slug includes a unique suffix, so separate changes with the same title
remain separate entries. For a stable change identifier, pass `--slug` with
lowercase ASCII letters, digits and hyphens (up to 180 bytes):

```bash
devlog new --title "Improve log tooling" --slug improve-log-tooling \
  --result "Added isolated entries." --why "Parallel changes share no file." \
  --evidence "Git integration tests passed."
```

Choose a different slug for each change on the same date. An existing fragment
is never overwritten. The entry's HTML `devlog-id` comment carries its identity
through folding; keep that comment intact.

`search`, `check` and `index` read pending fragments alongside month entries,
even if the writer setting is unset. Fragment search results carry their real
`pending/...md` path (also in the JSON match's optional `path` field), and
`--month` includes fragments when that month's file does not yet exist.
`index` adds a separate `## Pending` link list, marked with a
`devlog-pending-index` HTML comment so an existing month-layout repository
keeps its own `Pending` prose intact when no fragments exist. Authors need not run it after
`new`, so parallel PRs can leave shared files untouched. `check` accepts that
pending list being absent or not yet refreshed.

### Fragment-only PR checks

Enable the PR ownership gate explicitly:

```bash
devlog check --base origin/main --fragments-only
```

`--fragments-only` requires `--base` and works independently of the writer
setting. It runs the existing structural checks, then rejects additions,
modifications, renames and deletions of `YYYY-MM.md` files in the selected log
directory with `month-file-changed` (exit 65). It checks committed, staged,
unstaged and untracked changes separately, so restoring the working tree cannot
hide an edit in the index or branch. Both detected directory conventions and
an explicit `--dir` inside the repository are supported. Conventional logs
check both supported paths, so switching or shadowing the detected directory
cannot hide baseline deletions. Explicit paths retain their logical location
and merge-base symlink targets. Retargeting a symlink that already existed at
the merge base, including a parent-directory symlink, reports `log-path-changed`
(exit 65); an unchanged symlink is supported. An external log cannot be compared
to this repository and is refused in this mode.

The comparison starts at the merge base of `HEAD` and the supplied ref, so
unrelated default-branch advancement does not count as a PR edit. Fragments
present at that merge base are immutable: edits report `fragment-modified`,
and deletions report `fragment-deleted`, including a deletion whose unchanged
entry is already copied into a month file by a local fold. New fragments pass.
The month-file and fragment problem kinds appear in the normal check report;
JSON failures carry them under `error.details.problems`.

CI must fetch the target default branch and enough history to resolve a common
ancestor with the checked-out PR head. For a full-history checkout targeting
`main`, for example:

```bash
git fetch origin main:refs/remotes/origin/main
DEVLOG_LAYOUT=fragments devlog check --base origin/main --fragments-only
```

For a shallow checkout, fetch or unshallow the history first; substitute the
actual target branch when it differs from `main`. An unavailable ref, unborn
`HEAD`, or missing common ancestor reports `baseline-unavailable` (exit 69).
The PR mode fails closed rather than skipping its baseline checks.

Ordinary `devlog check` retains its behavior. Fragments present on its
baseline cannot be edited or deleted unless the unchanged entry, including its
identity, is present exactly once in its month file. The ordinary baseline is
local `origin/HEAD`, then local `main` or `master`; `--base` overrides it.
An unborn repository has no baseline entries. A repository with fragments and
commits but no resolvable baseline fails with `baseline-unavailable` (exit 69).
A month-only log with the writer switch unset keeps its structural checks.
`fix` repairs month files and the index; it does not edit fragments. A
maintainer correction after folding needs a separately authorized route when
fragment-only PR enforcement is enabled.

### `devlog fold`

```bash
devlog fold
```

Fold moves entries dated strictly before today (in the system time zone) into
`YYYY-MM.md` and deletes those fragments. Today's and future entries remain
pending. Month entries sort by date descending, then slug ascending; existing
entries without an identity comment use their date and exact title as identity
and their title as the ordering tie-breaker. Prose before the first entry and
entry bodies survive. Duplicate identities or incompatible content are refused
before any file is written. Source fragments remain until all required month
and index writes succeed, so a retry after an index-write failure finishes the
same fold without duplicating already-copied identities. Files that would be rewritten must be regular
files, and malformed fragments or unresolved conflicts are refused.

The same content and cutoff date produce identical files in independent
clones. A second fold is a no-op; with no eligible fragments it writes nothing,
including the index. JSON output uses `cli.devlog.fold.v1` and reports `folded`,
`months` and `index_updated`.

Assign folding to one owner, typically a scheduled CI job on the default
branch. That job supplies `DEVLOG_LAYOUT=fragments`, fetches the current default
branch, runs `devlog fold` and ordinary `devlog check` (without
`--fragments-only`), and commits only if there is a diff. The trusted fold
owner validates structure and exact fragment transfer through ordinary check;
its month updates and fragment deletions are allowed there. Keep the PR
ownership gate on feature PRs and route the trusted fold through a separately
authorized CI path. Use the repository's protected-branch and signing workflow to publish the
result, or open a PR. If the default branch moves before delivery, refetch and
rerun the fold against the new content. Keep the CLI and check environment in
sync across development and CI.

### Optional local merge driver

Repositories that fold on branches can configure a local merge driver:

```bash
git config merge.devlog.name "Deterministic devlog entry union"
git config merge.devlog.driver 'devlog merge %O %A %B'
```

Commit a `.gitattributes` rule matching only month files, adjusting the log
location as needed:

```gitattributes
docs/devlog/????-??.md merge=devlog
```

`merge` reads the ancestor, ours and theirs, unions entries by identity, then
re-sorts them and overwrites ours. It works without log discovery or the layout
setting. One-sided corrections are retained; incompatible edits to the same
identity or to the month preamble fail (exit 65) without overwriting ours, so
Git can report a conflict for a maintainer to resolve. Shared entries appear
once, while different fragment slugs remain distinct even with the same title.
The command emits no text on success; JSON uses `cli.devlog.merge.v1`.

Git configuration is local to each clone: `.gitattributes` alone does not
install the driver. Hosted web merges and merge queues do not run custom
merge drivers. Use isolated fragments and a single fold owner to avoid those
conflicts; the driver helps local merges and rebases only.

### Devlog location

The log is detected under the repository root, in order:

1. `docs/devlog/`
2. `docs/source/devlog/`

The second covers a repository with a source/render split, where the authored
copy is not the rendered one. Pass `--dir <DIR>` to override detection.

Paths are reported relative to the repository root. A `--dir` outside that root
— checking another checkout's log — is reported as the absolute path it is, so
what is printed can be pasted back into a command that reads it.

### Entry sections

`Result`, `Why / context`, and `Evidence` are required. `Links` and
`Follow-ups` are optional, and `new` omits an optional section rather than
writing a placeholder into it.

`Links` was required in the first cut of this crate. That was inferred from a
backfill whose entries were written in one pass and all carried links; measured
against the logs that already existed across the organization, 60 of 483
hand-written entries have no `Links` section because the author had nothing
worth linking. A required section that real authors routinely and correctly
omit is a wrong requirement.

### `devlog new`

```bash
devlog new \
  --title "Made rate-limit fetching concurrent" \
  --result "Added a bounded worker pool for per-account fetches." \
  --why "Fetching was serial, so the prompt segment was as slow as the slowest account." \
  --evidence "Deterministic concurrency regressions and parity guardrails pass." \
  --link '`3d049a61`'
```

Each bullet flag repeats. `--date` defaults to today; `Follow-ups` is omitted
when it has no bullets.

A bare URL in any bullet is written as an autolink, so `--link
https://github.com/sympoies/nils-cli/pull/1729` renders as
`- <https://github.com/sympoies/nils-cli/pull/1729>`. That is what `MD034`
requires and what `rumdl fmt` would have rewritten it to.

A URL that is already an autolink, the target of an inline `[text](url)` link,
or inside a code span is left exactly as written. Punctuation that ends the
sentence rather than the URL stays outside the brackets, and a parenthesis the
URL itself opened stays inside it — `see (https://example.com/a)` becomes
`see (<https://example.com/a>)`, while
`https://en.wikipedia.org/wiki/Fixture_(disambiguation)` is wrapped whole. Each
of those matches where `rumdl fmt` draws the same line.

A bare email address is left alone, because recognizing an address is guesswork
in a way that matching a scheme is not. `MD034` covers those too, so an entry
carrying one can still fail the lint.

The entry is inserted directly below the month heading
and the index is refreshed in the same operation, because a month file nothing
links to is a file nobody finds.

`new` inserts where you ask rather than sorting: back-dating an entry is a
deliberate act, and `check` reports the resulting order instead of silently
rewriting someone's file.

### `devlog search`

```bash
devlog search forge-cli            # every month
devlog search forge-cli --month 2026-05
```

Matching is literal and case-insensitive; the terms people look up are crate
names, flags, and error codes, which regex metacharacters would mangle.
Search remains usable while a month file has an unresolved merge conflict, but
prints a note naming the file and first marker line because matches can come
from both sides. JSON mode carries the same note in the envelope's `warnings`.

### `devlog check`

Reported problem kinds:

| Kind | Meaning |
| --- | --- |
| `unexpected-file` | Not a `YYYY-MM.md` regular file; invisible to search and the index. A symlink is reported here rather than followed. |
| `missing-heading` | The month file does not open with `# Development log - YYYY-MM`. |
| `malformed-entry-heading` | An entry heading is not `## YYYY-MM-DD - <title>`. |
| `missing-section` | An entry lacks `Result`, `Why / context`, or `Evidence`. |
| `unknown-section` | An entry has a section outside the template. |
| `date-month-mismatch` | An entry's date does not belong to its month file. |
| `not-newest-first` | Entries are not ordered newest-first. |
| `missing-index` | The log has no `README.md`. |
| `index-missing-month` | A month file is not listed in the index. |
| `index-stale-month` | The index lists a month with no file. |
| `conflict-markers` | The file still holds an unresolved merge conflict. |
| `unterminated-fence` | A fenced code block is not closed; the opening line is reported. |
| `invalid-fragment` | A pending entry does not satisfy the fragment contract. |
| `fragment-modified` | A baseline fragment was edited. |
| `fragment-deleted` | A baseline fragment was removed; ordinary check permits exact folding. |
| `month-file-changed` | Fragment-only PR mode found a month-file change. |
| `log-path-changed` | Fragment-only PR mode found a change to a merge-base log symlink, including a parent-directory symlink. |

A conflict marker stops the file being parsed any further. Both sides of a
conflict are well-formed entries, so counting them would describe an
unpublishable file as a healthy one. `new` and `index` refuse such a file for
the same reason, leaving it exactly as the merge left it. `new` writes the month
file and the index, and checks both before touching either, so a refusal never
leaves a half-finished entry behind a message saying nothing was written.

Markers quoted inside a fenced code block are not a conflict — the entry that
documents conflict handling is the obvious case — and `=======` is never treated
as a marker on its own, because on its own line it is also a Markdown setext
heading underline. A real conflict always writes an opening and a closing
marker at column zero, so neither exclusion costs detection.

Nothing inside a fence is structure. An entry heading or a section heading
quoted in a code block is an example, so `check` does not count it and `fix`
does not rewrite it. This is the same exclusion the conflict scan makes, for
the same reason, and an entry documenting this format is the case both exist
for: counting a quoted `## 2026-04-17 - Title` would report a malformed heading
and three missing sections that nobody could repair without editing the prose.
A log whose entries quote headings will therefore report a lower `entry_count`
than it did before this rule, which is the count being correct rather than
changing.

An unterminated fence keeps that same conservative mask through end of file,
so a conflict marker or heading below it is still treated as quoted content.
`check` reports the fence's opening line as `unterminated-fence`, making the
malformed boundary visible without guessing where the fence should close.

### `devlog fix`

```bash
devlog fix
```

Applies every repair a structural problem has exactly one correct answer for,
then reports what is left:

| Problem | Repair |
| --- | --- |
| A section label written as `**Result**` | Rewritten as `### Result`. |
| `## 2026-04-17 — Title` | The separator becomes the `-` the parser splits on. |
| A month heading that disagrees with its filename | The heading moves; the filename is what `search`, the index and `check` are keyed on. |
| Entries out of newest-first order | Stably re-sorted. |
| A required section an entry never had | Added, carrying a bullet that says it was not recorded. |
| A month file missing from the index | Linked, the same rewrite `devlog index` performs. |

Everything else is reported and left exactly as it was, and `fix` exits 65 as
`check` would. A file that is not a month, an entry heading with no readable
date, a section outside the template, a date in the wrong month: each of those
would have to be guessed at, and a log is the wrong place to guess.

Four of the repairs above are careful about what they do not touch. Only the
five template labels are promoted, only when the bold span is the whole line
and starts at column zero, and only where the label stands in for a section —
inside an entry, in an entry that has no heading for it already. Only the
separator position in a heading is rewritten, so a dash inside a title
survives. Nothing inside a fenced code block is touched at all, because an
entry documenting this format quotes both of those forms as examples. An
unterminated fenced region is likewise left exactly as written: there is no
knowing where its content ends, so there is nowhere in it a section can be
placed. Other deterministic repairs may still be applied, but the remaining
`unterminated-fence` problem makes `fix` exit 65.

A file that mixes line endings is left alone too. Rebuilding it drops each
carriage return, so one ending has to be chosen for the whole file, and
choosing rewrites every line that used the other — a whole-file diff from a
command reporting that it repaired nothing. Which ending such a file meant is
not something `fix` can know.

Those rules are measured, not assumed. Across the 1385 entries in the
organization's logs there are 3136 standalone bold lines: 3105 are one of the
five labels and 31 are prose, and that 31 includes `**Why**`, `**Follow-up**`
and `**Why / root cause**`, each of which a looser match would have promoted
into a section its author never wrote. Exactly one heading carries a dash today,
`## 2026-06-21 - ... (v1.3.4–v1.3.7)`, and its separator is already correct —
splitting it on its first dash yields something that is not a date, which is why
the date is parsed before anything is rewritten.

A log with an unresolved merge conflict is refused before anything is written,
for the same reason `new` and `index` refuse one: repairs sitting beside a
conflict make it read as an ordinary file.

There is no `--dry-run`. `check` is the read-only question and already answers
it, and the repairs are ordinary file edits in a git repository, so `git diff`
shows exactly what changed.

`fix` does not reformat. It leaves a log that needed nothing byte-identical —
including its line endings, which are carried through rather than normalized —
and it does not introduce an `MD022` or `MD012` violation where it inserts a
section or moves an entry. Tidying the Markdown around entries it did not touch
is `rumdl fmt`'s job, not this command's.

A symlinked month file is reported as `unexpected-file` and never written
through. `check` only ever read such a link; `fix` writes, and a committed
`2026-05.md -> ../../elsewhere` would otherwise put a repair outside the log
while leaving the link itself looking untouched in review.

#### Backfilled sections

A required section that an entry never had is added with:

```markdown
- Not recorded; added by `devlog fix`.
```

It records the absence and names what added it, and stops there. Inventing a
plausible `Result` for an entry whose author never wrote one would put a false
claim into a log that exists to be trusted later — and so would an earlier
wording that asserted the entry predated the section contract, because nothing
checks that. An entry written yesterday and missing a section gets this bullet
too, and in a repository whose log is an audit record, a tool-authored claim
about *why* evidence is absent is exactly the kind of content that must not be
invented.

## Output contract

`--format text` (default) and `--format json` per
`docs/specs/cli-output-contract-v1.md`. JSON envelopes are
`cli.devlog.<command>.v1`.

Exit codes:

| Code | Meaning |
| --- | --- |
| `0` | Success; for `search`, at least one match. |
| `1` | `search` found no matches, a requested month file is absent, or `new` refused a month file whose heading is wrong. |
| `64` | Usage error, including a malformed month or an impossible date. |
| `65` | `check` found structural or disallowed PR changes, `fix` could not repair all of them, or a file still holds an unresolved merge conflict. |
| `69` | No devlog directory, not a git work tree, or an unavailable required baseline. |
| `70` | Filesystem error. |

`search` distinguishes "no matches in an existing month" (`1`, with the term
echoed) from "that month has no file" (`1`, naming the missing file) so a typo
does not read as an empty log.

`new` refuses rather than repairs when a month file does not open with its
expected `# Development log - YYYY-MM` heading: the file is left untouched and
the expected heading is named. Rewriting someone's heading to make an insert
succeed would hide whichever problem produced it.

`fix` repairs that same heading, and the two are not in conflict. `new` is
being asked to add an entry, so a surprise in the file is a reason to stop and
say so. `fix` is being asked to repair the file, so the same surprise is the
work. Anyone who hits the refusal from `new` has been told exactly which
command to reach for next.

On `--format json`, `ok` mirrors the command outcome rather than execution: a
`check` that finds problems and a `search` that matches nothing both emit a
failure envelope (`structural-problems` / `no-matches`) carrying the same
payload under `error.details`, so a JSON consumer never sees `ok: true` beside
a non-zero exit.

## Dependencies

`git`, to resolve the repository root. No other external binary.
