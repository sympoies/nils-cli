## 2026-11-14 - Keep failed Claude runtimes quarantined until lifecycle recovery
<!-- devlog-id: 2026-11-14-issue-3692-runtime-adoption -->

### Result

- Refuse publication-only reopening of a failed resident with
  runtime_adoption_required; retain automatic fenced same-account recovery and
  guarded restart guidance.

### Why / context

- Ordinary OAuth exchange can reject the old credential before replacement
  publication, which does not prove runtime adoption or a successful provider
  request.

### Evidence

- Credential-free lifecycle regression distinguishes publication, launch
  generation and input admission, including delayed observation and missing,
  failed or late publication.

### Links

- Refs #3692

### Follow-ups

- Live provider publication, adoption and safe-request timing remains UNRUN
  pending #2954; the initial revoked error is not prevented.
