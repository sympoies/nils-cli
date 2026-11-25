## 2026-11-25 - Isolate review-loop dry-run fixture from GitHub CLI
<!-- devlog-id: 2026-11-25-forge-review-loop-dry-run-fixture -->

### Result

- Stub the review-loop schema fixture and assert that inspect makes no provider
  call while observe attempts only its read-only PR lookup.

### Why / context

- A detached GitHub CLI telemetry process can recreate the fixture state
  directory after teardown. The production observe preflight remains unchanged.

### Evidence

- Traced device-id recreation after fixture cleanup with telemetry sampling
  forced. The focused regression failed before gh was stubbed and passed with
  the stub.

### Links

- <https://github.com/sympoies/nils-cli/issues/4032>
