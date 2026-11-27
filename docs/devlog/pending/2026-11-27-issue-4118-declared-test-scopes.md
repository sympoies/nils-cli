## 2026-11-27 - Recognize declared test scopes for additive coverage
<!-- devlog-id: 2026-11-27-issue-4118-declared-test-scopes -->

### Result

- The test-kind evidence gate recognizes test directories declared on the
  fetched target base. Fixture wiring for additional JavaScript or TypeScript
  assertions has a review path with reproducible base and mutation runs.

### Why / context

- Repositories use dedicated test layouts outside the built-in scopes, and pure
  coverage of already-working behavior does not require product-behavior
  evidence.

### Evidence

- The pre-edit create dry-run returned `test_first_evidence_required` for pure
  coverage in `scripts/test/` and `packages/*/test/`. The integration fixtures
  check create, deliver, and adoption. Replaying the cited browser-test
  comparison admitted its insertion-only diff and still rejected the weakened
  assertion sample.

### Links

- <https://github.com/sympoies/nils-cli/issues/4118>
