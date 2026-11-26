## 2026-11-26 - Expose potentially stranded unkeyed prompts
<!-- devlog-id: 2026-11-26-stranded-unkeyed-prompts -->

### Result

- Project a content-free runtime-bound stranded-input signal for guarded
  unkeyed sends and Claude structured prompts.

### Why / context

- A paste can remain after an unconfirmed delivery without an idempotency key.

### Evidence

- Synthetic composer fixtures cover capture loss, clearing, runtime replacement
  and superseding sends.

### Links

- <https://github.com/sympoies/nils-cli/issues/4055>
