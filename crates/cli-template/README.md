# cli-template

## Overview
`cli-template` is a minimal binary crate used as:
- a sanity check for workspace packaging/build wiring, and
- a reference implementation for new CLIs in this repo.

It demonstrates common patterns used across the workspace:
- argument parsing with `clap`
- logging via `tracing` / `tracing-subscriber`
- clean stdout output + optional progress rendering on stderr via `nils-term`

## Commands
- `cli-template hello [name]` : prints a greeting to stdout (defaults to `world`)
- `cli-template progress-demo` : renders a short progress demo (progress on stderr, stdout stays clean)

## Common flags
- `--log-level <level>` : log level (e.g. `trace`, `debug`, `info`, `warn`, `error`)

## Development
Run locally from the workspace:
```bash
cargo run -p cli-template -- hello Nils
cargo run -p cli-template -- progress-demo
```

