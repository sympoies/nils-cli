## 2026-10-23 - Add opt-in strict execution checks for PR snapshots
<!-- devlog-id: 2026-10-23-issue-2643-strict-checks -->

### Result

- Moved the one-shot checks clap arguments into a dedicated module before
  adding --strict. Skipped and neutral effective-gate checks now fail only when
  strict mode is selected; per-check reporting and default provider semantics
  remain compatible.

### Why / context

- A skipped or neutral required check can satisfy a provider rule without
  proving the check ran. Callers can now opt into successful-execution gating.

### Evidence

- Failing-first fixture run: four strict tests failed before the flag existed.
  Final verification: 76 focused tests, forge-cli Clippy with warnings denied,
  completion syntax checks, and strict file-size audit passed under a 6 GiB
  memory cap with swap disabled.

### Links

- <https://github.com/sympoies/nils-cli/issues/2643>
