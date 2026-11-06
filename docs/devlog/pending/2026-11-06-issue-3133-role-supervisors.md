## 2026-11-06 - Authorize attested review-handoff supervisor roles
<!-- devlog-id: 2026-11-06-issue-3133-role-supervisors -->

### Result

- Allow repository-base role selectors to match operator-attested broker
  provenance and retain supervisor role and binding digest with coordinator
  retirement evidence.

### Why / context

- Stable role configuration survives supervisor replacement while ordinary
  session labels and inherited initiators grant no takeover authority.

### Evidence

- Focused role and digest takeover tests cover actor exclusions, retirement
  proof, persisted attestation and append-time reauthentication.

### Links

- <https://github.com/sympoies/nils-cli/issues/3133>
