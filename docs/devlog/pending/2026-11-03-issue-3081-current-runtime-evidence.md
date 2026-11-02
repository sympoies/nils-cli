## 2026-11-03 - Classify current runtime evidence terminology
<!-- devlog-id: 2026-11-03-issue-3081-current-runtime-evidence -->

### Result

- Remove 14 old-contract audit rows after renaming current exit cleanup,
  external usage response, invalid-state redaction and nullable broker evidence
  terminology; behavior and assertions stay unchanged.

### Why / context

- Nullable broker runtime evidence is still produced by current provisioning,
  and current cleanup and redaction safeguards must not be treated as obsolete
  state readers. Other state retirements remain separately gated by producer
  and retained-data inventory.

### Evidence

- Inspected current provisioning and parser producers against main; verified
  the exact audit-row inventory and preserved all existing assertions. This is
  a pure rename/comment classification change, so no RED is required.

### Links

- <https://github.com/sympoies/nils-cli/issues/3081>
