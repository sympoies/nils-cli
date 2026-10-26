## 2026-10-26 - Retire DSH session launch and resume support
<!-- devlog-id: 2026-10-26-issue-2964-retire-dsh-session-launch-resume -->

### Result

- agent-session no longer launches or resumes DSH sessions; retained DSH
  history records remain readable and receive an actionable unsupported-profile
  error.
- DSH launch-profile configuration and CLI agent selection are retired, while
  profile identifiers for supported agents are not misclassified by prefix.

### Why / context

- The DSH harness product is being retired, so its nils-cli compatibility
  declarations are retired with the product. Existing saved records should
  remain inspectable without retaining executable DSH launch behavior.

### Evidence

- Focused contracts passed: serve_config (22), cli_contract (15),
  retained-record resume refusal (1), targeted history/create guards,
  formatting, file-size and docs-prose audits.

### Links

- sympoies/nils-cli#2964

### Follow-ups

- The full local gate was outside this task's authorized local validation;
  required PR CI remains the merge gate.
