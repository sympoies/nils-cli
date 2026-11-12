## 2026-11-12 - agent-session: mask short flattened launch-notice echoes in last_prompt
<!-- devlog-id: 2026-11-12-short-launch-echo-mask -->

### Result

- A launch-notice echo that equals the bound notice after whitespace flattening
  (no ellipsis) is masked from last_prompt, keyed to the launch binding; an
  unbound prompt with the same text stays visible
- Each guard of the truncated-prefix predicate (ellipsis, header check, prefix
  anchoring) is pinned by a test under a binding; the last_prompt masking
  contract text in serve-observation-v1 is updated

### Why / context

- Notices under the provider echo limit are echoed whole and flattened, so they
  matched neither the exact nor the truncated-prefix mask

### Evidence

- Focused launch_row and provider_prompt tests plus local-fast pass; each
  truncated-prefix guard was mutated one at a time and failed the new guard
  test

### Links

- Refs #3603, #3595
