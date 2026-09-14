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
  Markdown lint baseline rejects bold labels.

`devlog check` reports all four. `devlog new` produces entries that satisfy
them by construction.

## Commands

| Command | Purpose |
| --- | --- |
| `devlog new` | Add an entry, creating the month file and index link when absent. |
| `devlog search <term>` | Literal, case-insensitive search across month files. |
| `devlog check` | Report structural problems. |
| `devlog index` | Rewrite the README month index from the tracked month files. |
| `devlog completion <bash\|zsh>` | Export the shell completion script. |

### Devlog location

The log is detected under the repository root, in order:

1. `docs/devlog/`
2. `docs/source/devlog/`

The second covers a repository with a source/render split, where the authored
copy is not the rendered one. Pass `--dir <DIR>` to override detection.

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
when it has no bullets. The entry is inserted directly below the month heading
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
| `missing-section` | An entry lacks `Result`, `Why / context`, `Evidence`, or `Links`. |
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
| `65` | `check` found structural problems, or a file still holds an unresolved merge conflict. |
| `69` | No devlog directory, or not a git work tree. |
| `70` | Filesystem error. |

`search` distinguishes "no matches in an existing month" (`1`, with the term
echoed) from "that month has no file" (`1`, naming the missing file) so a typo
does not read as an empty log.

`new` refuses rather than repairs when a month file does not open with its
expected `# Development log - YYYY-MM` heading: the file is left untouched and
the expected heading is named. Rewriting someone's heading to make an insert
succeed would hide whichever problem produced it.

On `--format json`, `ok` mirrors the command outcome rather than execution: a
`check` that finds problems and a `search` that matches nothing both emit a
failure envelope (`structural-problems` / `no-matches`) carrying the same
payload under `error.details`, so a JSON consumer never sees `ok: true` beside
a non-zero exit.

## Dependencies

`git`, to resolve the repository root. No other external binary.
