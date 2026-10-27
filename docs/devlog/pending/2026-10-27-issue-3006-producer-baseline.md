## 2026-10-27 - Make release producer integration independent of PR lockfile digests
<!-- devlog-id: 2026-10-27-issue-3006-producer-baseline -->

### Result

- The canonical producer integration covers a non-release lockfile change and
  normalizes fixture artifact digests before taking its baseline.

### Why / context

- A PR can change Cargo.lock without changing the third-party inventory. The
  previous fixture repaired stale digests during the bump, which the exact-base
  detector correctly rejected as additional changes. Keep the strict detector
  and prepare a consistent fixture instead.

### Evidence

- The real producer failed classification from a disposable non-release PR
  head. A deterministic lockfile-change contract failed before fixture
  normalization and passes afterward.

### Links

- <https://github.com/sympoies/nils-cli/issues/3006>
