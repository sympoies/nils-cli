# Governed HTTPS transport

Use an explicit HTTPS remote when the repository needs the configured Git
credential helper for governed fetch/push. This procedure changes only the
repository-local remote URL; linked worktrees share that choice. It does not
change forge accounts, credential storage, signing, or delivery authorization.

## Select and verify HTTPS

1. Configure an authorized system/global Git credential helper for the forge.
   For GitHub, the maintainer can use `gh auth setup-git` after authenticating the intended
   account. Repository-local and worktree-local helper entries, including
   URL-scoped helpers and entries reached through local includes, refuse before
   execution. The helper's credential must authenticate as the selected identity
   profile's expected account. A working API credential alone does not prove
   the helper uses that account.
2. Remove URL rewrite rules through their configuration owner. Both
   `insteadOf` and `pushInsteadOf` remain forbidden. Resolve multiple remote
   URLs or an explicit push URL before using the switch.
3. Select HTTPS and inspect the stored choice:

   ```bash
   git-cli transport set https
   git-cli transport status --format json
   ```

   For another named remote, pass `--remote <name>` to both commands. The
   selection is recorded in `remote.<name>.url` in the repository's common
   local Git configuration. Status is local readback, not an authentication
   probe.
4. Check that the intended identity policy is active:

   ```bash
   forge-cli identity explain --operation git-read --remote origin --format json
   ```

   Require `enforced: true` and the intended selection before treating a
   governed fetch as proof of principal binding. An absent policy, or
   `activation = "asserted-only"` without an asserted principal, leaves Git
   transport in passthrough mode. The URL switch does not activate policy;
   resolve activation or principal configuration through its owner.
5. In a clean, published, non-default managed worktree, run an authorized
   governed fetch, such as `git-cli sync-branch`. The identity guard resolves
   the trusted helper credential in an isolated context outside repository
   discovery, verifies its authenticated account against the
   selected profile, then pins that exact credential to the selected
   HTTPS host/repository for transport. It disables prompts and redirects.
   Missing credentials and account mismatches refuse before fetch/push;
   there is no automatic transport retry.
   For local sync diagnostics, use `--no-fetch --dry-run`: `--dry-run` alone
   still fetches and can update remote-tracking refs.
6. The maintainer verifies a real non-main push with `git-cli push` under the
   usual delivery authority. Review the destination and resulting remote
   branch. This hands-on acceptance is separate from local fixture tests.

If the authenticated account mismatches, repair the helper's account or the
identity profile through its owner, then retry the original governed command.
Do not paste tokens into remote URLs, command arguments, logs, or issue text.
API profile credentials and commit signing requirements remain independent.

## Switch back

```bash
git-cli transport set ssh
git-cli transport status --format json
```

This restores the canonical `git@<host>:<owner>/<repo>.git` remote shape for
the same target. The identity guard's existing SSH-shaped remote behavior is
unchanged: with an active identity policy, it still executes using the
profile-selected, verified credential over child-scoped pinned HTTPS.
Selecting an SSH URL does not enable native SSH authentication under policy.

The switch never writes global Git settings, changes credentials, or enables
URL rewrites. Existing default-branch, signing, ownership, review, and delivery
guards apply to either stored remote shape.
