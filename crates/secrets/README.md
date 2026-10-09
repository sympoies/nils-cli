# nils-secrets

`secrets` pulls and pushes dotenv entries in a SOPS-encrypted store. From an
application checkout, it uses the git `origin` remote to select an entry unless
an explicit name is supplied.

## Commands

```bash
secrets pull [name] [--output <path>] [--force]
secrets add [file]
secrets list
secrets which [name]
secrets edit [name]
secrets completion bash
secrets completion zsh
```

`[name]` overrides automatic entry detection: a bare name resolves against
`repos/` and then `stacks/`, or a store-relative entry path can be supplied.

## Pull output

`pull` writes to `./.env` by default. `--output` selects another destination;
existing destinations are preserved unless `--force` is used. Relative output
paths are resolved from the current directory. Created output files have mode
`600`.

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
resolved from that file's directory. `secrets which` explains the selected
store source and matching selector.

## Output modes and secret handling

Default output is human-readable text. `--format json` emits one versioned
envelope (`schema_version` / `ok` / `data` or `error`) per the
[CLI Service JSON Contract Guideline](../../docs/specs/cli-service-json-contract-guideline-v1.md).

Standard output and the JSON envelope carry metadata only: store paths, entry
names, booleans, and counts. Decrypted values are redirected to the selected
mode-600 output file and are never echoed to standard output or included in the
JSON envelope.

`add` encrypts into a hidden mode-600 temporary output beside the final entry,
asks SOPS to decrypt and validate the complete temporary document, and only
then atomically installs it over the tracked target. The sibling location
ensures a same-filesystem rename. Encryption failure, invalid output, or a
handled signal before installation leaves prior ciphertext unchanged and
removes the temporary output. After installation, the transaction completes
its Git operations. Hermetic integration tests use stub executables and
synthetic stores to exercise these contracts.

## Exit codes

| Code | Meaning |
| ---- | ------- |
| `0` | success |
| `1` | runtime error |
| `64` | command-line usage error |
| `65` | no store entry for the requested target / missing source file |
| `69` | the store, `sops`, or `git` is unavailable |

## Dependencies

Requires `git` and `sops` on `PATH`, plus a SOPS decryption identity configured
for the selected store.
