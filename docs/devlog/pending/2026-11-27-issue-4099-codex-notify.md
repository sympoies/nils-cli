## 2026-11-27 - Migrate the prior generated Codex notification selector
<!-- devlog-id: 2026-11-27-issue-4099-codex-notify -->

### Result

- Doctor and setup preview report migration-needed for the exact prior Codex
  notify selector. Reviewed setup installs the current selector.

### Why / context

- The selector changed in #3778. A prior generated selector was sent through
  the foreign-callback removal guard and stopped doctor with exit 65.

### Evidence

- The setup and doctor fixture reproduced exit 65 before the fix. Focused tests
  verified digest-gated migration, a no-op second apply, preservation of
  unrelated TOML, and refusal of foreign lookalikes.

### Links

- <https://github.com/sympoies/nils-cli/issues/4099>
