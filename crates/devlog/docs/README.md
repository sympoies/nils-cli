# devlog crate docs

Crate-local documentation index for the `devlog` CLI.

The crate README at `crates/devlog/README.md` is the canonical command,
output-contract, and exit-code reference. It covers devlog detection, the
`new` / `search` / `check` / `index` / `completion` surfaces, the JSON
envelope names, and the structural problem kinds `check` reports.

Workspace-level contracts this crate implements:

- `docs/specs/cli-output-contract-v1.md` — envelope shape and exit codes.
- `docs/runbooks/cli-completion-development-standard.md` — completion export
  and asset freshness.
- `docs/runbooks/new-cli-crate-development-standard.md` — crate scaffold,
  publish metadata, and `jiff` for date handling.

The log conventions themselves are owned per repository by that repository's
`docs/devlog/README.md`; this crate enforces them and does not restate them.
