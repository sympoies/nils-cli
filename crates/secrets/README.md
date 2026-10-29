# nils-secrets

`secrets` pulls, adds, and sets dotenv entries in a SOPS-encrypted store. From an
application checkout, it uses the git `origin` remote to select an entry unless
an explicit name is supplied.

## Commands

```bash
secrets pull [name] [--output <path>] [--force]
secrets add [file]
secrets set <name> <key> --value-stdin
secrets list
secrets which [name] [--explain]
secrets edit [name]
secrets completion bash
secrets completion zsh
```

`[name]` overrides automatic entry detection: a bare name resolves against
`repos/` and then `stacks/`, or a store-relative entry path can be supplied.

## Pull output

`pull` refreshes `./.env` by default. `--output` selects another destination;
an existing explicit destination is preserved unless `--force` is used.
Relative output paths are resolved from the current directory. Created output
files have mode `600`.

## Set one key from stdin

`secrets set <name> <key> --value-stdin` creates an entry or replaces exactly
one key. It uses the same store and entry-name lookup as `pull` and `which`.
Keys use ASCII letters, digits, and underscores, with a letter or underscore
first; the `sops_` prefix is reserved. Other keys and comments are preserved.
Existing documents must contain unique single-line `KEY=value` assignments,
blank lines, or `#` comments; ambiguous/unsupported input is refused unchanged.

Pipe the value from a producer that keeps it out of arguments and logs:

```bash
secret-producer | secrets set service TOKEN --value-stdin
```

Stdin must be non-terminal UTF-8, read through EOF, at most 1 MiB. Empty input
sets an empty string. Every byte is significant: a final LF is part of the
value, so producers must choose their newline behavior deliberately. NUL, CR,
and control characters other than LF and TAB are refused. Values become
single-line, double-quoted dotenv assignments: quotes, backslashes, dollar
signs, LF, and TAB are escaped as `\"`, `\\`, `\$`, `\n`, and `\t`. There is
no shell evaluation. Terminal input is refused before reading to prevent echo;
there is no interactive prompt, value argument, or plaintext input file.
Existing decrypted documents and SOPS output are limited to 16 MiB.

Before reading stdin or writing, `set` requires the selected store root to be
a clean linked worktree reported as managed by `git-cli worktree list`, on an
attached branch different from the locally resolved `origin/HEAD`. Create it
with `git-cli worktree add` in the store repository, set the local remote HEAD
if needed, and point `SECRETS_REPO` at that worktree. Primary, unmanaged,
detached, dirty, unresolved-default, and symlink entry paths are refused.
No store is fetched or automatically moved to another checkout.

Both new and existing entries are re-encrypted using the selected store's
root `.sops.yaml` creation rules for the **final entry path**, including
recipient changes. Recipient/config environment overrides are ignored during
encryption; decryption identities still use the normal SOPS configuration.
Rules that leave assignment values unencrypted are refused. A mode-600 sibling
contains ciphertext only; SOPS decrypts and MAC-verifies the complete candidate
before atomic same-filesystem rename. Before the installation commit point,
input, encryption, validation, or handled SIGINT/SIGTERM failure preserves prior
ciphertext and removes owned temporary files. The worktree is rechecked before
installation. Handle only one writer per store worktree. Uncatchable termination
(such as SIGKILL) may leave a private ciphertext temporary file to remove after
inspection; it never contains plaintext.

A successful exit means **encrypted write completed**. Signed commit/PR
delivery is explicitly handed off and remains **pending**. `set` does not stage,
commit, push, or contact a provider. Text and JSON report the entry, store,
branch, `encrypted_write_completed`, `delivery_verified`, and next actions;
`committed` and `pr_created` are false and `delivery_status` is `pending`.
In that store worktree, review and stage only the reported entry with
`git add -- <entry>`, use `semantic-commit` for a signed commit, publish with
`git-cli push`, and use `forge-cli pr deliver --no-merge` with metadata-only
text. Verify the signature and exact PR head through that governed workflow.
Signing or provider failure leaves the encrypted write pending: preserve it,
repair delivery, and do not replay `set`. A subsequent call refuses the dirty
worktree. Unrelated changes must be handled separately.

## Store selection

The optional TOML file at `$XDG_CONFIG_HOME/secrets/stores.toml` (or
`~/.config/secrets/stores.toml` when `XDG_CONFIG_HOME` is unset) supports a
configured default, checkout path prefixes, and git remote selectors:

```toml
default = "/srv/secrets/default"

[path_prefixes]
"/work/team" = "/srv/secrets/team"

[remotes]
"github.com/example" = "/srv/secrets/example"
"git.example/group" = "/srv/secrets/group"
```

Selection precedence is `SECRETS_REPO`, the longest matching checkout path
prefix, the longest matching remote host/owner/repository prefix, the configured
default, then `$XDG_DATA_HOME/secrets/store` (or
`~/.local/share/secrets/store`). Relative store paths in the TOML file are
resolved from that file's directory. Checkout path-prefix keys must be absolute;
matching uses lexically normalized paths and does not resolve symlink aliases
or expand `~`. Remote selectors are case-insensitive host/path prefixes and
match only at path-segment boundaries. `secrets which --explain` reports the
selected store source and matching selector. JSON output includes this
selection metadata as well.

## Output modes and secret handling

Default output is human-readable text. `--format json` emits one versioned
envelope (`schema_version` / `ok` / `data` or `error`) per the
[CLI Service JSON Contract Guideline](../../docs/specs/cli-service-json-contract-guideline-v1.md).

Standard output and the JSON envelope carry metadata only: store paths, entry
names, booleans, and counts. `pull` redirects decrypted values to the selected
mode-600 output file; `set` keeps its value and decrypted document in memory.
Values are never echoed to standard output or included in the JSON envelope.

`add` encrypts into a hidden mode-600 temporary output beside the final entry,
asks SOPS to decrypt and MAC-verify the complete temporary document, and only
then atomically installs it over the tracked target. The sibling location
ensures a same-filesystem rename and supports linked worktrees. The target never
contains plaintext; encryption failure, invalid output, or a handled signal
before installation leaves prior ciphertext unchanged and removes the temporary
output. After installation, handled signals are ignored while the add
transaction completes its Git operations. On Unix, Git children are isolated
from foreground process-group signals until add, commit, and push complete.
Hermetic integration tests use a synthetic secret canary and assert it never
appears on standard output or standard error.

## Exit codes

| Code | Meaning |
| ---- | ------- |
| `0` | success |
| `1` | runtime error |
| `64` | command-line usage error |
| `65` | no store entry for the requested target / missing source file |
| `69` | the store, `sops`, or `git` is unavailable |

## Dependencies

Requires `git` and `sops` on `PATH`; `set` also requires `git-cli` for managed
worktree verification. Configure a SOPS decryption identity for the selected
store.
