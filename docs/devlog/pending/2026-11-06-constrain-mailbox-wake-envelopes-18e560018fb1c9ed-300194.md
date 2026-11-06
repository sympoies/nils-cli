## 2026-11-06 - Constrain mailbox wake envelopes
<!-- devlog-id: 2026-11-06-constrain-mailbox-wake-envelopes-18e560018fb1c9ed-300194 -->

### Result

- The hook accepts only the mailbox wake summary and its documented guidance
  form.

### Why / context

- A valid summary must not authorize arbitrary trailing instructions.

### Evidence

- Regression tests reproduce acceptance of arbitrary trailing text and repeated
  gate-off registry rewrites.

### Links

- <https://github.com/sympoies/nils-cli/issues/3349>
