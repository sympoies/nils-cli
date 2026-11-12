## 2026-11-05 - Diagnose native Claude notice pre-claim refusals
<!-- devlog-id: 2026-11-05-issue-3330-notice-refusal-diagnostics -->

### Result

- Native notice pre-claim errors retain the top-level code and add fixed
  reasons for baseline, listener, authorization, readiness, claim and
  private-file failures.

### Why / context

- A single unavailable error hid which check refused delivery. Safe labels make
  the failing stage observable without exposing request contents or changing
  fallback permission.

### Evidence

- Guarded-send diagnostic assertions reproduced missing reasons before
  production edits; private-filesystem diagnostic tests also failed on absent
  details before their implementation changed.

### Links

- <https://github.com/sympoies/nils-cli/issues/3330>
