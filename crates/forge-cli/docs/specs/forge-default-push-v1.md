# forge-cli: Default-branch publication

Part of the [forge-cli v1 contract](forge-cli-spec-v1.md). Shared provider
parity, naming, [lock-down rules](forge-cli-spec-v1.md#lock-down-policy),
[exit codes](forge-cli-spec-v1.md#exit-code-map), and
[configuration](forge-cli-spec-v1.md#configuration) remain in the main spec.
The [operation catalog](forge-cli-ops-v1.yaml) owns backend mappings,
validation rules, and output schema versions.

## Operations

### `repo push-default`

- Input: `--head <ref>` (default `HEAD`, and must resolve to the checked-out
  commit), required `--expected-base <full-sha>`, and required
  `--reason-file <path>` naming a regular file of at most 2,000 bytes that
  contains the caller's explicit authorization basis.
- Validation:
  - the selected Git remote must expose exactly one actual push URL (including
    any configured `remote.<name>.pushurl`). Before provider metadata is read,
    the push destination must classify to the selected provider (unless an
    explicit provider plus custom host selected an otherwise unclassified
    authority), and its canonical authority must exactly equal the selected
    authority. The provider metadata URL must then match that same authority
    and repository identity. Canonical comparison maps provider transport
    aliases to their API authority, normalizes HTTPS port `443`, and preserves
    every non-default HTTPS port. Metadata from one authority cannot authorize
    a push to another; the local backend is unsupported;
  - HTTP(S) push URLs containing userinfo are rejected; callers use credential
    helpers rather than embedding credential material in a subprocess argument;
  - after the push URL is expanded once, no effective `url.*.insteadOf` or
    `url.*.pushInsteadOf` rule may match it again; empty rewrite prefixes are
    universal matches and are rejected. This prevents Git from retargeting the
    mutation independently of base/read-back operations;
  - the worktree is clean and checked out on a non-default branch;
  - the remote default branch still equals `--expected-base`;
  - `HEAD` is exactly one commit ahead of that base and the base is its
    ancestor;
  - `git log --format=%G?` reports `G` for the delivered commit.
- Mutation: exactly one command-scoped `git -c push.followTags=false -c
  push.pushOption= -c push.recurseSubmodules=no push --porcelain --no-follow-tags
  --no-recurse-submodules --no-push-option
  --force-with-lease=refs/heads/<default>:<expected-base> --
  <validated-push-url> <head-sha>:refs/heads/<default>` invocation. The exact
  old-OID lease is used only as a compare-and-swap guard; the independent
  ancestry proof keeps the update fast-forward-only. There is no unconstrained
  force, delete, retry, or direct-merge option, and inherited tag, submodule,
  and push-option expansion is disabled. Any concurrent remote change is
  returned as `default_push_rejected`.
- Destination pinning: the expected-base read, push, and post-push read-back all
  use the same validated URL rather than re-resolving the remote name.
- Resource bounds: every Git subprocess has a 120-second timeout and an 8 MiB
  per-stream capture limit. Timeout or output-limit failure kills the complete
  child process group and returns `git_timeout` or `git_output_limit`
  without retrying a push. The provider metadata subprocess uses the same
  finite deadline and backend capture/process-group contract. Shipped release
  builds always execute `git` with these hard bounds; executable and bound
  overrides exist only in debug test builds.
- Read-back: exact `git ls-remote` after the push must equal the delivered SHA;
  otherwise the op fails closed with `default_push_verification_failed` and
  tells the caller to inspect the already-mutated remote.
- Output schema: `cli.forge-cli.repo.push-default.v1`; the receipt contains the
  repository, remote/default branch, authoring branch, head/base SHAs, reason,
  exact refspec, `pushed`, and `observed_remote_sha`.
- Typed failures: `push_destination_missing`,
  `push_destination_ambiguous`, `push_destination_credentials_unsupported`,
  `push_destination_rewrite_ambiguous`,
  `provider_mismatch`, `provider_unsupported`, `repository_mismatch`,
  `dirty_worktree`, `detached_head`, `default_branch_checkout`,
  `head_not_checked_out`, `expected_base_mismatch`,
  `expected_base_missing`, `expected_base_not_ancestor`,
  `direct_commit_count_invalid`, `commit_signature_unverified`,
  `reason_file_unreadable`, `reason_invalid`, `local_path_present`,
  `object_id_invalid`, `remote_default_lookup_failed`,
  `remote_default_branch_missing`, `git_timeout`, `git_output_limit`,
  `backend_timeout`, `backend_output_limit`,
  `default_push_rejected`, `default_push_verification_failed`, and
  `software_error`. Callers must preserve their declared DATA, RUNTIME,
  UNAVAILABLE, SOFTWARE, or USAGE exit class.
