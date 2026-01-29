# Plan: Rust CLI workspace bootstrap for nils-cli

## Overview
This plan sets up a Rust development environment on macOS and scaffolds a Cargo workspace designed for multiple independently packaged CLI binaries. It also selects a baseline set of Rust crates for CLI development, references existing zsh CLI docs for conventions, and decides whether zle completion/wrapper assets live in this repo. It does not inventory or implement any specific CLI binaries from the existing zsh wrappers. The outcome is a ready-to-build repo plus a verified local toolchain and a documented completion strategy.

## Scope
- In scope: Rust toolchain installation on macOS, baseline developer tooling (rustfmt/clippy), repo scaffolding for a multi-binary workspace, baseline crate selection for CLI development, referencing `~/.config/zsh/docs/cli` for naming/convention guidance, a decision on whether zle completion/wrapper assets live in this repo, and validation that the workspace builds.
- Out of scope: Implementing any specific CLI functionality, porting zsh scripts, or enumerating which binaries to build.

## Assumptions (if any)
1. The machine has internet access and permissions to install developer tools.
2. The user prefers rustup-managed toolchains on macOS.
3. The repo will be a Cargo workspace with one binary crate per CLI and an optional shared library crate.
4. Existing zsh CLI docs at `~/.config/zsh/docs/cli` are available to reference for naming and UX conventions.
5. A clear decision will be made about whether zle completion/wrapper assets live in this repo.

## Sprint 1: Local Rust toolchain (macOS)
**Goal**: Install and verify the Rust toolchain and baseline dev tools.
**Demo/Validation**:
- Command(s): `rustc --version`, `cargo --version`, `rustup --version`, `rustfmt --version`, `cargo clippy -V`
- Verify: All commands return versions without error and the default toolchain is stable.

### Task 1.1: Install rustup and stable toolchain
- **Location**:
  - `~/.rustup`
  - `~/.cargo`
  - `~/.zshrc`
- **Description**: Install rustup using the official installer, set the default toolchain to stable, and ensure `~/.cargo/bin` is on PATH for zsh.
- **Dependencies**:
  - none
- **Complexity**: 3
- **Acceptance criteria**:
  - `rustup show` reports the stable toolchain as default.
  - `rustc --version` and `cargo --version` run successfully in a new zsh session.
  - `rustup which rustc` points to the rustup-managed toolchain.
- **Validation**:
  - `rustup show`
  - `zsh -lc "rustc --version"`
  - `zsh -lc "cargo --version"`
  - `rustup which rustc`

### Task 1.2: Add baseline Rust components
- **Location**:
  - `~/.rustup`
  - `~/.cargo/bin`
- **Description**: Install rustfmt and clippy components for linting and formatting.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 2
- **Acceptance criteria**:
  - `rustfmt` and `cargo clippy` are available.
- **Validation**:
  - `rustfmt --version`
  - `cargo clippy -V`

### Task 1.3: Install optional developer helpers
- **Location**:
  - `~/.cargo/bin`
- **Description**: Install `cargo-watch` for rapid local iteration during CLI development.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 2
- **Acceptance criteria**:
  - `cargo watch` runs and shows its help output.
- **Validation**:
  - `cargo watch --help`

### Task 1.4: Review existing zsh CLI docs for conventions
- **Location**:
  - `docs/zsh-cli-reference.md`
- **Description**: Review `~/.config/zsh/docs/cli` to capture naming conventions, common CLI patterns, and completion expectations that should influence the Rust repo scaffold.
- **Dependencies**:
  - none
- **Complexity**: 3
- **Acceptance criteria**:
  - `docs/zsh-cli-reference.md` includes sections for Source, Conventions, and Completion Notes.
  - The source section explicitly references `~/.config/zsh/docs/cli`.
  - Summary avoids enumerating specific binaries.
- **Validation**:
  - `rg "^## Source" docs/zsh-cli-reference.md`
  - `rg "^## Conventions" docs/zsh-cli-reference.md`
  - `rg "^## Completion Notes" docs/zsh-cli-reference.md`
  - `rg "~/.config/zsh/docs/cli" docs/zsh-cli-reference.md`

## Sprint 2: Repo scaffold + baseline CLI crates
**Goal**: Create a Cargo workspace that supports multiple independently packaged CLI binaries with shared dependencies.
**Demo/Validation**:
- Command(s): `cargo metadata --no-deps`, `cargo build -p cli-template`, `cargo run -p cli-template -- --help`
- Verify: Workspace metadata resolves and the template binary runs.

### Task 2.1: Create workspace root files
- **Location**:
  - `Cargo.toml`
  - `rust-toolchain.toml`
  - `.gitignore`
  - `README.md`
- **Description**: Scaffold the repo root with a Cargo workspace manifest, a toolchain file selecting the stable channel, and minimal documentation.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 4
- **Acceptance criteria**:
  - `cargo metadata --no-deps` succeeds.
  - `rust-toolchain.toml` specifies the stable channel.
- **Validation**:
  - `cargo metadata --no-deps`
  - `rg "channel = \\"stable\\"" rust-toolchain.toml`

### Task 2.2: Add workspace members for shared library and template CLI
- **Location**:
  - `crates/nils-common/Cargo.toml`
  - `crates/nils-common/src/lib.rs`
  - `crates/cli-template/Cargo.toml`
  - `crates/cli-template/src/main.rs`
- **Description**: Create a shared library crate (`nils-common`) and a minimal template binary crate (`cli-template`) to validate packaging for future CLIs.
- **Dependencies**:
  - Task 2.1
- **Complexity**: 4
- **Acceptance criteria**:
  - `cargo metadata --no-deps` lists both packages.
  - `cargo build -p cli-template` produces a binary.
- **Validation**:
  - `cargo metadata --no-deps | rg "cli-template"`
  - `cargo metadata --no-deps | rg "nils-common"`
  - `cargo build -p cli-template`

### Task 2.3: Define baseline CLI dependencies in the workspace
- **Location**:
  - `Cargo.toml`
- **Description**: Add baseline crates for CLI development via `[workspace.dependencies]`. Recommended set: `clap`, `clap_complete`, `anyhow`, `thiserror`, `serde`, `serde_json`, `toml`, `directories`, `tracing`, `tracing-subscriber`.
- **Dependencies**:
  - Task 2.2
- **Complexity**: 5
- **Acceptance criteria**:
  - Workspace dependencies are defined for the recommended set.
- **Validation**:
  - `rg "\[workspace.dependencies\]" Cargo.toml`
  - `rg "clap" Cargo.toml`
  - `rg "tracing" Cargo.toml`

### Task 2.4: Wire baseline dependencies into cli-template
- **Location**:
  - `crates/cli-template/Cargo.toml`
  - `crates/cli-template/src/main.rs`
- **Description**: Use the baseline dependencies in `cli-template` so `--help` works and logging initializes without errors.
- **Dependencies**:
  - Task 2.3
- **Complexity**: 4
- **Acceptance criteria**:
  - `cargo run -p cli-template -- --help` executes successfully.
- **Validation**:
  - `cargo run -p cli-template -- --help`

### Task 2.5: Document per-binary packaging guidance
- **Location**:
  - `README.md`
- **Description**: Document how each CLI crate is packaged as its own binary and how to add new CLI crates to the workspace.
- **Dependencies**:
  - Task 2.2
- **Complexity**: 2
- **Acceptance criteria**:
  - README explains `cargo build -p cli-template` and `cargo run -p cli-template -- ...` usage.
- **Validation**:
  - `rg "cargo run -p" README.md`

### Task 2.6: Add in-repo zsh completion/wrapper layout
- **Location**:
  - `README.md`
  - `docs/completions-strategy.md`
  - `completions/zsh/.gitkeep`
  - `wrappers/.gitkeep`
- **Description**: Decide whether zle completion and zsh wrapper assets should live in this repo. If yes, specify the directory layout (e.g. `completions/zsh/` and `wrappers/`). If no, document the external repo path and integration steps.
- **Dependencies**:
  - Task 1.4
- **Complexity**: 4
- **Acceptance criteria**:
  - `docs/completions-strategy.md` includes Decision, Rationale, Location, and Integration Steps sections.
  - Decision explicitly states in-repo placement with `completions/zsh/` and `wrappers/`.
  - `completions/zsh/` and `wrappers/` directories exist.
  - `README.md` documents optional installation steps for zsh wrappers and completion.
- **Validation**:
  - `rg "^## Decision" docs/completions-strategy.md`
  - `rg "^## Rationale" docs/completions-strategy.md`
  - `rg "^## Location" docs/completions-strategy.md`
  - `rg "^## Integration Steps" docs/completions-strategy.md`
  - `rg "completions/zsh" docs/completions-strategy.md`
  - `rg "wrappers/" docs/completions-strategy.md`
  - `test -f completions/zsh/.gitkeep`
  - `test -f wrappers/.gitkeep`
  - `rg "zsh completion" README.md`

### Task 2.7: Validate workspace build and test workflow
- **Location**:
  - `Cargo.toml`
  - `crates/cli-template/src/main.rs`
  - `crates/nils-common/src/lib.rs`
- **Description**: Ensure the workspace builds cleanly and the shared crate has at least one unit test to verify the test harness.
- **Dependencies**:
  - Task 2.1
  - Task 2.2
  - Task 2.3
  - Task 2.4
- **Complexity**: 3
- **Acceptance criteria**:
  - `cargo build` succeeds for the workspace.
  - `cargo test -p nils-common` passes.
- **Validation**:
  - `cargo build`
  - `cargo test -p nils-common`

## Testing Strategy
- Unit: Add at least one simple unit test in `nils-common` to validate test setup.
- Integration: `cargo build` and `cargo test` at the workspace root.
- E2E/manual: `cargo run -p cli-template -- --help` verifies CLI wiring.

## Risks & gotchas
- macOS PATH setup can fail if zsh does not source the updated config; verify in a new shell.
- Using Homebrew-installed Rust alongside rustup can cause toolchain conflicts.
- Workspace dependency versions should be pinned to avoid accidental breaking changes.

## Rollback plan
- Remove rustup toolchains with `rustup self uninstall` if needed.
- Revert repo scaffolding by deleting workspace files and `crates/` directory.
