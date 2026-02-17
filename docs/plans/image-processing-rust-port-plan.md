# Plan: Rust image-processing parity (binary + tests)

## Overview
This plan ports the existing `image-processing` skill implementation (a Bash wrapper calling a Python CLI)
into a Rust binary crate in this workspace, with behavioral parity as the primary goal.
The Rust `image-processing` binary must be able to fully replace the current
`skills/tools/media/image-processing/scripts/` implementation while keeping:

- CLI flags/subcommands and validation rules
- exit codes (0 success, 1 runtime failure, 2 usage error)
- JSON schema (`schema_version = 1`) and report output format
- external tool invocation and fallback behavior

Source of truth (current behavior):
- `https://github.com/graysurf/agent-kit/blob/main/skills/tools/media/image-processing/scripts/image_processing.py`
- `https://github.com/graysurf/agent-kit/blob/main/skills/tools/media/image-processing/references/IMAGE_PROCESSING_GUIDE.md`

## Scope
- In scope:
  - New Rust crate + binary: `crates/image-processing` → `image-processing`
  - CLI parity for subcommands: `info`, `auto-orient`, `convert`, `resize`, `rotate`, `crop`, `pad`,
    `flip`, `flop`, `optimize`
  - Output mode gating parity: `--out`, `--out-dir`, `--in-place --yes`
  - JSON and report artifacts: `out/image-processing/runs/<run_id>/{summary.json,report.md}`
  - Comprehensive deterministic integration tests (PATH-stubbed external tools)
  - Optional: update the Codex skill wrapper to call the new binary (thin script wrapper)
- Out of scope:
  - Adding new transforms or changing UX (messages/JSON shape)
  - Implementing image transforms in pure Rust (the port continues to shell out)

## Assumptions
1. Behavioral parity is defined by the current Python implementation.
2. Tests must be deterministic and must not require ImageMagick to be installed in CI; external tools
   will be PATH-stubbed.
3. The binary name remains `image-processing` and the crate name is `image-processing`.

## External dependencies inventory
Required (hard fail, exit 1):
- ImageMagick:
  - preferred: `magick`
  - fallback: `convert` + `identify`

Optional (silent fallback to ImageMagick backend):
- WebP optimize: `cwebp` + `dwebp`
- JPEG optimize: `cjpeg` + `djpeg`

Optional (fallback behavior):
- `git` (used only to detect repo root via `git rev-parse --show-toplevel`; if missing/fails, fall back to `cwd`)

## Testing strategy
- Integration tests run the compiled binary and assert:
  - exit codes and stdout/stderr contracts
  - JSON schema stability (parse JSON; ignore `run_id` randomness)
  - output mode gating and collision/overwrite rules
  - command construction (string rendering) and backend selection
  - optional-tool selection (presence/absence on PATH)
- External binaries are stubbed via a temp `PATH` directory:
  - `magick` OR (`convert` + `identify`) for required backend
  - `cjpeg`/`djpeg`, `cwebp`/`dwebp` for optional optimize paths
  - Stubs create output files so atomic rename paths are exercised

## Rollback plan
- Revert the workspace member + crate addition and restore the skill wrapper to call the Python script.
- Keep the original Python scripts until parity tests are green and the wrapper swap is validated.

## Sprint 1: Parity docs + crate scaffold
**Goal**: Make behavior explicit and create an empty Rust crate + CLI shell.
**Validation**:
- `cargo metadata --no-deps | rg "image-processing"`
- `cargo run -p image-processing -- --help`

### Task 1.1: Write spec and fixtures
- **Location**:
  - `crates/image-processing/README.md`
- **Description**: Document CLI surface area, error contracts, JSON schema, external dependencies,
  and deterministic fixtures to drive tests.
- **Dependencies**: none
- **Complexity**: 3
- **Acceptance criteria**:
  - Spec enumerates subcommands, flags, output mode rules, and exit codes.
  - Spec documents external tool detection and optimize fallback behavior.
  - Fixtures list includes at least one scenario per subcommand and all output modes + edge cases.
- **Validation**:
  - `rg "^##" crates/image-processing/README.md`
  - `rg "schema_version" crates/image-processing/README.md`

### Task 1.2: Create crate skeleton and wire workspace
- **Location**:
  - `Cargo.toml`
  - `crates/image-processing/Cargo.toml`
  - `crates/image-processing/src/main.rs`
- **Description**: Add workspace member + a compilable binary crate.
- **Dependencies**: Task 1.1
- **Complexity**: 3
- **Acceptance criteria**:
  - `cargo run -p image-processing -- --help` exits 0.
- **Validation**:
  - `cargo run -p image-processing -- --help`

## Sprint 2: Port core logic (parity-first)
**Goal**: Implement the full CLI and transformation planning/execution logic.
**Validation**:
- `cargo run -p image-processing -- info --in <file> --json | jq .schema_version`

### Task 2.1: Implement CLI parsing and validation rules
- **Location**:
  - `crates/image-processing/src/main.rs`
- **Description**: Implement `subcommand` positional + flags matching the Python CLI, and replicate
  validation rules (forbidden flags per subcommand, output mode gating, convert/resize/crop requirements).
- **Dependencies**: Task 1.2
- **Complexity**: 6
- **Acceptance criteria**:
  - Usage errors exit 2 and include the same core error strings as the Python version.
  - `--json` stdout is JSON only.
- **Validation**:
  - `cargo run -p image-processing -- convert --in a.png --out a.webp` exits 2 (missing `--to`)

### Task 2.2: Implement toolchain detection and command execution
- **Location**:
  - `crates/image-processing/src/*`
- **Description**: Detect ImageMagick backend, optional tools, probe image info, and implement all
  subcommands by building the same external command lines and writing outputs.
- **Dependencies**: Task 2.1
- **Complexity**: 7
- **Acceptance criteria**:
  - All subcommands produce JSON summary with expected keys.
  - `--dry-run` does not write output files (but still emits planned commands).
- **Validation**:
  - `cargo run -p image-processing -- resize --in a.png --scale 2 --out out.png --dry-run --json`

## Sprint 3: Comprehensive integration tests (deterministic)
**Goal**: Full test coverage for subcommands, flags, edge cases, and dependency fallback.
**Validation**:
- `cargo test -p image-processing`

### Task 3.1: Add stub harness and core tests
- **Location**:
  - `crates/image-processing/tests/common.rs`
  - `crates/image-processing/tests/*.rs`
- **Description**: Add PATH-stubbing helpers and core tests for each subcommand and output mode.
- **Dependencies**: Sprint 2
- **Complexity**: 7
- **Acceptance criteria**:
  - Every subcommand has at least one integration test.
  - Tests assert exit codes and JSON structure.
- **Validation**:
  - `cargo test -p image-processing`

### Task 3.2: Add edge-case suite
- **Location**:
  - `crates/image-processing/tests/edge_cases.rs`
- **Description**: Add tests for:
  - missing required tools
  - missing/invalid output modes
  - output collisions
  - overwrite gating
  - alpha → jpg background requirement
  - NO_COLOR is N/A (tool is not colorized) but ensure stdout is JSON-only when `--json`
- **Dependencies**: Task 3.1
- **Complexity**: 6
- **Acceptance criteria**:
  - Edge cases are deterministic and do not depend on host ImageMagick.
- **Validation**:
  - `cargo test -p image-processing --tests`

## Sprint 4: Replace skill wrapper (optional)
**Goal**: Make Codex skill use the Rust binary instead of Python scripts.
**Validation**:
- Run the skill entrypoint and confirm parity.

### Task 4.1: Update skill entrypoint to exec the binary
- **Location**:
  - `$AGENT_HOME/skills/tools/media/image-processing/scripts/image-processing.sh`
- **Description**: Replace the Python exec with `exec image-processing "$@"` (or an explicit install path).
- **Dependencies**: Sprint 3
- **Complexity**: 2
- **Acceptance criteria**:
  - Skill runs without requiring `python3`.
- **Validation**:
  - `$AGENT_HOME/skills/tools/media/image-processing/scripts/image-processing.sh --help`
