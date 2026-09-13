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

- One file per month: `docs/devlog/YYYY-MM.md`, newest entry first.
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

- Optional.
```

## Months

- [2026-09](2026-09.md)
- [2026-08](2026-08.md)
- [2026-07](2026-07.md)
- [2026-06](2026-06.md)
- [2026-05](2026-05.md)
- [2026-04](2026-04.md)
- [2026-03](2026-03.md)
- [2026-02](2026-02.md)
- [2026-01](2026-01.md)
