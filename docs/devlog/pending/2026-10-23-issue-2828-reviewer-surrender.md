## 2026-10-23 - Recover wrong-reviewer handoffs through persisted-reviewer surrender
<!-- devlog-id: 2026-10-23-issue-2828-reviewer-surrender -->

### Result

- Persisted reviewers can surrender the exact retained assignment without
  generation environment setup; replacement review writes remain
  generation-fenced.

### Why / context

- A wrong-public-author assignment could strand review because surrender reused
  the publication environment-generation check.

### Evidence

- Failing-first lifecycle and recovery diagnostics; replacement publishes and
  check passes at the unchanged head.

### Links

- <https://github.com/sympoies/nils-cli/issues/2828>
