## 2026-10-29 - Ratchet repository current-contract debt
<!-- devlog-id: 2026-10-29-issues-3073-3074-current-contract-audit -->

### Result

- Repository policy requires coordinated current-contract migrations, and an
  exact expiring audit replaces ineffective word-boundary guards while
  preserving tombstones.

### Why / context

- Scheduled debt must remain distinct from current defaults, immutable
  provenance and external version support; the first baseline grants no
  rollback window.

### Evidence

- Identifier/serde-alias RED captured against the original guard; focused
  audit, docs-hygiene, local-fast routing and synthetic release-preparation
  fixtures pass.

### Links

- <https://github.com/sympoies/nils-cli/issues/3074>
