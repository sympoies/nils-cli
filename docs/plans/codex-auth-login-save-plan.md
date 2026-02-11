# Plan: codex-cli auth login and auth save

## Overview
This plan adds two auth capabilities to `codex-cli`: (1) `auth login` with three supported
methods (ChatGPT browser OAuth, ChatGPT device-code OAuth, and API key), and (2) `auth save` for
writing the active `CODEX_AUTH_FILE` into the secrets directory with an explicit target filename.
The implementation keeps existing `auth use|refresh|auto-refresh|current|sync` behavior stable,
extends JSON contracts for service consumers, and adds completion/docs parity.

## Scope
- In scope:
  - Add `codex-cli auth login` with three methods:
    - ChatGPT browser login
    - ChatGPT device-code login
    - API key login
  - Add `codex-cli auth save SECRET_JSON` to copy the active auth file into secret storage.
  - Enforce filename-required behavior for `auth save` (missing name is usage error).
  - Add overwrite confirmation for existing target files, with an explicit non-interactive bypass.
  - Update auth JSON contract + consumer runbook for new command surfaces.
  - Add/extend unit + integration tests and shell completion coverage.
- Out of scope:
  - Replacing upstream OAuth semantics or token endpoint behavior.
  - Encrypting secret files at rest.
  - Extending login to non-OpenAI providers in this change.

## Assumptions (if any)
1. Secret directory env var for this feature is `CODEX_SECRET_DIR`.
2. New export command name is `auth save` (instead of overloading `auth sync`) because behavior is
   explicit and user-directed by filename.
3. Overwrite confirmation default is "no" when user declines or input is invalid; automation can
   bypass prompt with `--yes`.
4. API-key login configures credential flow for agent commands, while token-dependent surfaces
   (notably `diag rate-limits`) keep current token requirements.

## Sprint 1: Contract and CLI surface definition
**Goal**: Finalize command grammar, mode matrix, and machine-output contract before implementation.
**Demo/Validation**:
- Command(s):
  - `cargo run -q -p nils-codex-cli -- auth --help`
  - `cargo run -q -p nils-codex-cli -- auth login --help`
  - `cargo run -q -p nils-codex-cli -- auth save --help`
- Verify:
  - Help output exposes new subcommands and flags.
  - Invalid flag combinations return usage exit code `64`.
  - JSON contract docs list new `auth login|save` command entries.

**Parallelization notes**:
- `Task 1.1` should land first (flag matrix + behavior contract).
- `Task 1.2` depends on `Task 1.1`.
- `Task 1.3` starts after `Task 1.2` so output contract matches finalized prompt/non-prompt rules.

### Task 1.1: Define `auth login` mode matrix and error semantics
- **Location**:
  - `crates/codex-cli/src/cli.rs`
  - `crates/codex-cli/src/main.rs`
  - `crates/codex-cli/README.md`
- **Description**: Define `auth login` flags and mode mapping to exactly three supported methods,
  including validation of incompatible flags (for example: `--api-key` with `--device-code`).
- **Dependencies**:
  - none
- **Complexity**: 6
- **Acceptance criteria**:
  - `auth login` supports browser, device-code, and API-key modes.
  - Illegal combinations fail fast with consistent usage text and exit `64`.
  - README command section documents the three supported methods and examples.
- **Validation**:
  - `cargo run -q -p nils-codex-cli -- auth login --help | rg -n "api-key|device|chatgpt"`
  - `cargo run -q -p nils-codex-cli -- auth login --api-key --device-code; test $? -eq 64`

### Task 1.2: Define `auth save` behavior contract (required filename + overwrite prompt)
- **Location**:
  - `crates/codex-cli/src/cli.rs`
  - `crates/codex-cli/src/main.rs`
  - `crates/codex-cli/README.md`
- **Description**: Add `auth save SECRET_JSON` CLI contract, require explicit filename, define
  path-validation rules (no traversal), and define overwrite prompt + `--yes` bypass behavior.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 5
- **Acceptance criteria**:
  - Missing filename returns usage error (`64`) with deterministic message.
  - Existing target file triggers confirmation prompt unless `--yes` is set.
  - Non-TTY mode with existing target and no `--yes` fails fast with stable error code/message
    (no blocking prompt).
  - JSON mode (`--json` / `--format json`) never prompts; overwrite without `--yes` returns
    structured error.
  - Invalid names (empty, traversal, path separators) are rejected consistently.
- **Validation**:
  - `cargo run -q -p nils-codex-cli -- auth save; test $? -eq 64`
  - `cargo run -q -p nils-codex-cli -- auth save ../bad.json; test $? -eq 64`
  - `cargo test -p nils-codex-cli --test auth_save -- --nocapture` (includes non-TTY overwrite
    confirmation gate behavior)
  - `cargo test -p nils-codex-cli --test auth_json_contract -- --nocapture` (includes JSON-mode
    overwrite structured error behavior)

### Task 1.3: Extend auth JSON contract for `login` and `save`
- **Location**:
  - `docs/specs/codex-cli-diag-auth-json-contract-v1.md`
  - `docs/runbooks/codex-cli-json-consumers.md`
  - `crates/codex-cli/src/auth/output.rs`
- **Description**: Add stable result/error envelopes for `auth login` and `auth save`, including
  non-sensitive fields only (never raw tokens/api keys). Keep additive compatibility within
  `codex-cli.auth.v1`.
- **Dependencies**:
  - Task 1.1
  - Task 1.2
- **Complexity**: 5
- **Acceptance criteria**:
  - Contract table includes `auth login` and `auth save`.
  - Runbook includes partial-failure/automation guidance for new commands.
  - Result structs exist in code and are used by command handlers.
- **Validation**:
  - `rg -n "auth login|auth save" docs/specs/codex-cli-diag-auth-json-contract-v1.md docs/runbooks/codex-cli-json-consumers.md crates/codex-cli/src/auth/output.rs`

## Sprint 2: Implement `auth login` (3 methods)
**Goal**: Ship a robust `auth login` command that supports all three requested login methods with
clear exit semantics and testable behavior.
**Demo/Validation**:
- Command(s):
  - `cargo test -p nils-codex-cli --test auth_login -- --nocapture`
  - `cargo test -p nils-codex-cli --test dispatch -- --nocapture`
- Verify:
  - Mode selection and argument mapping are deterministic.
  - Execution failures return stable error envelopes/messages.
  - Existing auth commands do not regress.

**Parallelization notes**:
- `Task 2.1` is foundational.
- `Task 2.4` starts after `Task 2.2`.
- `Task 2.3` depends on `Task 2.2` and finalizes shared output/error contract.

### Task 2.1: Add `auth::login` module and command dispatch wiring
- **Location**:
  - `crates/codex-cli/src/auth/login.rs`
  - `crates/codex-cli/src/auth/mod.rs`
  - `crates/codex-cli/src/main.rs`
  - `crates/codex-cli/src/cli.rs`
- **Description**: Implement login-mode resolution and dispatch for browser/device-code/API-key
  flows with a dedicated auth module. Keep command-level errors mapped to existing CLI conventions.
- **Dependencies**:
  - Task 1.1
  - Task 1.3
- **Complexity**: 7
- **Acceptance criteria**:
  - `auth login` (no mode flags) runs browser ChatGPT login mode by default.
  - `auth login --device-code` resolves to ChatGPT device-code flow.
  - `auth login --api-key` resolves to API-key flow.
- **Validation**:
  - `cargo test -p nils-codex-cli --test main_entrypoint -- --nocapture`
  - `cargo test -p nils-codex-cli --test auth_login -- --nocapture`

### Task 2.2: Implement per-method login adapters and invocation mapping
- **Location**:
  - `crates/codex-cli/src/auth/login.rs`
  - `crates/codex-cli/src/main.rs`
- **Description**: Add execution adapters for browser/device-code/API-key login and verify each
  mode maps to the expected invocation path.
- **Dependencies**:
  - Task 2.1
- **Complexity**: 7
- **Acceptance criteria**:
  - Browser login is default when no mode flags are provided.
  - Device-code and API-key flags route to the correct adapter.
  - Adapter-level failures are surfaced to caller with deterministic internal status mapping.
- **Validation**:
  - `cargo test -p nils-codex-cli --test auth_login -- --nocapture`

### Task 2.3: Implement shared login output/error contract and redaction guarantees
- **Location**:
  - `crates/codex-cli/src/auth/login.rs`
  - `crates/codex-cli/src/auth/output.rs`
  - `crates/codex-cli/tests/auth_json_contract.rs`
- **Description**: Translate login/process failures into stable auth JSON/text envelopes and enforce
  non-leakage for sensitive fields.
- **Dependencies**:
  - Task 1.3
  - Task 2.2
- **Complexity**: 6
- **Acceptance criteria**:
  - Success output identifies chosen login method and completion status.
  - Failure output includes stable `error.code` values for usage/process failures.
  - JSON outputs never include raw `access_token`, `refresh_token`, or API key values.
- **Validation**:
  - `cargo test -p nils-codex-cli --test auth_json_contract -- --nocapture`

### Task 2.4: Add login-focused integration tests with process stubs
- **Location**:
  - `crates/codex-cli/tests/auth_login.rs`
  - `crates/nils-test-support/src/cmd.rs`
- **Description**: Add end-to-end style tests that stub external process behavior and assert mode
  argument mapping, exit codes, and output contracts for all three login methods.
- **Dependencies**:
  - Task 2.2
- **Complexity**: 7
- **Acceptance criteria**:
  - Tests cover browser/device-code/API-key success and failure paths.
  - Tests verify invalid mode combinations return `64`.
  - Tests verify output contract parity across text and JSON modes.
- **Validation**:
  - `cargo test -p nils-codex-cli --test auth_login -- --nocapture`

## Sprint 3: Implement `auth save` with overwrite confirmation
**Goal**: Add a safe export path from active auth file to secrets directory with explicit filename
and interactive overwrite guard.
**Demo/Validation**:
- Command(s):
  - `cargo test -p nils-codex-cli --test auth_save -- --nocapture`
  - `cargo test -p nils-codex-cli --test auth_json_contract -- --nocapture`
- Verify:
  - Filename is mandatory and sanitized.
  - Overwrite confirmation behaves correctly.
  - Saved file permissions/format match existing secret handling expectations.

**Parallelization notes**:
- `Task 3.1` and `Task 3.2` can proceed in parallel after CLI shape from Sprint 1 is stable.
- `Task 3.3` depends on both implementation tasks.

### Task 3.1: Add target-path resolution and filename validation for `auth save`
- **Location**:
  - `crates/codex-cli/src/auth/save.rs`
  - `crates/codex-cli/src/auth/mod.rs`
  - `crates/codex-cli/src/paths.rs`
  - `crates/codex-cli/src/main.rs`
- **Description**: Implement save target resolution using `CODEX_SECRET_DIR`, require explicit
  filename, and reject unsafe names/traversal.
- **Dependencies**:
  - Task 1.2
  - Task 1.3
- **Complexity**: 7
- **Acceptance criteria**:
  - Missing filename fails with usage error (`64`).
  - Missing/unset/non-directory `CODEX_SECRET_DIR` fails with stable exit code and `error.code`.
  - Missing/not-found/unreadable `CODEX_AUTH_FILE` fails with stable exit code and `error.code`.
  - Save writes to resolved secret directory only.
  - Invalid filenames are rejected before any file write.
- **Validation**:
  - `cargo test -p nils-codex-cli --test auth_save -- --nocapture`

### Task 3.2: Implement overwrite prompt and `--yes` non-interactive bypass
- **Location**:
  - `crates/codex-cli/src/auth/save.rs`
  - `crates/codex-cli/src/cli.rs`
  - `crates/codex-cli/tests/auth_save.rs`
- **Description**: Add prompt flow for existing destination files (`[y/N]`), default-no behavior,
  and `--yes` for unattended workflows.
- **Dependencies**:
  - Task 1.2
- **Complexity**: 6
- **Acceptance criteria**:
  - Existing file triggers prompt in text mode when `--yes` is absent.
  - Non-affirmative input aborts safely without writing.
  - Non-TTY mode with existing target and no `--yes` returns stable overwrite-confirmation error
    without waiting for input.
  - JSON mode (`--json` / `--format json`) never prompts; overwrite without `--yes` returns
    structured error.
  - `--yes` overwrites directly and reports success.
- **Validation**:
  - `cargo test -p nils-codex-cli --test auth_save -- --nocapture`

### Task 3.3: Add JSON contract and integration coverage for `auth save`
- **Location**:
  - `crates/codex-cli/src/auth/output.rs`
  - `crates/codex-cli/tests/auth_json_contract.rs`
  - `crates/codex-cli/tests/auth_save.rs`
- **Description**: Add `auth save` JSON result/error payloads and integration tests that assert
  required fields, overwrite outcomes, and no-secret leakage.
- **Dependencies**:
  - Task 3.1
  - Task 3.2
- **Complexity**: 6
- **Acceptance criteria**:
  - JSON success payload reports target path and overwrite status.
  - JSON failure payload reports stable codes (`invalid-usage`, `overwrite-declined`,
    `overwrite-confirmation-required`, etc.).
  - No sensitive token fields appear in output payloads.
- **Validation**:
  - `cargo test -p nils-codex-cli --test auth_json_contract -- --nocapture`

## Sprint 4: Completions, docs parity, and release gate
**Goal**: Finalize UX parity with completions/docs and clear required project checks.
**Demo/Validation**:
- Command(s):
  - `zsh -f tests/zsh/completion.test.zsh`
  - `./.codex/skills/nils-cli-checks/scripts/nils-cli-checks.sh`
- Verify:
  - Completions include new auth subcommands/flags.
  - README + runbook/spec remain consistent with implementation.
  - Mandatory lint/test gates pass.

**Parallelization notes**:
- `Task 4.1` and `Task 4.2` can run in parallel.
- `Task 4.3` depends on both and on Sprint 2/3 completion.

### Task 4.1: Update zsh/bash completions for `auth login|save`
- **Location**:
  - `completions/zsh/_codex-cli`
  - `completions/bash/codex-cli`
  - `tests/zsh/completion.test.zsh`
- **Description**: Extend completion trees with new auth subcommands and mode flags
  (`--api-key`, `--device-code`, `--yes`), and add regression checks.
- **Dependencies**:
  - Task 2.1
  - Task 3.2
- **Complexity**: 4
- **Acceptance criteria**:
  - Completion offers `login` and `save` under `auth`.
  - Mode/bypass flags are suggested in the right subcommand context.
  - Zsh completion regression suite passes.
- **Validation**:
  - `rg -n "auth_cmds=\\(|login:|save:|--api-key|--device-code|--yes" completions/zsh/_codex-cli completions/bash/codex-cli tests/zsh/completion.test.zsh`
  - `zsh -f tests/zsh/completion.test.zsh`

### Task 4.2: Update docs and command examples
- **Location**:
  - `crates/codex-cli/README.md`
  - `docs/specs/codex-cli-diag-auth-json-contract-v1.md`
  - `docs/runbooks/codex-cli-json-consumers.md`
- **Description**: Add command docs and examples for three login methods and `auth save` overwrite
  behavior; ensure wording matches implemented flags and exit semantics.
- **Dependencies**:
  - Task 1.3
  - Task 2.3
  - Task 3.3
- **Complexity**: 3
- **Acceptance criteria**:
  - README auth section includes `login` and `save`.
  - JSON contract examples include both new commands.
  - Runbook migration checklist references new command routing.
- **Validation**:
  - `rg -n "auth login|auth save|device|api-key|overwrite" crates/codex-cli/README.md docs/specs/codex-cli-diag-auth-json-contract-v1.md docs/runbooks/codex-cli-json-consumers.md`

### Task 4.3: Run required checks and finalize merge readiness
- **Location**:
  - `.codex/skills/nils-cli-checks/scripts/nils-cli-checks.sh`
  - `DEVELOPMENT.md`
- **Description**: Execute mandatory project checks and confirm no regressions in auth, completions,
  docs, or existing command behavior.
- **Dependencies**:
  - Task 4.1
  - Task 4.2
- **Complexity**: 4
- **Acceptance criteria**:
  - Required lint/test suite passes.
  - New auth tests and existing auth tests pass together.
  - No completion regressions in zsh suite.
- **Validation**:
  - `./.codex/skills/nils-cli-checks/scripts/nils-cli-checks.sh`

## Testing Strategy
- Unit:
  - Add mode-resolution tests for `auth login`.
  - Add filename/path-validation and overwrite-decision tests for `auth save`.
- Integration:
  - Add binary-level tests for login method selection, process error mapping, and JSON envelopes.
  - Add binary-level tests for save path resolution (`CODEX_SECRET_DIR`), prompt behavior, and
    write success, including non-TTY and JSON-mode overwrite paths.
- E2E/manual:
  - Manual smoke: run each login mode in a real shell.
  - Manual smoke: save active auth file into secrets, verify prompt on overwrite.
  - Completion smoke via `zsh -f tests/zsh/completion.test.zsh`.

## Risks & gotchas
- API-key login and token-based rate-limit diagnostics are different auth models; docs must be
  explicit to avoid user confusion.
- Interactive overwrite prompts can break automation unless `--yes` and JSON-mode behavior are
  clearly defined.
- Upstream login command/UX changes can impact delegated login behavior and tests if stubs are too
  loose.

## Rollback plan
- Revert `auth login` and `auth save` command wiring from `cli.rs` and `main.rs`.
- Remove `auth/login.rs` and `auth/save.rs` modules plus output structs and tests tied to them.
- Revert completion/docs/spec updates for the new commands.
- Re-run `./.codex/skills/nils-cli-checks/scripts/nils-cli-checks.sh` to confirm repository returns
  to pre-change baseline.
