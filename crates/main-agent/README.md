# main-agent

## Overview

`main-agent` is the typed, authenticated facade for durable Main Agent
orchestration runs and their interactive managed workers. A Main Agent session
creates one revision-fenced run, starts isolated worker assignments, supervises
them, accepts their results, and closes out the run. Every command authenticates
through the calling session's capability, so private objective and assignment
packets never leave that session.

The session engine, orchestration registry, worker runtime, and group lifecycle
live in [`nils-agent-session`](../agent-session/README.md). This crate owns the
facade's command surface and its controller and worker workflows.

## Package vs binary name

| Field        | Value             |
| ------------ | ----------------- |
| Package name | `nils-main-agent` |
| Binary name  | `main-agent`      |

`main-agent` must be installed next to the same-release `agent-session` binary.
Worker launch resolves `agent-session` as an exact sibling in the same release
directory and fails closed when it is missing, linked, or from another release.

## Usage

```bash
main-agent capabilities --provider codex --format json
main-agent packet-schema --format json
main-agent self readiness --format json
main-agent init --packet-file objective.json --if-absent --idempotency-key init-001 --format json
main-agent status --format json
main-agent worker start --assignment-file assignment.json --idempotency-key start-001 --format json
main-agent worker supervise ASSIGNMENT_ID --format json
main-agent worker accept ASSIGNMENT_ID --if-revision 3 --idempotency-key accept-001 --format json
main-agent closeout --if-run-revision 7 --checkpoint-file final.json --idempotency-key closeout-001 --format json
main-agent runs orphaned --format json
main-agent runs close-orphaned --older-than 7d --format json
main-agent completion zsh
```

Run `main-agent --help` for the safe lifecycle, macro-first recovery, and
revision and retry rules, and `main-agent <command> --help` for each command.

## Output and exit codes

Commands accept `--format text|json`. JSON output is a versioned envelope whose
failures carry a stable `code` and `message`; packet contents and credentials
are never included.

| Exit | Meaning                  |
| ---- | ------------------------ |
| 0    | success                  |
| 1    | runtime error            |
| 64   | command-line usage error |
| 65   | invalid or stale data    |
| 69   | temporarily unavailable  |

## Documentation

The operator runbook and the stable contracts are maintained with the
orchestration engine in `nils-agent-session`:

- [Main Agent orchestration runbook](../agent-session/docs/runbooks/main-agent-orchestration.md)
- [Main Agent orchestration v1](../agent-session/docs/specs/main-agent-orchestration-v1.md)
- [Main Agent DSH external runtime v1](../agent-session/docs/specs/main-agent-dsh-external-runtime-v1.md)

See [docs/README.md](docs/README.md) for this crate's documentation index.

## Development

The agent-session integration tests drive this binary, and this binary launches
`agent-session` workers, so validate the two packages together:

```bash
cargo build -p nils-agent-session -p nils-main-agent --bins
cargo test -p nils-main-agent
cargo test -p nils-agent-session
```

`bash scripts/ci/nils-cli-checks-entrypoint.sh --local-fast` selects both
packages whenever either one changes.

## Dependencies

- Runtime: the same-release `agent-session` binary, `tmux`, and the Codex or
  Claude Code provider CLI a worker launches. See
  [BINARY_DEPENDENCIES.md](../../BINARY_DEPENDENCIES.md).
