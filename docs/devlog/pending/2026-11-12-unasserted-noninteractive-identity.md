## 2026-11-12 - Forge identity warns on unasserted non-interactive runs
<!-- devlog-id: 2026-11-12-unasserted-noninteractive-identity -->

### Result

- New identity policy key `unasserted_noninteractive = allow | warn | deny`
  (default `warn`) applies when `activation = "asserted-only"` and no principal
  is asserted in a non-interactive run (no TTY on stdin/stdout, or
  `INVOCATION_ID`, `GITHUB_ACTIONS` or `AGENT_SESSION_ID` set).
- `warn` prints one stderr line naming the fix and appends an audit record with
  caller argv0 and parent process base names only; `deny` refuses with
  `identity_unasserted_noninteractive` (exit 65).
- The stderr warning is `warning: forge identity: no principal asserted in a
  non-interactive run; set FORGE_IDENTITY_PRINCIPAL to a configured principal,
  or set the identity policy key unasserted_noninteractive = "allow" to keep the
  shared default account on purpose`; the key is `unasserted_noninteractive`.
- The warning prints once per process. Audit records are capped at one per
  caller (argv0 and parent) per hour, tracked by a stamp file under
  `forge-cli/unasserted-callers` in the state root, so scheduled automation
  writes at most 24 lines a day per caller. The decision is local (environment
  and policy file): no identity probe and no provider call is added.
- Runs that assert a principal, `identity explain`/`doctor`, and raw `gh`
  callers are unaffected.

### Why / context

- Automation that forgot a principal silently used the host's default `gh`
  account and could exhaust its shared REST quota with no signal (#3596).
- Fleet step: hosts with an `asserted-only` policy now warn for automation that
  sets no `FORGE_IDENTITY_PRINCIPAL`; set the key to `deny` once owners declare
  principals.

### Evidence

- `nils-common` and `forge-cli` integration tests cover the three modes,
  detection rules, asserted pass-through and the secret-free audit record.
