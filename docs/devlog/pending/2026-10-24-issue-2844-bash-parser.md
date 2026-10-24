## 2026-10-24 - Restore local-fast parsing on Bash 3.2
<!-- devlog-id: 2026-10-24-issue-2844-bash-parser -->

### Result

- Remove unnecessary command-substitution quotes around the embedded planner
  assignment so Bash 3.2 can parse the local-fast script.

### Why / context

- Bash 3.2 misparses quotes inside the heredoc when the surrounding command
  substitution is double-quoted; assignment values already suppress word
  splitting.

### Evidence

- Syntax-only parsing reproduced exit 2 before the fix and passes with Bash 3.2
  and Bash 5.3 after it. Existing routing and mocked orchestration tests pass
  with Bash 5.3.

### Links

- <https://github.com/sympoies/nils-cli/issues/2844>
