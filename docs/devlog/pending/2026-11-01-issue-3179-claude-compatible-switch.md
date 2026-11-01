## 2026-11-01 - Restore materialized Claude switches without optional generation tracking
<!-- devlog-id: 2026-11-01-issue-3179-claude-compatible-switch -->

### Result

- Manual and queued Claude account replacement accepts a validated
  account-broker.v2 inventory without credential_state_v1, with no generation
  baseline. Advertised capability still requires verified publication; typed
  refusal details reach the serve boundary.

### Why / context

- Requiring optional publication tracking during every switch blocked
  legitimate brokers that support ordinary launches. Inventory validation
  distinguishes capability absence from invalid or unavailable broker
  responses. Same-account recovery generation rules and account-fenced resume
  controls remain authoritative.

### Evidence

- A synthetic switch failed on the baseline with capability_missing before
  production edits; 19 focused switch tests passed after the fix and
  same-profile helper build. Local-fast audits and clippy passed; its startup
  stderr fixture failed on the working head and clean baseline, leaving the
  gate incomplete.

### Links

- <https://github.com/sympoies/nils-cli/issues/3179>
