# Operator rules file

Every parsed `agent-session` command, including hook-invoked mailbox commands
and `serve`, loads the same local JSON operator rules file. Missing files use
built-in defaults.

## Location and format

`AGENT_SESSION_RULES_FILE` selects the file. Otherwise its location is
`$XDG_CONFIG_HOME/agent-session/rules.json`, or
`$HOME/.config/agent-session/rules.json` when `XDG_CONFIG_HOME` is unset or empty.
CLI processes and serve must use the same location to apply the same rules.
This file is separate from the deployment inputs supplied by `serve --config`.

```json
{
  "schema_version": "agent-session.operator-rules.v1",
  "coordinator_requires_root": true
}
```

`schema_version` is required. Omitted `coordinator_requires_root` means `true`.
This rule covers both `coordinator` and `domain-coordinator`: when true they
require a root lineage during local and Console starts, daemon relays, HTTP
create/resume, and display metadata role updates. Setting it to false allows
those roles on parented sessions. Lineage depth, authentication, work references,
and other launch restrictions retain their existing behavior.

Only the documented keys and types are accepted. The document must be a JSON
object no larger than 64 KiB. The loader never applies part of an invalid file.

## Validate and inspect

```bash
agent-session rules --check --format json
agent-session serve --check --format json
agent-session rules --format json
```

Both check commands use the same loader and return exit 64 on invalid rules,
with the file and key in the error. `serve --config deployment.toml --check`
validates operator rules before checking deployment inputs. Checks never start
a daemon. A missing rules file is valid and reports built-in defaults.

`rules` reports the schema version, selected file, effective values, and source
(`built_in` or `file`). To inspect the running daemon, read
`GET /operator-rules`; its response also reports `retained_last_good`, which is
true when the current file is invalid and serve kept its previous rules.
The endpoint is a loopback-readable configuration summary, like session reads;
it contains no document contents beyond the supported effective rules.

## Invalid files and reloads

A normal CLI invocation loads once. If reading or validating the file fails,
it emits one warning naming the file and key, then uses all built-in defaults.
Mailbox send/read and other commands continue. A false value in an invalid
file cannot relax the built-in root restriction.

Serve reloads before each root-role check and configuration inspection. It
keeps its last completely valid rule set on failure, or built-in defaults if
none has loaded. A repeated identical error warns once until a successful load;
a different error emits a new warning. Repairing the file applies the new valid
rules on the next check. Removing it deliberately restores built-in defaults.
Use an atomic file replacement to deploy a complete document.

The destination notification policy remains a separately loaded policy with
its existing malformed-file error behavior. Operator rules do not change it.
