## 2026-11-07 - Authenticate Claude release candidates with the bot app
<!-- devlog-id: 2026-11-07-authenticate-claude-release-candidates-with-the-bot-app-18e58fd71ab566c9-dd66 -->

### Result

- Candidate publication uses the bot app token and fails when its credentials
  are absent.

### Why / context

- App-authenticated pull requests can run CI without changing repository
  workflow permissions.

### Evidence

- Release preparation fixture tests passed.

### Links

- <https://github.com/sympoies/nils-cli/issues/3442>
