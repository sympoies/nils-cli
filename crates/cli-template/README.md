# cli-template

## Overview
cli-template is a minimal reference CLI for the nils-cli workspace. It demonstrates clap
argument parsing, tracing-based logging, and optional progress output via nils-term.

## Usage
```text
Usage:
  cli-template [--log-level <level>] [command]

Commands:
  hello [name]
  progress-demo

Help:
  cli-template --help
```

## Commands
- `hello [name]`: Print a greeting (defaults to `world`).
- `progress-demo`: Render a short progress demo (progress on stderr, stdout stays clean).

## Flags
- `--log-level <level>`: Log level (`trace|debug|info|warn|error`).

## Docs

- [Docs index](docs/README.md)
