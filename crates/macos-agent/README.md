# macos-agent

`macos-agent` is a macOS-oriented CLI for agent desktop automation.
It provides parseable primitives for discovery, observation, and input actions:
window/app listing, window activation, click, type, hotkey, screenshot, and wait helpers.

## Quick Start

```bash
# readiness check
macos-agent preflight --format json

# list targets
macos-agent windows list --format tsv
macos-agent apps list --format json

# activate + input
macos-agent window activate --app Terminal --wait-ms 1500
macos-agent input click --x 200 --y 160
macos-agent input type --text "hello world"
macos-agent input hotkey --mods cmd,shift --key 4

# observation
macos-agent observe screenshot --active-window --path ./tmp/macos-agent.png

# stabilization waits
macos-agent wait app-active --app Terminal --timeout-ms 1500
macos-agent wait window-present --app Terminal --window-name Inbox --timeout-ms 1500
```

## Command Surface

- `preflight`
  - `macos-agent preflight [--strict]`
- `windows`
  - `macos-agent windows list [--app <name>] [--window-name <name>] [--on-screen-only]`
- `apps`
  - `macos-agent apps list`
- `window`
  - `macos-agent window activate (--window-id <id> | --active-window | --app <name> [--window-name <name>] | --bundle-id <bundle_id>) [--wait-ms <ms>]`
- `input`
  - `macos-agent input click --x <px> --y <px> [--button <left|right|middle>] [--count <n>] [--pre-wait-ms <ms>] [--post-wait-ms <ms>]`
  - `macos-agent input type --text <text> [--delay-ms <ms>] [--enter]`
  - `macos-agent input hotkey --mods <cmd,ctrl,alt,shift,fn> --key <key>`
- `observe`
  - `macos-agent observe screenshot (--window-id <id> | --active-window | --app <name> [--window-name <name>]) [--path <file>] [--image-format <png|jpg|webp>]`
- `wait`
  - `macos-agent wait sleep --ms <ms>`
  - `macos-agent wait app-active (--app <name> | --bundle-id <bundle_id>) [--timeout-ms <ms>] [--poll-ms <ms>]`
  - `macos-agent wait window-present (--window-id <id> | --active-window | --app <name> [--window-name <name>]) [--timeout-ms <ms>] [--poll-ms <ms>]`

## Global Flags

- `--format <text|json|tsv>`
- `--dry-run`
- `--retries <n>`
- `--retry-delay-ms <ms>`
- `--timeout-ms <ms>`

Notes:
- `--format tsv` is only supported by `windows list` and `apps list`.
- `--dry-run` guarantees no OS automation command execution for mutating actions.

## Output Contract

- Success:
  - Writes payload to `stdout` only.
  - `stderr` remains empty.
- Error:
  - Writes message to `stderr` only.
  - `stdout` remains empty.
  - Messages start with `error:`.

JSON envelope (`--format json`):

```json
{
  "schema_version": 1,
  "ok": true,
  "command": "input.click",
  "result": {
    "policy": {
      "dry_run": false,
      "retries": 1,
      "retry_delay_ms": 150,
      "timeout_ms": 4000
    },
    "meta": {
      "action_id": "input.click-20260101-000000-7",
      "elapsed_ms": 12
    }
  }
}
```

Mutating action commands (`window activate`, `input click`, `input type`, `input hotkey`) always
include `result.policy` in JSON output so agent-side retry and timeout policy can be parsed without
guessing defaults.

Exit codes:
- `0`: success
- `1`: runtime failure
- `2`: usage error

## Permission Matrix

| Capability | Required setup | Typical failure symptom | Mitigation |
| --- | --- | --- | --- |
| Accessibility | Terminal host allowed in **System Settings > Privacy & Security > Accessibility** | click/type/hotkey fail | Enable the shell host app (Terminal/iTerm/etc.) and retry |
| Automation (Apple Events) | Terminal host allowed in **System Settings > Privacy & Security > Automation** | activation / System Events probe fails | Allow the terminal app to control System Events |
| Screen Recording | Terminal host allowed in **System Settings > Privacy & Security > Screen Recording** | observe screenshot fails | Enable Screen Recording for terminal host |
| `cliclick` binary | Installed and on `PATH` | preflight reports missing `cliclick` | `brew install cliclick` |

## Reliability Boundaries and Practices

Desktop UI automation is inherently brittle due to animation timing, focus drift, and app responsiveness.
Use these defaults for better stability:

- Always activate context before input:
  - `window activate ... --wait-ms 1000`
- Add small waits around click chains:
  - `input click ... --pre-wait-ms 100 --post-wait-ms 100`
- Enable retries for transient failures:
  - `--retries 2 --retry-delay-ms 150`
- Keep timeouts explicit for slow apps:
  - `--timeout-ms 5000`
- Use `wait app-active` / `wait window-present` before mutating actions.

## Deterministic Test Mode

Set `CODEX_MACOS_AGENT_TEST_MODE=1` to run with deterministic fixtures and without controlling the real desktop.
This mode is used by CI-safe integration tests.

## Opt-in Real macOS E2E Checks

`crates/macos-agent/tests/e2e_real_macos.rs` contains real-desktop checks for:
- TCC signal quality in `preflight` (Accessibility/Automation statuses + hints)
- focus drift detection path for activation + `wait app-active`

These checks are disabled by default and require explicit opt-in:

```bash
MACOS_AGENT_REAL_E2E=1 cargo test -p macos-agent --test e2e_real_macos
MACOS_AGENT_REAL_E2E=1 MACOS_AGENT_REAL_E2E_MUTATING=1 MACOS_AGENT_REAL_E2E_APP=Finder \
  cargo test -p macos-agent --test e2e_real_macos
```
