## 2026-11-12 - Forge identity warns on unasserted non-interactive runs
<!-- devlog-id: 2026-11-12-unasserted-noninteractive-identity -->

### Result

- New identity policy key `unasserted_noninteractive = allow | warn | deny` (default `warn`) applies when `activation = "asserted-only"` and no principal is asserted in a non-interactive run (no TTY on stdin/stdout, or `INVOCATION_ID`, `GITHUB_ACTIONS` or `AGENT_SESSION_ID` set).
- `warn` prints one stderr line naming the fix and appends an audit record with caller argv0 and parent process base names only; `deny` refuses with `identity_unasserted_noninteractive` (exit 65).
- Runs that assert a principal, `identity explain`/`doctor`, and raw `gh` callers are unaffected.

### Why / context

- Automation that forgot a principal silently used the host's default `gh` account and could exhaust its shared REST quota with no signal (#3596).
- Fleet step: hosts with an `asserted-only` policy now warn for automation that sets no `FORGE_IDENTITY_PRINCIPAL`; set the key to `deny` once owners declare principals.

### Evidence

- `nils-common` and `forge-cli` integration tests cover the three modes, detection rules, asserted pass-through and the secret-free audit record.
