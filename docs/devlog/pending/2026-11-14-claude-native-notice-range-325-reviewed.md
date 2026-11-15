## 2026-11-14 - Prepare the Claude native notice range through 2.1.325
<!-- devlog-id: 2026-11-14-claude-native-notice-range-325-reviewed -->

### Result

- Prepared the bounded 2.1.325 range extension; exact-version native acceptance
  remains a PR merge gate.

### Why / context

- Bot loader success alone does not prove native prompt delivery or human-draft
  preservation. Reviewed upstream changes include validator, hooks, agents
  status and transcript persistence fixes.

### Evidence

- Bot run 43520842841 passed all three module contract tests on 2.1.325. The
  production supported-range test failed first on candidate acceptance before
  raising SUPPORTED_MAX.

### Links

- <https://github.com/sympoies/nils-cli/pull/3698>
