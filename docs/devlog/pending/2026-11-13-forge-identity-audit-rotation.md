## 2026-11-13 - Forge identity audit file rotation and unasserted guard pins
<!-- devlog-id: 2026-11-13-forge-identity-audit-rotation -->

### Result

- The identity audit file `identity-audit.jsonl` now rotates by size: the
  active file never exceeds 4 MiB and four rotated copies (`.1` newest to `.4`)
  are kept, so at most five files and 20 MiB exist. Record format and mode 0600
  are unchanged.
- Rotation and append run under an advisory lock (`identity-audit.lock`), so
  concurrent writers lose and interleave no record and the record being written
  always lands whole in the new active file.
- The cap scope for unasserted audit records is documented as accepted:
  parallel first runs of one caller can each record and the generic `deny`
  refusal record is uncapped, both now bounded by the rotation limit.

### Why / context

- The audit file had no size cap and a host already held tens of MiB (#3612,
  follow-up to #3596).
- Fleet step: a pre-existing oversized audit file is rotated whole on its first
  append and drops out after four further rotations; delete it by hand to
  reclaim space sooner.

### Evidence

- Unit tests fill the file past the limit and assert rotation, the bound, mode
  and a multi-writer run; three guard tests pin the stamp path, the automation
  marker with TTYs, and an EPIPE stderr; each guard test fails when its guard
  is removed.
- No provider call or identity probe is added: the zero-added-calls test for
  warn and deny passes unchanged.
