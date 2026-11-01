## 2026-11-01 - Restore materialized Claude switches without optional generation tracking
<!-- devlog-id: 2026-11-01-issue-3179-claude-compatible-switch -->

### Result

- Manual and queued Claude account replacement accepts a validated
  account-broker.v2 inventory without credential_state_v1, with no generation
  baseline. Advertised capability still requires verified publication; typed
  refusal details reach the serve boundary.
- Post-materialization account entries reuse the canonical broker nickname,
  uniqueness and public-metadata validation before replacement is authorized.

### Why / context

- Requiring optional publication tracking during every switch blocked
  legitimate brokers that support ordinary launches. Inventory validation
  distinguishes capability absence from invalid or unavailable broker
  responses. Same-account recovery generation rules and account-fenced resume
  controls remain authoritative.
- A valid admission list does not validate a later response changed by
  materialization. Review found this gap; extracting the existing validator
  keeps ordinary lists and switch preflight on the same contract.

### Evidence

- Meaningful capability-absent switch RED preceded the initial fix. A second
  RED on the reviewed head returned HTTP 200 for a duplicate post-materialization
  list instead of HTTP 422; the shared-validation repair preceded its GREEN.
- Focused switch, canonical broker and publication tests passed with existing
  assertions retained. The original local startup failure, clean-base control
  and actual same-source alternate-host gate PASS are retained with the PR;
  the environment-specific difference remains tracked separately.

### Links

- <https://github.com/sympoies/nils-cli/issues/3179>
- <https://github.com/sympoies/nils-cli/issues/3182>
- <https://github.com/sympoies/nils-cli/pull/3183>
