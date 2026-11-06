## 2026-11-05 - Bind governed HTTPS transport to the configured helper account
<!-- devlog-id: 2026-11-05-issue-3313-governed-https-transport -->

### Result

- Added explicit repository-local HTTPS/SSH URL selection and status, with
  helper credential actor verification before governed HTTPS transport.

### Why / context

- Active identity policy already pinned SSH-shaped remotes to HTTPS; explicit
  HTTPS now uses the configured Git helper while retaining principal selection,
  token pinning, signing and rewrite refusal.

### Evidence

- Pre-fix focused gate: matching helper failed with
  identity_credential_missing; mismatched helper reached fake transport;
  rewrite refusal passed. Final scoped validation is recorded in the PR.

### Links

- <https://github.com/sympoies/nils-cli/issues/3313>

### Follow-ups

- Maintainer verification of a real non-main HTTPS push remains pending.
