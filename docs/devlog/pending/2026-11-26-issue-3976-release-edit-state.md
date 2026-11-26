## 2026-11-26 - Add draft and publish transitions to forge-cli release edit
<!-- devlog-id: 2026-11-26-issue-3976-release-edit-state -->

### Result

- Release edits can set draft state independently or alongside notes and title.
  The result is verified against the exact release ID, tag, and resolved
  target.

### Why / context

- A release can return to draft while assets are repaired, then be published
  again with its existing metadata.

### Evidence

- Fake-provider lifecycle tests cover both directions, repeated requests,
  metadata preservation, mismatch readback, and a provider-free dry run.

### Links

- <https://github.com/sympoies/nils-cli/issues/3976>
