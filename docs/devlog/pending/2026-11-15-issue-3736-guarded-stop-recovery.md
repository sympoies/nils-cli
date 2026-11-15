## 2026-11-15 - Recover guarded wakes with fresh structured provider idle evidence
<!-- devlog-id: 2026-11-15-issue-3736-guarded-stop-recovery -->

### Result

- Guarded Claude sends can admit a stopped turn using fresh
  runtime/generation/revision-bound structured idle without inventing a
  successful turn outcome. Preclaim refusals expose safe_to_retry; consumed
  attempts remain non-replayable.

### Why / context

- A Stop reporting background work can lack a later completion hook even after
  the provider becomes idle. Composer emptiness and quiet time alone cannot
  establish safe admission.

### Evidence

- The synthetic stopped-wake regression failed before implementation and passed
  afterward with negative evidence, draft, runtime identity and consumed-key
  cases. Retained red evidence and final scoped validation accompany the PR.

### Links

- <https://github.com/sympoies/nils-cli/issues/3736>
