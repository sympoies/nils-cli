# codex-core migration

## Goal

Migrate Codex runtime consumers to the shared `codex_core` crate while keeping CLI UX and provider
contracts stable.

## Scope

- Runtime primitives moved into `codex_core`: `auth`, `jwt`, `paths`, `config`, `exec`, typed errors.
- `codex-cli` keeps UX-only concerns: Clap command parsing, help text, rendering, compatibility redirects.
- `agent-provider-codex` maps `codex_core` runtime behavior into `provider-adapter.v1` responses.

## Adoption order

1. Add `nils-codex-core` dependency.
2. Migrate runtime imports from `codex_cli` to `codex_core`.
3. Re-run provider + cli contract tests.
4. Add/verify boundary checks to prevent re-coupling.

## before / after import examples

### Provider adapter migration

Before:

```rust
use codex_cli::{agent, auth, paths};

let auth_file = paths::resolve_auth_file();
let subject = auth::email_from_auth_file(&path)?;
let exit_code = agent::exec::exec_dangerous(prompt, caller, &mut stderr);
```

After:

```rust
use codex_core::{auth, exec, paths};

let auth_file = paths::resolve_auth_file();
let subject = auth::email_from_auth_file(&path)?;
let exit_code = exec::exec_dangerous(prompt, caller, &mut stderr);
```

### codex-cli runtime wiring migration

Before:

```rust
pub mod paths {
    // runtime logic lived directly in codex_cli
}
```

After:

```rust
pub use codex_core::paths::{
    resolve_auth_file, resolve_feature_dir, resolve_script_dir,
    resolve_secret_cache_dir, resolve_secret_dir, resolve_zdotdir,
};
```

## Compatibility expectations

- User-facing `codex-cli` output text and exit semantics remain unchanged.
- Provider category/code mapping remains unchanged (`missing-task`, `missing-binary`,
  `disabled-policy`, `execute-failed`, `invalid-auth-file`).
- No JSON contract changes for `codex-cli.auth.v1` and `codex-cli.diag.rate-limits.v1`.

## Guardrails

- `scripts/ci/codex-core-boundary-check.sh` must pass.
- `crates/agent-provider-codex/tests/dependency_boundary.rs` enforces no `codex_cli` import.

## Validation checklist

- `cargo test -p nils-codex-core --test auth_contract --test paths_config_contract --test exec_contract --test error_contract`
- `cargo test -p nils-codex-cli --test paths --test jwt --test auth_current_sync --test agent_exec --test agent_prompt`
- `cargo test -p nils-agent-provider-codex --test adapter_contract --test dependency_boundary`
- `cargo test -p nils-agentctl --test provider_registry --test provider_commands --test workflow_run`
