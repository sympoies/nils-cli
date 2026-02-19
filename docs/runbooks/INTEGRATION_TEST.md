# Integration Test Runbook

## Purpose

- Define completion-focused integration verification for contributors.
- Keep policy aligned with `docs/runbooks/cli-completion-development-standard.md`.

## Completion mode policy

- Completion mode is clap-first (`clap_complete`) and no-legacy.
- Legacy completion mode is forbidden.
- `*_COMPLETION_MODE` toggles are forbidden, including `<CLI_NAME_UPPER>_COMPLETION_MODE`.

## Completion verification commands

- Runbook reference: `docs/runbooks/cli-completion-development-standard.md`.
- Run these commands when completion/alias assets change:
  - `zsh -f tests/zsh/completion.test.zsh`
  - `zsh -n completions/zsh/_<cli>`
  - `bash -n completions/bash/<cli>`

## Release packaging expectations

- Shipped artifacts must include both completion trees:
  - `completions/zsh/`
  - `completions/bash/`
- Shipped artifacts must include both alias files:
  - `completions/zsh/aliases.zsh`
  - `completions/bash/aliases.bash`
