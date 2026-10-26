# Development log

A time-ordered narrative of notable work on the `nils-cli` workspace: what
changed, why it mattered, the evidence, and the links worth keeping for future
debugging. It complements, rather than duplicates, the repository's other
records:

- Commit messages say what changed. The devlog preserves the non-obvious
  context, validation results, and external references that a diff cannot.
- `README.md`, `DEVELOPMENT.md`, `AGENTS.md`, and the runbooks and specs under
  `docs/` describe today's contract. The devlog is an append-only historical
  narrative; update the canonical owner first when behavior or guidance
  changes.
- Pull requests, `docs/plans/`, and `docs/discussions/` retain detailed
  delivery evidence. The devlog summarizes the milestones that stay useful
  after those records close.

## When to add an entry

Add one after non-trivial development work produces a durable outcome worth a
future lookup: a shipped capability, a contract or schema decision, a
compatibility or security decision, a validation milestone, an
incident-relevant finding, or an external reference. Skip trivial, transient,
and same-turn fixes with no future debugging or decision value.

## Conventions

- Authors add one isolated fragment per durable outcome under
  `docs/devlog/pending/YYYY-MM-DD-slug.md` with `devlog new`.
- Source `scripts/dev-env.sh` first (`DEVLOG_LAYOUT=fragments`); use
  `devlog >=1.31.16`. An explicit `--slug` must be unique for that date.
- Keep the generated `devlog-id` comment. Merged fragments are immutable;
  write a new entry for a correction.
- Monthly files remain the historical view, newest entry first. Ordinary
  PRs do not edit them or regenerate the shared index.
- Write in English, like the rest of the repository.
- Keep current docs current. The devlog records history; it does not own the
  current CLI contract, policy, setup, or runbook.
- This is a public repository. Never record secrets, credentials, private
  conversations, provider payloads, personal identifiers, internal hostnames,
  private topology, or machine-local paths. Reference identifiers, never
  values.
- Search past entries with `devlog search <term> [--month YYYY-MM]`, and check
  structural integrity with `devlog check`.
- When an entry is committed separately, use
  `docs(devlog): <YYYY-MM> - <subject>`.

### Writing and checking an entry

```bash
source scripts/dev-env.sh
devlog new --title "<durable outcome>" --slug "issue-123-example" \
  --result "<what changed>" --why "<non-obvious context>" \
  --evidence "<validation actually run>" --link "<issue or PR URL>"
devlog check --base origin/main --fragments-only
```

Fetch `origin/main` before checking. The comparison uses the merge base, so
unrelated changes landing on main do not count as edits in your PR. The local
and CI gates reject month-file edits and changes to merged fragments, including
in documentation-only PRs. Search includes pending entries without an index
refresh. Stage only your fragment; `new` leaves the month and index untouched.

### Folding accepted fragments

After each release, the maintainer prepares one dedicated maintenance PR
from current `origin/main` in a managed worktree:

```bash
git-cli worktree add devlog-fold-YYYY-MM-DD --kind chore --from origin/main
# In the new worktree:
bash scripts/ci/fold-devlog.sh
```

The script requires a clean `chore/devlog-fold-<date>` branch, sets the writer
environment, runs `devlog fold` and ordinary `devlog check --base HEAD`, and
leaves the diff for review. It does not commit, push, tag or publish. Commit
with `semantic-commit` using the exact subject
`chore(devlog): fold accepted fragments`, then run the normal pre-PR gate and
deliver with `forge-cli`. Skip the PR when there is no diff. Only this
maintenance path folds entries; ordinary feature PRs continue adding fragments.

Folding uses UTC on both contributor machines and CI. Entries dated before
the preparation day in UTC are folded; today's and future entries stay pending. Repeating a completed fold does not duplicate entries. The gate
requires the canonical branch and commit subject (also on the queue's squash
commit), replays the retired baseline fragments with `devlog fold`, restores
retained fragments and regenerates the index, then compares the complete log
byte for byte. It admits only exact transfers and generated index changes,
including when review crosses midnight. Arbitrary history edits and added or
modified fragments in a fold are rejected. If a concurrent fold changes the
base, prepare again from the new base before review.

The dedicated PR keeps release preparation's existing version-only patch
contract intact and uses normal protected-branch checks and review. Nobody
pushes a fold directly to main, and no new scheduled writer or protection
exception is needed. Pending entries remain searchable between releases.
Structural or factual history repairs require a separately reviewed maintenance
change to this policy; `devlog fix` is not an ordinary authoring step.

### Entry template

```md
## YYYY-MM-DD - <short title>

### Result

- What shipped or changed.

### Why / context

- The non-obvious reasoning or compatibility context.

### Evidence

- Commands run and concrete observations.

### Links

- Commits, issues, pull requests, external references, and relevant docs.

### Follow-ups

- Anything left open.
```

`Result`, `Why / context`, and `Evidence` are required. `Links` and `Follow-ups`
are optional: omit the whole section rather than leaving a placeholder in it.

## Months

- [2026-10](2026-10.md)
- [2026-09](2026-09.md)
- [2026-08](2026-08.md)
- [2026-07](2026-07.md)
- [2026-06](2026-06.md)
- [2026-05](2026-05.md)
- [2026-04](2026-04.md)
- [2026-03](2026-03.md)
- [2026-02](2026-02.md)
- [2026-01](2026-01.md)
