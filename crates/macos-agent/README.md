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
  - `macos-agent preflight [--strict] [--include-probes]`
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
- `scenario`
  - `macos-agent scenario run --file <scenario.json>`
- `profile`
  - `macos-agent profile validate --file <profile.json>`
  - `macos-agent profile init [--name <profile-name>] [--path <output.json>]`

## Global Flags

- `--format <text|json|tsv>`
- `--error-format <text|json>`
- `--dry-run`
- `--retries <n>`
- `--retry-delay-ms <ms>`
- `--timeout-ms <ms>`
- `--trace`
- `--trace-dir <path>`

Notes:
- `--format tsv` is only supported by `windows list` and `apps list`.
- `--dry-run` guarantees no OS automation command execution for mutating actions.
- `--error-format json` emits machine-parseable error payloads on `stderr`.
- `--trace` writes per-command trace artifacts to `CODEX_HOME/out/macos-agent-trace/`.
- `--trace-dir` overrides trace artifact output directory.
- When trace mode is enabled, `macos-agent` verifies trace directory writability before running actions.

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
These action results also include `result.meta.attempts_used` so flaky steps can be detected quickly.

Exit codes:
- `0`: success
- `1`: runtime failure
- `2`: usage error

Error envelope (`--error-format json`):

```json
{
  "schema_version": 1,
  "ok": false,
  "error": {
    "category": "runtime",
    "operation": "input.click",
    "message": "input.click failed via `cliclick` (exit 2): cliclick failed",
    "hints": [
      "Check macOS Accessibility/Automation permissions if this action controls System Events."
    ]
  }
}
```

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

`crates/macos-agent/tests/e2e_real_apps.rs` contains app workflow checks for:
- Finder activation + window presence + navigation hotkeys + screenshot evidence
- Arc YouTube flow (open home, click 3 videos, play/pause, comment checkpoint)
- Spotify flow (UI track click, play/pause toggles, player-state probe)
- Cross-app Arc↔Spotify focus recovery and matrix artifact index output

These checks are disabled by default and require explicit opt-in:

```bash
MACOS_AGENT_REAL_E2E=1 cargo test -p macos-agent --test e2e_real_macos
MACOS_AGENT_REAL_E2E=1 MACOS_AGENT_REAL_E2E_MUTATING=1 MACOS_AGENT_REAL_E2E_APP=Finder \
  cargo test -p macos-agent --test e2e_real_macos
MACOS_AGENT_REAL_E2E=1 MACOS_AGENT_REAL_E2E_MUTATING=1 MACOS_AGENT_REAL_E2E_APPS=finder \
  cargo test -p macos-agent --test e2e_real_apps -- finder_navigation_and_state_checks --nocapture
MACOS_AGENT_REAL_E2E=1 MACOS_AGENT_REAL_E2E_MUTATING=1 MACOS_AGENT_REAL_E2E_APPS=arc,spotify,finder \
  MACOS_AGENT_REAL_E2E_PROFILE=default-1440p \
  cargo test -p macos-agent --test e2e_real_apps -- matrix_runner_supports_app_subset_selection_real --nocapture
```

Real-app E2E environment variables:
- `MACOS_AGENT_REAL_E2E=1`: enable real desktop tests.
- `MACOS_AGENT_REAL_E2E_MUTATING=1`: allow mutating desktop actions (click/type/hotkey).
- `MACOS_AGENT_REAL_E2E_APPS=arc,spotify,finder`: select app subset in deterministic order.
  - Unsupported app names are treated as configuration errors (fail fast).
- `MACOS_AGENT_REAL_E2E_PROFILE=default-1440p`: choose coordinate profile fixture.
- `MACOS_AGENT_REAL_E2E_INPUT_SOURCE=com.apple.keylayout.ABC` (or `abc`): optional; if set, tests switch to the target input source once via `im-select` before text-entry flows.
- `MACOS_AGENT_REAL_E2E_STEP_TIMEOUT_MS=15000`: optional per-step timeout guard for real-app helper commands.
- `MACOS_AGENT_REAL_E2E_ITERATIONS=5`: optional short-loop repetition count for matrix runs.

Input-method notes for reliability:
- Arc YouTube navigation uses address-bar focus + clipboard paste + `Return` (not per-key character typing), then verifies the active URL contains `youtube.com` and is not a Google search URL.
- Spotify search input uses clipboard paste (`Cmd+A` + `Cmd+V`) and then `Return`, avoiding IME-dependent character typing.
- If you want deterministic keyboard layout, install `im-select` (`brew install im-select`) and set `MACOS_AGENT_REAL_E2E_INPUT_SOURCE=abc`.

Real-app artifact notes:
- Every real-app scenario writes `steps.jsonl` and `step-summary.json` under its artifact directory.
- `artifact-index.json` includes per-scenario `step_ledger_path`, `failing_step_id`, and `last_successful_step_id`.
- Real-app checks are manual/local validation flows and should not be included in default CI jobs.

## Immediate Feedback Loop

### Workflow 1: readiness then action probe

```bash
macos-agent --format json preflight --include-probes
macos-agent --format json window activate --app Terminal --wait-ms 1200 --retries 1
macos-agent --format json wait app-active --app Terminal --timeout-ms 1500
```

### Workflow 2: machine-parseable failure triage

```bash
macos-agent --error-format json --trace input click --x 200 --y 160
# Read latest trace in CODEX_HOME/out/macos-agent-trace/
```

### Workflow 3: iterate with scenario file + profile checks

```bash
macos-agent profile validate --file crates/macos-agent/tests/fixtures/real_e2e_profile_default_1440p.json
macos-agent --format json scenario run --file crates/macos-agent/tests/fixtures/scenario-basic.json
macos-agent profile init --name local-1440p --path "$CODEX_HOME/out/local-profile.json"
```

## Troubleshooting matrix

| Symptom | Next command | What to inspect |
| --- | --- | --- |
| `not authorized` or Apple Events failures | `macos-agent --format json preflight --include-probes` | `error.hints`, Automation/Accessibility rows |
| Flaky click/input behavior | `macos-agent --trace --error-format json input click ...` | latest trace JSON (`attempts_used`, timeout/retry policy) |
| Trace enabled but command does not start | `macos-agent --trace --trace-dir <path> --error-format json preflight` | `trace.write` error and writable-path hint |
| Real-app scenario failed mid-flow | run target `e2e_real_apps` command with `--nocapture` | `steps.jsonl`, `step-summary.json`, `artifact-index.json` |
| Profile coordinate drift | `macos-agent profile validate --file <profile.json>` | key-path validation errors and bounds issues |
