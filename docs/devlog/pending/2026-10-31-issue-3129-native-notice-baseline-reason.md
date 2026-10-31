## 2026-10-31 - Distinct refusal for native Claude notices before a provider transcript exists
<!-- devlog-id: 2026-10-31-issue-3129-native-notice-baseline-reason -->

### Result

- A Claude notice sent to a session whose bound provider conversation has no
  resolvable transcript yet now returns notice-native-baseline-unavailable
  (guard=baseline, stage=admission) instead of the generic
  notice-native-unavailable.

### Why / context

- The native route cannot prove a new confirming row without a pre-send
  baseline, and inventing one for a file that does not exist would need a new
  ledger stamp shape and unobserved provider behavior. The refusal already
  precedes the endpoint, claim and input, so the spec now names it, says no key
  is claimed, and allows a new request after an ordinary turn with no automatic
  retry or terminal fallback.

### Evidence

- A modeled notice send with the bound transcript absent returned
  notice-native-unavailable before the change. It now returns the distinct code
  with no module text, no claimed key and no ledger record, and the same key
  delivers once after the transcript exists.

### Links

- <https://github.com/sympoies/nils-cli/issues/3129>
- <https://github.com/sympoies/nils-cli/issues/3068>
