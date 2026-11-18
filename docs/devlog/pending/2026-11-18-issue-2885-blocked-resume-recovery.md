## 2026-11-18 - Suppress daemon-blocked runtime exit recovery
<!-- devlog-id: 2026-11-18-issue-2885-blocked-resume-recovery -->

### Result

- Daemon session lists remove the runtime-exit Resume action when
  launch-profile checks block resume, retaining the bounded exit reason.

### Why / context

- Lifecycle recovery is projected before request-local daemon readiness checks;
  both resume affordances must agree after enrichment.

### Evidence

- The extended profile-removal route regression reproduced the stale recovery
  action before production edits; the strict file-size audit passes.

### Links

- <https://github.com/sympoies/nils-cli/issues/2885>
