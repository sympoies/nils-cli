# Test Environment Isolation Policy

Status: active
Tracking: sympoies/nils-cli#2755

## Problem

A developer machine carries state that CI runners do not: an agent home, provider
config roots, credential files, auth-sync and remote-auth hooks, a global git
config, XDG directories. A test that spawns a child with the caller's environment
reads and writes all of it. CI is green because the runner has none of it.

Observed effects of one inherited environment: a test ran the host's auth-sync
command and copied real credentials into a fixture, a failing assertion printed
part of a credential, and agent-session tests created per-session directories and
lock files under the real agent home.

## Rules

- A test must not read or write the developer's real home, agent home, provider
  config, credential store, git config, or XDG directories, and must not run a
  command named by an inherited hook variable.
- A spawn helper starts the child from a cleared environment plus an explicit
  allowlist, and points every home and config root at a per-test temp directory.
  A test that needs another variable sets it explicitly.
- A test never prints credential material. Assertion messages compare key sets or
  digests, not file contents.

## Spawn helpers

- `CmdOptions` (and the shared `git` helpers) apply `nils_test_support::hermetic`:
  the child starts from a cleared environment plus `ENV_ALLOWLIST`, and `HOME`,
  `AGENT_HOME`, `CODEX_HOME`, `CLAUDE_CONFIG_DIR`, `XDG_*`, and `GIT_CONFIG_GLOBAL`
  point at a per-spawn temp directory. `GIT_CONFIG_NOSYSTEM` is set.
- A variable the test changed in-process (through `EnvGuard`) or passed with
  `with_env` reaches the child. Only values that equal what the process inherited
  are treated as ambient state and dropped.
- A test that builds its own `Command` calls `hermetic::isolate` and keeps the
  returned home alive until the child exits.
- Product code that reads an inherited home root under `#[cfg(test)]` can use
  `hermetic::is_ambient` to ignore an inherited value and keep a fixture's own.

## Enforcement

- `scripts/ci/test-env-isolation-guard.sh` runs representative tests under
  `env -i` with a poisoned environment. Every home and config variable points at a
  sentinel directory, and the auth-sync and remote-auth hooks are commands that
  record a hit. It fails when a sentinel changes, a hook command runs, or a run
  cannot build. Ordinary test failures are left to the regular gate. It never
  prints test output. The workspace lane of `scripts/ci/nils-cli-local-fast.sh` runs
  it after the suite, so the local gate and CI agree.
- `scripts/ci/tests/test-env-isolation-guard.test.sh` covers the guard itself
  with a fake `cargo`, because a real run cannot be made to leak on demand.
- The guard is a detector over a selection, not a proof for the whole suite. Add a
  representative test to its `SELECTION` list when a new leak class is found.
