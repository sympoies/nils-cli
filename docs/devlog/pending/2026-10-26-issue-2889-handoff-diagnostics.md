## 2026-10-26 - Name each retired-coordinator takeover prerequisite
<!-- devlog-id: 2026-10-26-issue-2889-handoff-diagnostics -->

### Result

- forge-cli review-handoff takeover refusals now carry a stable
  details.prerequisite, retryable and next_action, and a message that states
  the remedy, instead of one generic policy-unavailable or evidence-unavailable
  message. All head, base, tip, supervisor and freshness guards are unchanged.
- An actor lookup where gh failed without any HTTP response is now reported and
  retried as a transient failure instead of non-retryable.

### Why / context

- Cached-base, remote, config, supervisor and lifecycle-evidence failures were
  indistinguishable and always reported as retryable session-evidence
  refreshes.

### Evidence

- New fixtures failed on the baseline for each prerequisite and pass after the
  change; the targeted forge-cli and nils-common nextest selection passed.

### Links

- <https://github.com/sympoies/nils-cli/issues/2889>
