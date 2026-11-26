## 2026-11-27 - Add remote-cli command and script execution
<!-- devlog-id: 2026-11-27-issue-3759-remote-cli -->

### Result

- Added OpenSSH-backed exec and shell commands with remote cwd resolution,
  non-interactive sudo, deadlines, typed JSON, and Bash and Zsh completions.

### Why / context

- OpenSSH joins command arguments and uses exit 255 for transport failures, so
  the CLI quotes individual arguments and identifies confirmed remote exits of
  255.

### Evidence

- Focused CLI integration tests exercise a fake OpenSSH transport, including
  remote home expansion and partial output after a deadline.

### Links

- <https://github.com/sympoies/nils-cli/issues/3759>
