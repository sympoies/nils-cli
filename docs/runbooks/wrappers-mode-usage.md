# Wrapper Execution Mode Runbook

## Purpose

This runbook defines how `wrappers/*` choose between:
- a locally installed binary, and
- workspace debug execution (`cargo run -q -p ...`).

It also documents the safety rules added to prevent wrapper self-recursion and ambiguous fallback behavior.

## Scope

This applies to all current wrappers under `wrappers/`:
- `agent-docs`
- `agentctl`
- `api-gql`
- `api-rest`
- `api-test`
- `cli-template`
- `codex-cli`
- `fzf-cli`
- `git-cli`
- `git-lock`
- `git-scope`
- `git-summary`
- `image-processing`
- `macos-agent`
- `plan-tooling`
- `screen-record`
- `semantic-commit`

## Environment Contract

### `NILS_WRAPPER_MODE`

Supported values:
- `auto` (default)
- `debug`
- `installed`

Behavior summary:

| Mode | Resolution behavior |
|---|---|
| `auto` | Try installed binary first; if not found, fallback to `cargo run -q -p <package> -- ...`. |
| `debug` | Force `cargo run -q -p <package> -- ...`; fail if `cargo` is unavailable. |
| `installed` | Force installed binary lookup only; do not fallback to `cargo`. |

Invalid mode value exits with code `64` and an explicit error.

Note: debug mode uses Cargo package names (for example `nils-git-scope`, `nils-codex-cli`), not
necessarily the binary name.

### `NILS_WRAPPER_INSTALL_PREFIX`

Optional install prefix for installed binary lookup.

Default:
- `~/.local/nils-cli`

Lookup order for installed mode resolution:
1. `${NILS_WRAPPER_INSTALL_PREFIX:-$HOME/.local/nils-cli}/<bin>`
2. `command -v <bin>`

Both `~` and `~/...` forms are expanded.

## Safety Guarantees

All wrappers enforce:
- self-recursion guard: candidate binary path must not resolve to the wrapper itself (`-ef` check)
- explicit mode validation (`auto|debug|installed` only)
- deterministic fallback and error messages by mode

This prevents infinite recursion when `wrappers/` appears before real binaries in `PATH`.

## `codex-cli` Special Behavior

`codex-cli` preserves migrated command routing:
- `provider`
- `debug`
- `workflow`
- `automation`

These commands are executed via `agentctl` using the selected wrapper mode.

If routing cannot execute, wrapper prints:
- `codex-cli: use \`agentctl <cmd>\` for provider-neutral orchestration`
- exits `64`

## `git-cli` Compatibility Behavior

`git-cli` wrapper preserves the existing compatibility rule:
- when called as `git-cli -- help ...`, arguments are normalized to `git-cli help ...` before
  resolution/execution.

## Usage Examples

Set globally (for current shell):

```bash
export NILS_WRAPPER_MODE=debug
```

Force installed binary usage:

```bash
export NILS_WRAPPER_MODE=installed
export NILS_WRAPPER_INSTALL_PREFIX="$HOME/.local/nils-cli"
```

One-shot override for a single command:

```bash
NILS_WRAPPER_MODE=auto ./wrappers/git-scope tracked
NILS_WRAPPER_MODE=debug ./wrappers/codex-cli auth current
NILS_WRAPPER_MODE=installed ./wrappers/semantic-commit --help
```

`.env` example:

```dotenv
NILS_WRAPPER_MODE=debug
# NILS_WRAPPER_MODE=auto
# NILS_WRAPPER_MODE=installed
```

## Troubleshooting

### `... invalid NILS_WRAPPER_MODE=...`
- Cause: unsupported mode string
- Fix: use one of `auto`, `debug`, `installed`

### `... cargo not found (required when NILS_WRAPPER_MODE=debug)`
- Cause: debug mode without Cargo on `PATH`
- Fix: install Rust/Cargo, or switch mode to `auto`/`installed`

### `... installed binary not found (NILS_WRAPPER_MODE=installed)`
- Cause: installed mode cannot find target binary in prefix or `PATH`
- Fix:
  1. run install flow (`./.agents/skills/nils-cli-install/scripts/nils-cli-install.sh`)
  2. confirm `NILS_WRAPPER_INSTALL_PREFIX` and `PATH`

### `... binary not found (install via cargo install or build the workspace)`
- Cause: `auto` mode could not find installed binary and cannot run cargo fallback
- Fix: install binary or ensure Cargo is available

## Verification Quick Checks

```bash
NILS_WRAPPER_MODE=debug ./wrappers/cli-template --help
NILS_WRAPPER_MODE=auto ./wrappers/cli-template --help
NILS_WRAPPER_MODE=installed NILS_WRAPPER_INSTALL_PREFIX="$HOME/.local/nils-cli" ./wrappers/cli-template --help
scripts/ci/wrapper-mode-smoke.sh
```
