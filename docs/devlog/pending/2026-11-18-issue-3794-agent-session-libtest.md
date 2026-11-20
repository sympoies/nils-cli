## 2026-11-18 - Require nextest for agent-session library tests
<!-- devlog-id: 2026-11-18-issue-3794-agent-session-libtest -->

### Result

- Agent-session library tests require nextest. Plain Cargo reports the
  supported command before libtest starts. Workspace and containment checks use
  nextest.

### Why / context

- The library fixtures depend on workspace helper binaries and share
  process-wide environment and locks inside libtest.

### Evidence

- A 16-thread baseline with built helpers completed 2474 tests with three
  ignored. The plain Cargo guard and the exact nextest containment case passed
  their focused checks.

### Links

- <https://github.com/sympoies/nils-cli/issues/3794>
