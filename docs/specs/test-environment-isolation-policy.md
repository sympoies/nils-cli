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

## Enforcement

- `scripts/ci/test-env-isolation-guard.sh` runs representative tests under
  `env -i` with a poisoned environment. Every home and config variable points at a
  sentinel directory, and the auth-sync and remote-auth hooks are commands that
  record a hit. It fails when a sentinel changes, a hook command runs, or a
  representative run fails. It never prints test output.
- `scripts/ci/tests/test-env-isolation-guard.test.sh` covers the guard itself
  with a fake `cargo`, because a real run cannot be made to leak on demand.
- The guard is a detector over a selection, not a proof for the whole suite. Add a
  representative test to its `SELECTION` list when a new leak class is found.
