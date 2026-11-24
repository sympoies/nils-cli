## 2026-11-24 - Fence stopped cross-host resume sources
<!-- devlog-id: 2026-11-24-agent-session-cross-host-resume-source-fence -->

### Result

- Added read-only source planning and exact stopped history export under a
  durable writer fence. The source can release the fence after authenticating
  the destination no-writer receipt.

### Why / context

- An issued proof must remain true while the destination is deciding whether to
  admit a writer.

### Evidence

- Thirty-seven focused proof, source lifecycle, and normal resume tests passed.

### Links

- <https://github.com/sympoies/nils-cli/issues/3981>
