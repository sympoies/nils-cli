# codex-cli

Rust port of the Zsh Codex helpers (`codex-tools`, `codex-use`, `codex-rate-limits`, `codex-starship`, etc.) for the `nils-cli` workspace.

## Install

- Build the workspace: `cargo build`
- Run help: `cargo run -p codex-cli -- --help`

For a local release install (all workspace binaries), follow `DEVELOPMENT.md`:

- `./.codex/skills/nils-cli-install/scripts/nils-cli-install.sh`

## Quickstart

Agent commands are **dangerous-mode gated** (they call `codex exec --dangerously-bypass-approvals-and-sandbox`).

```sh
export CODEX_ALLOW_DANGEROUS_ENABLED=true
codex-cli agent advice "How do I debug a flaky test?"
```

End-to-end flow (auth -> refresh -> rate-limits -> starship):

```sh
codex-cli auth use work
codex-cli auth refresh
codex-cli diag rate-limits --one-line
export CODEX_STARSHIP_ENABLED=true
codex-cli starship --refresh
```

## Command groups

### `codex-cli agent`

- `codex-cli agent prompt [PROMPT...]`
- `codex-cli agent advice [QUESTION...]`
- `codex-cli agent knowledge [CONCEPT...]`
- `codex-cli agent commit [-p|--push] [-a|--auto-stage] [EXTRA_PROMPT...]`

Notes:

- Requires `CODEX_ALLOW_DANGEROUS_ENABLED=true`.
- Requires the external `codex` CLI on `PATH` (the Rust wrapper shells out to `codex exec`).

### `codex-cli auth`

- `codex-cli auth use <profile|email>`
- `codex-cli auth refresh [secret.json]`
- `codex-cli auth auto-refresh`
- `codex-cli auth current`
- `codex-cli auth sync`

### `codex-cli diag`

- `codex-cli diag rate-limits [OPTIONS] [secret.json]`

Useful flags:

- `--one-line` (single-line output; also used by `--cached`)
- `--all` (table for all secrets)
- `--async` (concurrent all-secrets mode)
- `--cached` (no network; implies `--one-line`)

### `codex-cli config`

Because a child process cannot mutate the parent shell environment, `config set` prints a shell snippet.

- Show effective values:

  ```sh
  codex-cli config show
  ```

- Set a value in your current shell:

  ```sh
  eval "$(codex-cli config set model gpt-5.1-codex-mini)"
  eval "$(codex-cli config set reasoning medium)"
  eval "$(codex-cli config set dangerous true)"
  ```

### `codex-cli starship`

- Enable:

  ```sh
  export CODEX_STARSHIP_ENABLED=true
  ```

- Use from Starship:

  ```toml
  [custom.codex]
  command = "codex-cli starship"
  when = "true"
  ```

## Zsh wrappers and migration

The repo ships thin wrapper scripts under `wrappers/` to preserve legacy command names.

### Wrapper mapping

| Wrapper | Runs |
|---|---|
| `codex-use` | `codex-cli auth use` |
| `codex-refresh-auth` | `codex-cli auth refresh` |
| `codex-auto-refresh` | `codex-cli auth auto-refresh` |
| `codex-rate-limits` | `codex-cli diag rate-limits` |
| `codex-rate-limits-async` | `codex-cli diag rate-limits --async` |
| `codex-starship` | `codex-cli starship` |
| `cx` | `codex-cli` |
| `cxgp` | `codex-cli agent prompt` |
| `cxga` | `codex-cli agent advice` |
| `cxgk` | `codex-cli agent knowledge` |
| `cxgc` | `codex-cli agent commit` |
| `cxau` | `codex-cli auth use` |
| `cxar` | `codex-cli auth refresh` |
| `cxaa` | `codex-cli auth auto-refresh` |
| `cxac` | `codex-cli auth current` |
| `cxas` | `codex-cli auth sync` |
| `cxdr` | `codex-cli diag rate-limits` |
| `cxcs` | `codex-cli config show` |
| `cxct` | `codex-cli config set` |
| `crl` | `codex-cli diag rate-limits` |
| `crla` | `codex-cli diag rate-limits --async` |

### Zsh completion

- Completion file: `completions/zsh/_codex-cli`
- Setup:
  - Add `wrappers/` to `PATH`
  - Add `completions/zsh/` to `fpath` and run `compinit`

