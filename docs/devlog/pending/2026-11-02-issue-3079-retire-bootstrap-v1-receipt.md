## 2026-11-02 - Retire the v1 bootstrap receipt upgrade
<!-- devlog-id: 2026-11-02-issue-3079-retire-bootstrap-v1-receipt -->

### Result

- forge-cli repo bootstrap now reads only schema v2 receipts; a v1 receipt is
  rejected as an unsupported schema and the in-memory v1 upgrade, its struct
  and its allowlist row are removed.

### Why / context

- The upgrade existed only for receipts written by an earlier release; the
  interrupted-create integration fixture now uses a v2 receipt of the same
  meaning, and a retained v1 receipt is refused with its path and left for its
  owner to reconcile against the remote state.

### Evidence

- The v1 rejection test failed before the change and passes after;
  repo_bootstrap unit and integration tests, fmt, clippy for the crate, the
  own-compat audit and the file-size audit pass.

### Links

- <https://github.com/sympoies/nils-cli/issues/3079>
