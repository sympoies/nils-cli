# claude-cli

## Overview

`claude-cli` is a provider-specific Rust CLI for Claude-oriented helpers that
should not live in shell glue. It owns safe one-shot Claude Code execution,
upstream-owned authentication delegation, wrapper configuration,
prompt-segment rendering, usage source selection, cache fallback, session
resume, and completion export.

## Usage

```text
claude-cli agent prompt [--runtime safe|inherited] [--model <model>] [--effort <level>] [input...]
claude-cli agent advice [--runtime safe|inherited] [--model <model>] [--effort <level>] [input...]
claude-cli agent knowledge [--runtime safe|inherited] [--model <model>] [--effort <level>] [input...]
claude-cli agent commit [--auto-stage] [--push] [--model <model>] [--effort <level>] [extra...]
claude-cli agent doctor [--format text|json]
claude-cli agent resume <SESSION_ID> [--cd <dir>]
claude-cli auth login [--claudeai|--console] [--email <email>] [--sso]
claude-cli auth status [--format text|json]
claude-cli auth logout
claude-cli auth save [-y|--yes] <name|name.json> [--format text|json]
claude-cli auth use <name|name.json|email> [--format text|json]
claude-cli auth remove [-y|--yes] <name|name.json> [--format text|json]
claude-cli auth current [--format text|json]
claude-cli auth refresh <name>... [--accounts-dir <dir>] [--format text|json]
claude-cli auth auto-refresh [--accounts-dir <dir>] [--format text|json]
claude-cli auth remote export (--name <name>|--current|--all) --access-only
claude-cli auth remote pull --ssh <host> (--name <name>|--current) --access-only
                            --write-active [--keychain auto|required|off]
                            [--format text|json]
claude-cli auth remote pull --ssh <host> --all --into <accounts-dir> --access-only
                            [--keychain auto|required|off] [--format text|json]
claude-cli auth reset-rate-limits --program <juniper_tide|cedar_ember>
                                  [--request-id <uuid>] [-y|--yes]
                                  [--format text|json] <profile>
claude-cli config show
claude-cli config set <key> <value>
claude-cli diag rate-limits [<profile>] [--all] [--format text|json] [--one-line]
                            [--cached] [--async [--watch] [--jobs <n>]]
claude-cli prompt-segment [options]
claude-cli prompt-segment check
claude-cli prompt-segment status [--format text|json]
claude-cli usage [--format text|json] [--source auto|oauth|cli|cache]
                 [-c|--clear-cache] [-d|--debug]
claude-cli completion <bash|zsh>
```

## Scope boundary

| Job | Primary owner |
| --- | --- |
| Safe one-shot runtime, auth delegation, wrapper config, prompt/usage cache, rendering, completion | `claude-cli` |
| Authority profiles, refresh scheduling through Claude Code, access-only replica projection | `claude-cli` |
| Credential format, the OAuth exchange itself, browser login, managed policy | upstream Claude Code |
| Shell aliases, Starship wiring, PATH/fpath registration | shell integration |

Outside the token authority and replica commands described under
[Token authority and access-only replicas](#token-authority-and-access-only-replicas),
the wrapper does not read, copy, export, refresh, or directly delete Claude
Code credentials. Those commands never delete credentials either.

## Agent commands

- `agent prompt [input...]`: Run a raw one-shot prompt. If input arguments are
  omitted, read the prompt from stdin.
- `agent advice [input...]`: Run the versioned
  `nils-claude-cli.agent-advice.v1` engineering-advice template.
- `agent knowledge [input...]`: Run the versioned
  `nils-claude-cli.agent-knowledge.v1` explanation template.
- `agent commit [extra...]`: Generate a bounded structured commit message from
  staged context and invoke `semantic-commit` as the only commit writer.
- `agent doctor [--format text|json]`: Check Claude capabilities, the upstream
  installation doctor, and the `git` / `semantic-commit` dependencies without
  making a model call.
- `agent resume <SESSION_ID> [--cd <dir>]`: Resolve the recorded working
  directory from local Claude project history and launch
  `claude --resume <SESSION_ID>` there. `--cd` overrides resolution.

One-shot commands default to `--runtime safe`. Before launch, the wrapper
probes `claude --help` and fails with exit `69` when a required flag is absent.
The safe profile uses:

- `--safe-mode` and `--strict-mcp-config`;
- `--no-session-persistence`;
- `--permission-mode dontAsk`, disabled slash commands, and disabled Chrome;
- a `Read,Glob,Grep` allowlist for `prompt` and `advice`;
- an empty tool allowlist for `knowledge`.

Prompt input is limited to 1 MiB and delivered to Claude over stdin so it does
not appear in child-process argv. Templates, models, and flags remain discrete
argv entries without shell interpolation. `--runtime inherited` is an explicit
escape hatch that permits upstream customizations while keeping the
command-specific tool allowlist. Session persistence remains disabled by
default and can only be enabled for inherited mode with
`CLAUDE_CLI_NO_SESSION_PERSISTENCE=false`.

Claude `--safe-mode` still permits administrator-managed policy, upstream
authentication, model selection, and built-in permissions. It is a
Claude-specific safe boundary, not a claim of Codex-equivalent isolation.

### Agent commit safety contract

`agent commit` has no inherited-runtime fallback. It always:

- reads at most 2 MiB from `semantic-commit staged-context --format bundle`;
- rejects secret-like staged content with stable `nils-scrub` pattern ids
  before Claude is launched;
- sends that bundle and at most 64 KiB of optional guidance over stdin;
- runs Claude in a temporary working directory with safe mode, strict MCP,
  no session persistence, disabled slash commands/Chrome, and an empty tool
  list;
- validates Claude output against JSON Schema and local Conventional Commit
  constraints;
- rechecks both `HEAD` and the staged tree after model generation;
- invokes `semantic-commit commit --expect-head ... --automation` only when
  those checks still match;
- verifies the created commit has the captured parent and tree before reporting
  success or allowing a push.

`--auto-stage` explicitly runs `git add -A` before the snapshot. `--push`
requires an attached branch with a configured upstream, captures its effective
push endpoint before the model call, and revalidates that endpoint before
pushing only the verified commit through an explicit non-force refspec. The
push command uses the captured endpoint rather than resolving the mutable
remote alias again and pins exact-match command-scope URL rewrites so inherited
`insteadOf` / `pushInsteadOf` chains cannot retarget it. A model, schema, drift,
or commit failure leaves the index staged when no writer-side mutation is
observed. If `semantic-commit` times out or fails after changing repository
state, the wrapper preserves that state, skips push, and requires inspection
instead of claiming the index is still staged. A post-commit integrity or push
failure preserves and reports the local commit but never pushes an unverified
object.

`agent doctor` bounds and discards upstream stdout/stderr, verifies the exact
`semantic-commit staged-context` and `commit` help surfaces, and never copies
settings paths or other private diagnostic text into its JSON contract. A
successful diagnosis emits `claude-cli.agent.doctor.v1`; `ok: true` means the
checks completed, while `result.ready` and exit `0`/`1` carry readiness.
`commit_profile` covers the fixed safe commit profile;
`configured_commit_profile` also requires `--model` or `--effort` when those
wrapper overrides are configured.

## Authentication commands

- `auth login`: Delegate to `claude auth login`, including `--claudeai`,
  `--console`, `--email`, and `--sso`.
- `auth status [--format text|json]`: Call `claude auth status --json`, retain
  only public status classifications, and preserve upstream exit `0`/`1`
  authenticated meaning.
- `auth logout`: Delegate to `claude auth logout`.

Auth status drops email, organization identity, token, credential path, and
unknown upstream fields.

### Limit resets

`auth reset-rate-limits --program <juniper_tide|cedar_ember> <profile>`
redeems at most one Claude limit reset for a stored profile:
`juniper_tide` is the weekly reset of the 5-hour session limit, offered only
at that limit, and `cedar_ember` uses the next granted reset.

- Non-interactive and JSON runs require `--yes` and a canonical lowercase
  UUID `--request-id`; reuse the same id when retrying after an unknown
  result. Without `--yes` an interactive run asks for confirmation first.
- The stored token is never refreshed; an expired token fails before any
  request. The command re-reads the usage status the way `diag rate-limits`
  does and reports `unavailable` without posting when the program cannot be
  used. Otherwise it sends one `POST /api/organizations/<org>/reset_rate_limits`
  and never retries it. For `cedar_ember` the grant id comes from that status
  read.
- `--format json` emits `claude-cli.auth.reset-rate-limits.v1` with
  `program`, `outcome` (`reset`, `already_used`, `not_limited`, `cooldown`,
  `ineligible`, `unavailable`), `posted`, `reason`, `resets_left`,
  `next_available_at`, `cooldown_until`, and `weekly_resets_at`. Every outcome
  exits `0`; an expired or rejected sign-in exits `2`, and a provider failure
  exits `3` (`error.details.retryable` says whether to retry with the same id).
- Output never contains tokens, organization or account uuids, grant ids, the
  request id, or the profile path. See the
  [reset contract](docs/specs/claude-cli-auth-reset-rate-limits-json-contract-v1.md).

### Token authority and access-only replicas

One host (the authority) keeps a refresh-capable login per account as a named
profile. Every other host pulls an access-only copy over SSH and never holds a
refresh token, so the authority is the only refresher. This is the same model
as `codex-cli auth remote`, and both use the shared
`nils-common::provider_runtime::remote` transport.

- `auth save [-y] <name>`: Store the active login (made with `/login`) as
  profile `<name>` in `CLAUDE_SECRET_DIR`, then rewrite that source login
  access-only so the profile is the only refresh-token holder. Refuses a login
  without a refresh token and, even with `--yes`, an existing profile that
  belongs to another account (`profile-identity-mismatch`, exit `65`).
  Replacing a profile of the same account asks `[y/N]` on a terminal; `--yes`
  skips the prompt, and JSON output or a non-interactive run without `--yes`
  fails with `overwrite-confirmation-required` (exit `1`).
- `auth use <name|name.json|email>`: Make the matching profile the current
  default and write it as the local active login, access-only. A target with
  `@` matches a profile's `oauthAccount.emailAddress`; a bare target that is
  not a profile name matches the email local part. Several matches exit `2`
  (`ambiguous-profile`, with `candidates`), no match exits `1`
  (`profile-not-found`), and an invalid name exits `64`.
- `auth remove [-y] <name>`: Delete `<name>.json` from `CLAUDE_SECRET_DIR`.
  Refuses the current default (`profile-is-current-default`, exit `1`) and a
  missing profile (`profile-not-found`, exit `1`). It asks `[y/N]` on a
  terminal; JSON output or a non-interactive run without `--yes` exits `64`
  (`usage-error`).
- `auth current`: Report the current default, its account identity, access
  token expiry, and the stored profile names. Never prints tokens. Exits `2`
  with `matched: false` when no current default is recorded.
- `auth refresh <name>...` / `auth auto-refresh`: Exchange a profile's refresh
  token through Claude Code's documented `CLAUDE_CODE_OAUTH_REFRESH_TOKEN` +
  `CLAUDE_CODE_OAUTH_SCOPES` `claude auth login` path, in a private temporary
  config dir. The rotated login replaces the profile only when it belongs to
  the same account; the current default is re-projected. `auto-refresh`
  refreshes profiles with less than `CLAUDE_AUTH_REFRESH_MARGIN_SECONDS`
  (default four hours) of access token left. Exits `1` when any profile fails.
  Refresh needs file credential storage, so it refuses Keychain hosts (macOS).
  If the exchange succeeds but the result cannot be stored as the profile (for
  example, it belongs to another account), the rotated login is kept in
  `<profile>.refresh-quarantine` (mode 0600) instead of being discarded.
  With `--accounts-dir <dir>` (or `CLAUDE_ACCOUNTS_DIR`), each refreshed
  profile is also re-projected access-only into `<dir>/<profile>/` (files
  only); the JSON result lists them in `projected`, and a failed projection is
  reported in `projection_failed` and exits `1`.
- `auth remote export`: Print a profile's access-only payload (`profile`, the
  access fields of `claudeAiOauth`, `oauthAccount`) for SSH transport. `--all`
  prints `{"current": <name|null>, "profiles": [<payload>...]}` in one call.
- `auth remote pull`: Run `claude-cli auth remote export` on the authority and
  write the result as the active login.
- `auth remote pull --all --into <accounts-dir>`: Export every profile in one
  SSH round trip and write each into its own Claude Code config dir,
  `<accounts-dir>/<profile>/`, for use as `CLAUDE_CONFIG_DIR`. The default
  login is not touched. See [Per-account config directories](#per-account-config-directories).

`save`, `use`, `remove`, and `refresh` hold an exclusive lock on
`CLAUDE_SECRET_DIR/.lock`, so profile and active-login writes never interleave.
A confirmation prompt is answered before the lock is taken, and the checks run
again under the lock.

Name resolution, the confirmation flow, and these exit codes are shared with
`codex-cli auth` through `nils-common::provider_runtime::accounts`.

#### Behavior changes (account-management parity)

Callers written against earlier `claude-cli` releases must update:

- `auth current` exits `2` (result `matched: false`) when no current default
  is set; it used to exit `0`. Do not treat exit `0` from `auth current` as a
  capability probe; check for the new surface with
  `claude-cli auth remove --help` instead.
- `auth save` over an existing profile of the same account now needs
  `-y/--yes` in JSON or non-interactive runs; without it the command exits `1`
  with `overwrite-confirmation-required`. The same applies when that profile
  appears between the confirmation check and the locked write.
- `auth use` on a missing profile exits `1` (`profile-not-found`) instead of
  `65`.

An access-only login stores `"refreshToken": ""`, which Claude Code treats as
having no refresh token, so it never calls the token endpoint. A running Claude
Code session picks up a replaced login on its next request. Only
`claudeAiOauth` is replaced in `.credentials.json`, so other entries such as MCP
server tokens are kept, and `oauthAccount` in the Claude Code config is
rewritten only when it changes.

On macOS Claude Code reads its login from the login Keychain before the file,
so the projection also writes the `Claude Code-credentials` item (with
Claude Code's config-dir suffix when `CLAUDE_CONFIG_DIR` is set). The secret is
passed to `security -i` on stdin, never on argv; an item too large for one
`security -i` line (4032 characters, Claude Code's own limit) is not written. `--keychain auto` reports
`unavailable` when the Keychain is locked (for example over SSH), `required`
fails instead, and `off` skips it. Run the pull from the GUI session (a
LaunchAgent) so the Keychain is writable.

### Per-account config directories

`auth remote pull --all --into <accounts-dir>` keeps one config dir per
authority profile, so separate sessions can run as separate accounts with
`CLAUDE_CONFIG_DIR=<accounts-dir>/<profile>`. For each profile it writes:

- `.credentials.json`: a real file (mode 0600) with the access-only
  `claudeAiOauth` and `"refreshToken": ""`; other entries are kept.
- `.claude.json`: `oauthAccount` is set; a missing file is created with
  `{"hasCompletedOnboarding": true}` plus the account, and an existing file
  keeps everything else.
- `.claude-cli-account`: the ownership marker.
- On a Keychain host, the item `Claude Code-credentials-<first 8 hex of
  sha256(<config dir>)>`, the name Claude Code uses for that
  `CLAUDE_CONFIG_DIR`. The path is made absolute but not canonicalized, so set
  `CLAUDE_CONFIG_DIR` to the reported `config_dir` exactly.

An existing non-empty directory without the marker is never adopted: that
profile fails with `account-dir-not-owned`, and profile names made only of dots
are refused. Directories of profiles that no longer exist on the authority are removed, but
only real directories that hold the ownership marker; symlinks inside them are
unlinked, never followed. An export with no profiles is refused, so nothing is
pruned. Shared files such as `settings.json` or `projects/` are not managed
here. The JSON result (`claude-cli.auth.v1`) reports `current`, `into`,
`pruned`, and per profile `name`, `config_dir`, `written`, `keychain`,
`expires_at`, and `has_refresh_token` (always `false`); a profile that could
not be written carries an `error` and makes the command exit `1`. Pulls and
refresh projections hold `<accounts-dir>/.lock`.

## Configuration commands

- `config show`: Print the effective wrapper model, effort, runtime, and
  no-session-persistence values. Invalid configured values fail with exit `64`
  instead of being silently replaced by defaults.
- `config set <key> <value>`: Validate a supported key and emit one safely
  quoted POSIX-shell `export`.

Supported keys are `model`, `effort`, `agent-runtime`, and
`no-session-persistence`. These commands never modify Claude settings or
credential files.

## Prompt segment

- `prompt-segment [--no-5h] [--ttl <duration>] [--time-format <strftime>]
  [--show-timezone] [--refresh]`: Render Claude usage.
- `prompt-segment check`: Exit `0` when a Claude OAuth token is available,
  otherwise `1`.
- `prompt-segment status [--format text|json]`: Report cache readiness without
  exposing token material.

Output:

```text
5h:<remaining>% W:<remaining>% <weekly_reset_time>[<stale_suffix>]
```

Without `--refresh`, stale or missing cache starts a coalesced detached refresh
after any eligible cached line is printed. The prompt path does not wait for
the network request. `--refresh` is the explicit blocking operation.
`--no-5h` hides the five-hour window. `--show-timezone` adds the local UTC
offset to the default time format; explicit `--time-format` takes precedence.

Cached values are display-eligible while cache mtime is less than 600 seconds
old. A timestamp up to five seconds in the future is tolerated. Expired files
are retained but contribute no prompt or usage windows.

## Usage command

- `usage [--format text|json] [--source auto|oauth|cli|cache]`: Read Claude
  usage through a service-consumable contract.
- `auto`: Try OAuth, then a bounded native Claude `/usage` probe, then cache.
- `oauth`, `cli`, and `cache`: Select one source for focused diagnostics.
- `-c, --clear-cache`: Remove the resolved `usage.json` and its
  `usage.refresh.at` throttle stamp before querying, so the next background
  refresh is not suppressed while no cache remains. The cache directory and the
  refresh locks a running refresh may hold are left alone. Combining it with
  `--source cache` is rejected with exit `64`.
- `-d, --debug`: Report one bounded line per attempted source on stderr
  (`source`, `outcome`, optional `reason`, and `elapsed_ms`). Stdout stays
  exactly one versioned envelope, so debug mode never changes what a
  `claude-cli.usage.v1` consumer parses.

JSON uses `claude-cli.usage.v1`. It includes provider, source, stale state,
normalized windows, and an optional provider-neutral `reason_code`. Provider
responses, terminal errors, and credentials are classified locally and never
forwarded.

## Diagnostics: rate limits

`diag rate-limits` reports Claude OAuth rate limits in the shared
`diag rate-limits` shape that `codex-cli diag rate-limits` also emits, so one
collector reads both.

- Targets: `--all` (and `--async`) reads every `<name>.json` profile in
  `CLAUDE_SECRET_DIR`; `<profile>` reads one; no target reads the active login
  (`$CLAUDE_CONFIG_DIR/.credentials.json`, then the macOS Keychain item that
  Claude Code uses). With `CLAUDE_RATE_LIMITS_DEFAULT_ALL_ENABLED=true`, a
  text run with no target and no `--cached` reads every profile as `--all`
  does; JSON output and named profiles are unchanged.
- Each target sends its stored access token once to the OAuth usage endpoint,
  with Claude Code's status query (`?at_wall=1&skip_spend=1`, appended after
  any query an endpoint override already has) and
  `User-Agent: claude-cli/<Claude Code version> (external, cli)`. The version
  is `CLAUDE_RATE_LIMITS_CLAUDE_CODE_VERSION` when it is `MAJOR.MINOR.PATCH`,
  else the leading version of `claude --version` (once per run, three-second
  deadline), else `2.1.284`; `CLAUDE_PROMPT_SEGMENT_USER_AGENT` overrides the
  whole header. `five_hour` becomes the `5h` window (300 minutes) and `seven_day` the
  `Weekly` window (10080 minutes). Tokens are never refreshed, rewritten, or
  printed. An expired token reports `auth_expired` without a request. HTTP
  `401`, `403`, and `429` map to `auth_expired`, `permission_denied`, and
  `rate_limited`; any other failure is `service_unavailable`.
- `--format json` (or `--json`) emits `claude-cli.diag.rate-limits.v1`. `--all`
  and `--async` emit one result per profile (`name`, `target_file`, `status`,
  `ok`, `source`, `reason_code`, `summary`, `windows`, `error`) and exit `1`
  when any result failed, which is a result without `windows`. A single target
  exits `1` on failure.
- Each network result also carries `limit_resets`: the normalized
  `juniper_tide` and `cedar_ember` reset status, each `null` when the upstream
  block is missing or malformed. A malformed block never fails the result.
  Cached and cache-fallback results omit `limit_resets`. The shape is in the
  [JSON contract](docs/specs/claude-cli-json-contract-v1.md#diag-rate-limits-limit-resets);
  grant ids stay host-local.
- Text output prints the shared accounts table for `--all` and `--async`, and
  `--one-line` prints `5h:<n>% W:<n>% <reset>`. `--async` queries profiles
  concurrently (`--jobs`, default 5) and falls back to the last cached values
  on failure; `--watch` redraws every 60 seconds. Reading more than one
  profile shows a progress bar on an interactive stderr.
- Each successful read caches its windows under the prompt-segment cache
  directory in `diag-rate-limits/<name>.kv`. `--cached` reads only that cache,
  within `CLAUDE_RATE_LIMITS_CACHE_TTL` (default 180 seconds) unless
  `CLAUDE_RATE_LIMITS_CACHE_ALLOW_STALE=true`.

`claude-cli usage` is unchanged and remains the prompt-segment usage reader.

## Completion

`completion <bash|zsh>` exports clap-generated shell completion to stdout.

## Environment

- `CLAUDE_CLI_BIN`: Claude executable for agent/auth; default `claude`.
- `CLAUDE_CONFIG_DIR`: Claude Code config dir for the active login; default
  `~/.claude` (with `~/.claude.json`).
- `CLAUDE_SECRET_DIR`: authority profile dir; default `~/.config/claude_secrets`.
- `CLAUDE_AUTH_REFRESH_MARGIN_SECONDS`: `auto-refresh` margin; default `14400`.
- `CLAUDE_AUTH_KEYCHAIN`: `on` or `off` overrides the macOS Keychain default.
- `CLAUDE_ACCOUNTS_DIR`: default `--accounts-dir` for `refresh` and
  `auto-refresh`.
- `CLAUDE_CLI_MODEL`, `CLAUDE_CLI_EFFORT`: one-shot defaults.
- `CLAUDE_CLI_AGENT_RUNTIME`: `safe` (default) or `inherited`.
- `CLAUDE_CLI_NO_SESSION_PERSISTENCE`: default `true`; safe mode always
  disables persistence.
- One-shot capability probes and auth-status delegation bound captured output
  while the child is running. They terminate the child process group after
  five and three seconds, respectively.
- Commit generation caps captured Claude output at 1 MiB with a five-minute
  deadline. `agent doctor` caps upstream diagnostic output at 256 KiB with a
  15-second deadline.
- Auto-stage has a 60-second deadline. Commit creation and push each have a
  120-second deadline. All bounded subprocess caps are aggregate across stdout
  and stderr, and deadline/limit failure terminates the child process group.
- `CLAUDE_PROMPT_TTL`, `CLAUDE_PROMPT_SEGMENT_TTL`: cache TTL; default `60`
  seconds. `0` forces blocking refresh.
- `CLAUDE_PROMPT_STALE_SUFFIX`,
  `CLAUDE_PROMPT_SEGMENT_STALE_SUFFIX`: stale suffix.
- `CLAUDE_PROMPT_SEGMENT_CACHE_DIR`: cache-directory override.
- `CLAUDE_PROMPT_SEGMENT_ENDPOINT`: usage-endpoint override, also used by
  `diag rate-limits`.
- `CLAUDE_RATE_LIMITS_CACHE_TTL`, `CLAUDE_RATE_LIMITS_CACHE_ALLOW_STALE`:
  `diag rate-limits --cached` freshness.
- `CLAUDE_RATE_LIMITS_DEFAULT_ALL_ENABLED`: default `diag rate-limits` to
  `--all` when no target is provided (default: `false`).
- `CLAUDE_RATE_LIMITS_CLAUDE_CODE_VERSION`: Claude Code version for the
  `diag rate-limits` and `auth reset-rate-limits` User-Agent; default: detected
  from `claude --version`, else `2.1.284`.
- `CLAUDE_RATE_LIMITS_API_BASE_URL`: `auth reset-rate-limits` API base;
  default: the usage endpoint's origin.
- `CLAUDE_RATE_LIMITS_RESET_MAX_TIME_SECONDS`: reset POST timeout; default
  `25`, at most `120`.
- `CLAUDE_PROMPT_SEGMENT_REFRESH_MIN_SECONDS`: detached refresh cooldown;
  default `60`.
- `CLAUDE_PROMPT_SEGMENT_EXE`: detached self-refresh executable override.
- `CLAUDE_PROMPT_SEGMENT_ZSH_ESCAPE_ENABLED=1`: double percent characters.
- `CLAUDE_PROMPT_SEGMENT_ACCESS_TOKEN`,
  `CLAUDE_PROMPT_SEGMENT_CREDENTIALS_JSON`: automation credentials.
- `CLAUDE_PROMPT_SEGMENT_CLAUDE_BIN`: native CLI usage-probe executable.
- `CLAUDE_PROMPT_SEGMENT_CLAUDE_TIMEOUT_SECONDS`: bounded CLI usage timeout;
  default `15`.
- `CLAUDE_PROMPT_SEGMENT_CLAUDE_PTY_DISABLED=1`: disable Unix PTY probing.
- `CLAUDE_PROMPT_SEGMENT_CLAUDE_PTY_STARTUP_DELAY_MS`,
  `CLAUDE_PROMPT_SEGMENT_CLAUDE_PTY_USAGE_DELAY_MS`: PTY timing controls.
- `CLAUDE_PROMPT_SEGMENT_KEYCHAIN_DISABLED=1`: disable macOS Keychain lookup.
- `CLAUDE_PROMPT_SEGMENT_KEYCHAIN_SERVICE`: Keychain service override.
- `NO_COLOR=1`: disable ANSI color.

## Dependencies

- `claude` is required for agent, auth, resume, and the optional CLI usage
  fallback.
- `git` and `semantic-commit` are required for `agent commit`; doctor reports
  both dependencies without modifying a repository.
- macOS `security` is used for prompt-segment Keychain lookup unless an
  automation credential override is supplied.
- Unix `script` enables the richer native CLI usage-probe path.
- Resume reads `$CLAUDE_CONFIG_DIR/projects` (default `~/.claude/projects`)
  through `nils-provider-resume`.

## Exit codes

- `0`: success, help, or no prompt output needed.
- `1`: operational false/failed state, including unauthenticated status.
- `2`: ambiguous profile target, no current default, or a reset sign-in that
  must be renewed.
- `3`: `auth reset-rate-limits` provider failure.
- `64`: usage or argument error.
- `65`: invalid input data, invalid structured commit output, or unresolved
  resume session.
- `69`: required executable or Claude capability unavailable.

## Docs

- [Docs index](docs/README.md)
- [JSON contracts](docs/specs/claude-cli-json-contract-v1.md)
- [Reset rate-limits contract](docs/specs/claude-cli-auth-reset-rate-limits-json-contract-v1.md)
- [Usage consumer runbook](docs/runbooks/usage-consumer.md)
