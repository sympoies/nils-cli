# Completion Coverage Matrix (Workspace Binaries)

## Policy notes

- Inventory source of truth: `python3 scripts/workspace-bins.py`.
- Obligation rule for this matrix: all workspace binaries are `required` unless explicitly `excluded` as internal/example binaries.
- Explicit exclusion: `cli-template` is `excluded` because it is an example/template CLI and is out of scope for user-facing release contract migration (`docs/plans/repo-completion-standard-rollout-plan.md`).
- Explicit treatment: `image-processing` is `required` (user-facing CLI in `README.md`) and currently lacks completion assets in both shells.
- Alias policy: alias families are required only for `git-scope` (`gs*`), `git-cli` (`gx*`), `codex-cli` (`cx*`), and `fzf-cli` (`fx*`) per `AGENTS.md`; other binaries have no alias requirement.
- Completion quality baseline for every `required` binary: subcommands + long/short flags + declared value candidates.
- Completion quality baseline for every `required` binary also requires context-aware filtering by cursor position (not global candidate dumps).
- Preferred implementation mode for every `required` binary: clap-first (`clap` + `clap_complete`) generated baseline completions, with thin shell adapters and optional dynamic value extensions only when needed.
- Legacy completion-mode toggles (`*_COMPLETION_MODE`) are not supported; adapters must stay clap-first and fail closed when generated completion cannot be loaded.
- This matrix currently tracks asset coverage and obligation decisions; deeper quality/export-command conformance is validated in rollout sprints and completion tests.

## Matrix

| Binary | Obligation | Zsh completion (`completions/zsh`) | Bash completion (`completions/bash`) | Alias requirement | Rationale |
| --- | --- | --- | --- | --- | --- |
| `agent-docs` | `required` | `present` (`_agent-docs`) | `present` (`agent-docs`) | not required | Workspace binary; completion files exist in both shell directories. |
| `agentctl` | `required` | `present` (`_agentctl`) | `present` (`agentctl`) | not required | Workspace binary; completion files exist in both shell directories. |
| `api-gql` | `required` | `present` (`_api-gql`) | `present` (`api-gql`) | not required | Workspace binary; completion files exist in both shell directories. |
| `api-grpc` | `required` | `present` (`_api-grpc`) | `present` (`api-grpc`) | not required | Workspace binary; completion files exist in both shell directories. |
| `api-rest` | `required` | `present` (`_api-rest`) | `present` (`api-rest`) | not required | Workspace binary; completion files exist in both shell directories. |
| `api-test` | `required` | `present` (`_api-test`) | `present` (`api-test`) | not required | Workspace binary; completion files exist in both shell directories. |
| `api-websocket` | `required` | `present` (`_api-websocket`) | `present` (`api-websocket`) | not required | Workspace binary; completion files exist in both shell directories. |
| `cli-template` | `excluded` | `missing` | `missing` | not required | Explicitly treated as internal/example template CLI and out of scope for user-facing release contract migration. |
| `codex-cli` | `required` | `present` (`_codex-cli`) | `present` (`codex-cli`) | required (`cx*`) and present in `aliases.zsh` + `aliases.bash` | Alias family is mandated by policy; completion files exist in both shell directories. |
| `fzf-cli` | `required` | `present` (`_fzf-cli`) | `present` (`fzf-cli`) | required (`fx*`) and present in `aliases.zsh` + `aliases.bash` | Alias family is mandated by policy; completion files exist in both shell directories. |
| `git-cli` | `required` | `present` (`_git-cli`) | `present` (`git-cli`) | required (`gx*`) and present in `aliases.zsh` + `aliases.bash` | Alias family is mandated by policy; completion files exist in both shell directories. |
| `git-lock` | `required` | `present` (`_git-lock`) | `present` (`git-lock`) | not required | Workspace binary; completion files exist in both shell directories. |
| `git-scope` | `required` | `present` (`_git-scope`) | `present` (`git-scope`) | required (`gs*`) and present in `aliases.zsh` + `aliases.bash` | Alias family is mandated by policy; completion files exist in both shell directories. |
| `git-summary` | `required` | `present` (`_git-summary`) | `present` (`git-summary`) | not required | Workspace binary; completion files exist in both shell directories. |
| `image-processing` | `required` | `missing` | `missing` | not required | Explicitly treated as a user-facing CLI (not internal/example) and therefore completion-required; current gap is both shell completion assets. |
| `macos-agent` | `required` | `present` (`_macos-agent`) | `present` (`macos-agent`) | not required | Workspace binary; completion files exist in both shell directories. |
| `memo-cli` | `required` | `present` (`_memo-cli`) | `present` (`memo-cli`) | not required | Workspace binary; completion files exist in both shell directories. |
| `plan-tooling` | `required` | `present` (`_plan-tooling`) | `present` (`plan-tooling`) | not required | Workspace binary; completion files exist in both shell directories. |
| `screen-record` | `required` | `present` (`_screen-record`) | `present` (`screen-record`) | not required | Workspace binary; completion files exist in both shell directories. |
| `semantic-commit` | `required` | `present` (`_semantic-commit`) | `present` (`semantic-commit`) | not required | Workspace binary; completion files exist in both shell directories. |

## Exclusion summary

- `excluded`: `cli-template` (explicit internal/example-template exclusion).
- No other workspace binary is excluded in this matrix.
