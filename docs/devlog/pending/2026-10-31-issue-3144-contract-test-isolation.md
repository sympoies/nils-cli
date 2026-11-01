## 2026-10-31 - Isolate the Claude module contract check from user config
<!-- devlog-id: 2026-10-31-issue-3144-contract-test-isolation -->

### Result

- The module load check now runs the installed Claude Code in a private home
  inside its own temporary directory, with a cleared environment, no inherited
  credentials and no updater.

### Why / context

- Since the check runs on any installed release at or above the tested minimum,
  the child wrote provider config and backups into the isolation guard's
  sentinel directory, so local-fast failed on development hosts with Claude
  installed while CI hosts without it stayed green.

### Evidence

- The isolation guard on the contract tests failed with Claude Code 2.1.296 and
  with 2.1.314 on PATH (sentinel claude-home/.claude.json and a backup written)
  and passes with either and with none installed; the check still runs validate
  when a supported release is installed.

### Links

- <https://github.com/sympoies/nils-cli/issues/3144>
- <https://github.com/sympoies/nils-cli/issues/3136>
