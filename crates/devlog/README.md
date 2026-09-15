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
| `devlog new` | Add an entry, creating the month file and index link when absent. |
| `devlog search <term>` | Literal, case-insensitive search across month files. |
| `devlog check` | Report structural problems. |
| `devlog fix` | Repair the structural problems that have one correct repair. |
| `devlog index` | Rewrite the README month index from the tracked month files. |
| `devlog completion <bash\|zsh>` | Export the shell completion script. |

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

### `devlog check`

Reported problem kinds:

| Kind | Meaning |
| --- | --- |
| `unexpected-file` | Not a `YYYY-MM.md` month file; invisible to search and the index. |
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

Three of the repairs above are careful about what they do not touch. Only the
five template labels are promoted, and only when the bold span is the whole
line, so prose the author emphasized stays prose. Only the separator position
in a heading is rewritten, so a dash inside a title survives. And nothing
inside a fenced code block is touched at all, because an entry documenting this
format quotes both of those forms as examples.

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

`fix` does not reformat. It leaves a log that needed nothing byte-identical,
and it does not introduce a `MD022` or `MD012` violation where it inserts a
section — but tidying the Markdown around entries it did not touch is
`rumdl fmt`'s job, not this command's.

#### Backfilled sections

A required section that an entry never had is added with:

```markdown
- Not recorded separately; this entry predates the section contract.
```

That records the absence instead of describing work nobody wrote. Every entry
this lands in was written before the section contract existed, so the sentence
is true of all of them; inventing a plausible `Result` for an entry whose
author never wrote one would put a false claim into a log that exists to be
trusted later.

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
| `65` | `check` found structural problems, `fix` could not repair all of them, or a file still holds an unresolved merge conflict. |
| `69` | No devlog directory, or not a git work tree. |
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
