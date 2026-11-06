# Governed HTTPS transport

Use an explicit HTTPS remote when the repository needs the configured Git
credential helper for governed fetch/push. This procedure changes only the
repository-local remote URL; linked worktrees share that choice. It does not
change forge accounts, credential storage, signing, or delivery authorization.

## Select and verify HTTPS

1. Configure an authorized system/global Git credential helper for the forge.
   The helper must honour the requested username and return that user's token.
   Repository-local and worktree-local helper entries, including
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

## Multi-account helper and isolated testing

A trusted helper can select the requested account without changing the active
account. For example, a GitHub helper may use this shape (the requested account
must already be authenticated):

```sh
#!/bin/sh
[ "$1" = get ] || exit 0
host= username=
while IFS= read -r line && [ -n "$line" ]; do
  case "$line" in
    host=*) host=${line#host=} ;;
    username=*) username=${line#username=} ;;
  esac
done
[ -n "$host" ] && [ -n "$username" ] || exit 0
token=$(gh auth token --hostname "$host" --user "$username") || exit 1
printf 'username=%s\npassword=%s\n' "$username" "$token"
```

Configure the helper through its trusted system/global configuration owner.
A helper that ignores the username may return another account; the guard still
verifies the returned credential and refuses a mismatch before transport.
For isolated testing, point `GIT_CONFIG_GLOBAL` at a scratch configuration
file with an `[include]` pointing to the real signing configuration and a
helper entry pointing to the test helper. This preserves signing settings while
leaving shared configuration and active accounts unchanged. Exercise both a
matching response and a mismatched response before removing the scratch files.

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
