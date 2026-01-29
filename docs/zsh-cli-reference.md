# Zsh CLI Reference

This reference summarizes shared patterns across the Zsh helper CLI documentation without duplicating
per-command details. Use it as a quick orientation, then consult the source docs for exact flags
and examples.

## Source

- Primary reference directory: `~/.config/zsh/docs/cli` (per-tool markdown guides).
- This file is a synthesis of those guides; it does not replace them.

## Conventions

- Opt-in features: some helpers only load when enabled via `ZSH_FEATURES` in `~/.zshenv` before
  sourcing the config. Defaults are conservative and features stay disabled unless explicitly turned on.
- Dispatcher pattern: grouped helpers expose a top-level entrypoint with subcommands and `help` output;
  short aliases are often provided for common flows.
- Environment-driven configuration: most behaviors are controlled by environment variables, with
  CLI flags overriding env defaults for one-off runs.
- Safe defaults and guardrails: destructive actions prompt for confirmation or require explicit flags;
  some tools are designed to no-op (exit 0) when dependencies are missing to avoid breaking shells.
- Interactive UX and previews: several helpers use fuzzy search or multi-step pickers with previews;
  preview tooling has sensible fallbacks when optional binaries are unavailable.
- Caching and background refresh: status or completion data may be cached under `$ZSH_CACHE_DIR` with
  TTLs and background refresh to keep interactive latency low.
- Cross-platform behavior: browser-opening helpers use platform-appropriate open commands, and date
  handling aims to be compatible on macOS and Linux.

## Completion Notes

- Completion functions are typically loaded during `compinit` for opt-in feature groups.
- Some completions are dynamic (e.g., remote PR/commit lists) and use short-lived caches to avoid
  repeated network calls while iterating with `<TAB>`.
- fzf-tab enhances interactive completion flows when installed, but completions should still work
  without it.
- If completions are missing, verify the relevant feature flag is enabled, ensure wrappers are on
  `PATH`, and re-run `compinit` or start a fresh shell.
