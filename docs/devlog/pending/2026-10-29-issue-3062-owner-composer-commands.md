## 2026-10-29 - Route authenticated owner commands through guarded provider TUI input
<!-- devlog-id: 2026-10-29-issue-3062-owner-composer-commands -->

### Result

- Authenticated conversation slash commands use existing guarded Codex and
  Claude TUI input, with mirrored context denylists and exact owner login
  recovery under quarantine.

### Why / context

- Provider commands must not become app-server model text. Recovery authority
  is sealed inside the authenticated conversation handler; generic input and
  account/runtime guards remain intact.

### Evidence

- Three meaningful regressions failed before production edits. Focused
  validation passes: 57 receipt tests, 12 native-send tests, four quarantine
  tests, package clippy, formatting and file-size ratchet.

### Links

- <https://github.com/sympoies/nils-cli/issues/3062>

### Follow-ups

- Scheduled native baseline and exact-head interactive acceptance remain
  separate from headless evidence; full validation remains with merge-queue CI.
