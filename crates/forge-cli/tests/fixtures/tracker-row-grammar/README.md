# Tracker row grammar fixtures

Conformance corpus for the program tracker phase-table grammar. The grammar is
normative in
`core/skills/issue/issue-follow-up/references/tracker-row-grammar.md`; when the
two disagree, that file wins and the corpus is the defect.

A repository that implements a parser copies this directory unchanged and runs
every pair. The path is stable. `tests/ci/test_tracker_row_grammar_fixtures.py`
keeps the corpus self-consistent in this repository.

## Layout

| Path | Body | Expectation |
| --- | --- | --- |
| `valid/<name>.md` | A tracker body with no finding | `valid/<name>.json`: the parsed rows and the generated graph |
| `invalid/<code>.md`, `invalid/<code>--<variant>.md` | A tracker body that has findings | The sibling `.json`: exactly the findings a linter reports |

Every `.md` file has one `.json` sibling with the same stem. An invalid file
name starts with a finding code it exercises.

Read each body as bytes and keep them. Some bodies depend on exact bytes:

- `valid/edge-cases.md` has two rows that each carry one U+00A0 NO-BREAK
  SPACE, and one row with a tab in its `after` list.
- `invalid/stale-graph--unclosed.md` ends on its last graph line, with no
  final line feed.

Do not reformat the corpus, and do not let an editor or formatter normalise
whitespace or add a final line feed.

## Valid expectation

```json
{
  "rows": [
    {
      "id": "A1",
      "title": "Local projection",
      "ref": {"owner": "example", "repo": "alpha", "number": 14},
      "notes": "PR example/alpha#16",
      "after": [],
      "done": true,
      "phase": "Phase 1: Foundations"
    },
    {
      "id": "REL",
      "title": "Release containing A1",
      "ref": null,
      "notes": null,
      "after": ["A1"],
      "done": false,
      "phase": "Phase 1: Foundations"
    }
  ],
  "graph": "graph LR\n  A1\n  REL{{REL}}\n  A1 --> REL"
}
```

- `rows` is in table order.
- `title` is the text as written, Markdown included.
- `ref` is `null` for a gate. `owner` and `repo` are both `null` for a `#N`
  ref, which names the tracker's own repository.
- `notes` and `phase` are `null` when absent. `after` is `[]` when absent and
  keeps the written order.
- `graph` is the generated Mermaid source without its fence: lines joined by
  one line feed, with no trailing line feed. The body's `mermaid` block holds
  the same lines.

| File | Covers |
| --- | --- |
| `valid/minimal.md` | Two rows and one dependency |
| `valid/full.md` | A complete tracker: phases, notes, gates, both ref forms, done and open rows, one issue on two rows, a title with `: `, parentheses, and backticks, ignored prose and sub-items |
| `valid/no-dependencies.md` | No `after` clause anywhere |
| `valid/no-rows.md` | A phase table with no rows, whose generated graph is the single line `graph LR` |
| `valid/edge-cases.md` | The corners of the grammar; see below |

`valid/edge-cases.md` pins, in file order:

- Section lookup: a task item in another section, and one in a section whose
  heading only begins with `## Phase table`, are not rows; both section
  headings match without regard to ASCII case; a second phase table is not
  read.
- Ignored lines: plain bullets, `*` and ordered task items, indented task
  items, a deeper heading, and a bare `###`, which leaves the phase unchanged.
- Ids are case-sensitive: `Ab` and `AB` are two rows.
- A task item inside a code fence is a row.
- Dependencies: the last ` · after` marker wins; `· afterwards` is not the
  marker; the list is trimmed; a comma may have spaces and a tab around it; a
  bullet dot is not the middle dot.
- Notes: parentheses before the ref, nested pairs, only the last group, padded
  notes, an unbalanced parenthesis, a group with no space before it, a group
  that is the whole rest (`(draft)`), and empty or blank groups.
- Refs: a trimmed title, a missing space and a doubled space after the colon,
  a bare number, the allowed name characters, a leading zero, and the fifteen
  digit limit on both sides.
- Whitespace: a phase name is trimmed, and a no-break space is never trimmed,
  neither before the ref colon nor at the end of the line.
- Graph block: a fence with another info string or another letter case does
  not open the block, and lines outside the block are not compared.

## Invalid expectation

```json
{
  "findings": [
    {"code": "unknown-dependency", "line": 6, "ids": ["A2", "A9"]}
  ]
}
```

Compare `findings` as an unordered collection; the array order carries no
meaning. `line` is the one-based line number in the `.md` file.

| `code` | `line` | `ids` |
| --- | --- | --- |
| `malformed-row` | The row line | `[]` |
| `duplicate-id` | The row that reuses the id | The reused id |
| `unknown-dependency` | The row that declares it | The row id, then the unknown id |
| `self-dependency` | The row | The row id |
| `cycle` | The first row that uses the group's first id | Every id of the group, in table order |
| `stale-graph` | `null` | `[]` |

These fields pin the corpus. The output format of a linter belongs to that
linter.

| File | Covers |
| --- | --- |
| `invalid/malformed-row.md` | One row per way a row line fails the grammar |
| `invalid/malformed-row--dependent.md` | A valid row that depends on the id a malformed row shows: that id is unknown |
| `invalid/malformed-row--lower-case-id.md` | A bold token that starts with a lower-case letter (`end`, `a1`) is not an id, and a list that names it is malformed too |
| `invalid/malformed-row--lower-case-after.md` | A lower-case token that appears only in an `after` list: the row is malformed, and no `unknown-dependency` is reported |
| `invalid/duplicate-id.md` | An id reused twice |
| `invalid/duplicate-id--with-after.md` | Duplicate rows still count for dependencies: one closes a cycle through the shared id, one names itself and a missing id |
| `invalid/unknown-dependency.md` | A missing id, and an id that differs only in case |
| `invalid/self-dependency.md` | A row that lists its own id; this is not a cycle |
| `invalid/cycle.md` | Two separate loops, and a row downstream of a loop that is not in it |
| `invalid/stale-graph--different.md` | A row was added and the block was not derived again |
| `invalid/stale-graph--edge-order.md` | The right edges in the wrong order |
| `invalid/stale-graph--gate-shape.md` | A gate drawn as a plain node |
| `invalid/stale-graph--no-block.md` | A section that holds a bullet list instead of a block |
| `invalid/stale-graph--missing-section.md` | No `## Dependency graph` section |
| `invalid/stale-graph--no-phase-table.md` | A placeholder body with no phase table and no graph; zero rows still generate `graph LR` |
| `invalid/stale-graph--unclosed.md` | The generated lines in a block that is never closed |
| `invalid/stale-graph--second-block-current.md` | A stale first block followed by a current one; the first block is the block |
| `invalid/stale-graph--indented-open.md` | The generated lines under an indented opening fence, which opens nothing |
| `invalid/stale-graph--indented-close.md` | The generated lines above an indented closing fence, which closes nothing |

The files that show a row finding have no current graph, and their
expectations carry no `stale-graph`: that code is reported only when the
table has no other finding.

## Not covered

The repository's whitespace check rejects a file with a carriage return or
with a trailing space or tab, so no fixture carries one. An implementation
tests its own removal of trailing spaces, tabs, and carriage returns.

All names are fictional. Do not add a real repository, host, or person.
