# secrets docs

`secrets` owns pull/add/set of dotenv entries in a SOPS store. `pull` writes
plaintext to a mode-600 output file. `set` reads one value exclusively from
non-terminal stdin, updates an assignment in memory, validates ciphertext and
installs it atomically in a governed non-default managed store worktree.
It explicitly hands off signed commit/PR delivery as pending.

See the crate [README](../README.md) for stdin framing, dotenv escaping,
store selection, transaction boundaries, delivery recovery, output modes,
exit codes, and the no-secret-leak guarantee. The shared JSON envelope contract
lives in `docs/specs/cli-service-json-contract-guideline-v1.md`.
