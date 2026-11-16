## 2026-11-16 - Add persisted state retirement shape counts
<!-- devlog-id: 2026-11-16-issue-3081-state-shape-inventory -->

### Result

- Added a read-only per-store aggregate inventory for slice-2 recovery, switch,
  title and title-fence shapes, plus local operation, heartbeat, version,
  mailbox and retitle counts.

### Why / context

- Missing stores, required collections and malformed fields cannot stand in for observed absence;
  title-pair evidence requires exact renderer comparison and provider-split
  terminals preserve current Codex behavior.

### Evidence

- Synthetic CLI suite passed 26 tests, including strict JSON rejection, exact
  title boundaries, file trust, privacy and unchanged state metadata.

### Links

- <https://github.com/sympoies/nils-cli/issues/3081>
