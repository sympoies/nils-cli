## 2026-11-10 - Distinguish missing completion evidence during guarded sends
<!-- devlog-id: 2026-11-10-issue-3367-completion-refusal -->

### Result

- Guarded idle admission reports send-completion-evidence-missing with explicit
  operator recovery guidance when a stopped turn lacks provider completion
  evidence.

### Why / context

- An observed Stop with background work is not proof of idle; a missing later
  completion must remain guarded while giving callers a distinct reason to
  escalate.

### Evidence

- Diagnostic-derived event-ingress regression reproduced unknown/inferred state
  and the generic idle refusal before the fix; affected guard validation is
  retained with the PR.

### Links

- <https://github.com/sympoies/nils-cli/issues/3367>
