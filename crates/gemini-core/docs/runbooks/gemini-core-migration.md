# gemini-core migration

## Goal

Migrate Gemini runtime consumers to the shared `gemini_core` crate while keeping CLI UX and
provider contracts stable.

## Scope

- Runtime primitives moved into `gemini_core`: `auth`, `jwt`, `paths`, `config`, `exec`, typed
  errors.
- `gemini-cli` keeps UX-only concerns: Clap command parsing, help text, rendering, compatibility
  redirects.
- `agent-provider-gemini` maps `gemini_core` runtime behavior into `provider-adapter.v1`
  responses.

## Adoption order

1. Add `nils-gemini-core` dependency.
2. Migrate runtime imports from `gemini_cli` to `gemini_core`.
3. Re-run provider + cli contract tests.
4. Add/verify boundary checks to prevent re-coupling.

## before / after import examples

### Provider adapter migration

Before:

```rust
use gemini_cli::{agent, auth, paths};

let auth_file = paths::resolve_auth_file();
let subject = auth::email_from_auth_file(&path)?;
let exit_code = agent::exec::exec_dangerous(prompt, caller, &mut stderr);
```

After:

```rust
use gemini_core::{auth, exec, paths};

let auth_file = paths::resolve_auth_file();
let subject = auth::email_from_auth_file(&path)?;
let exit_code = exec::exec_dangerous(prompt, caller, &mut stderr);
```

### gemini-cli runtime wiring migration

Before:

```rust
pub mod paths {
    // runtime logic lived directly in gemini_cli
}
```

After:

```rust
pub use gemini_core::paths::{
    resolve_auth_file, resolve_feature_dir, resolve_script_dir, resolve_secret_cache_dir,
    resolve_secret_dir, resolve_zdotdir,
};
```

## Compatibility expectations

- User-facing `gemini-cli` output text and exit semantics remain unchanged.
- Provider category/code mapping remains unchanged once runtime integration is enabled.
- No JSON contract changes for `gemini-cli.auth.v1` and `gemini-cli.diag.rate-limits.v1`.

## Guardrails

- `scripts/ci/gemini-core-boundary-check.sh` must pass.
- `crates/agent-provider-gemini/tests/dependency_boundary.rs` enforces no `gemini_cli` import and
  no `gemini-cli` package dependency.

## Validation checklist

- `cargo test -p nils-gemini-core --test auth_contract --test paths_config_contract --test exec_contract --test error_contract`
- `bash scripts/ci/gemini-core-boundary-check.sh`
- `cargo test -p nils-agent-provider-gemini --test adapter_contract --test dependency_boundary`
