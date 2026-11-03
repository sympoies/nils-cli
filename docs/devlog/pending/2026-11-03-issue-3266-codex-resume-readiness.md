## 2026-11-03 - Deliver mailbox wakes after Codex resume readiness
<!-- devlog-id: 2026-11-03-issue-3266-codex-resume-readiness -->

### Result

- Resumed Codex startup becomes waiting when the existing native probe observes
  its bound conversation idle; queued mailbox wakes can be delivered once.

### Why / context

- Runtime activation retains last_turn while resetting phase to starting.
  Excluding that retained history from probe candidates caused indefinite
  deferral without a new turn-end event. Shared startup settlement preserves
  Claude hook readiness and rejects changed revisions or pending ingress; no
  timeout fallback is used.

### Evidence

- The synthetic no-turn-end regression failed before the fix with
  recipient-working and notified_generation zero, then passed with the existing
  mailbox controller tests and busy, loading and pending-ingress exclusions.

### Links

- <https://github.com/sympoies/nils-cli/issues/3266>
