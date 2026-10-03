# Peekaboo Backend Upgrade

## Stable identities

The GUI authority is the signed Peekaboo app at
`$HOME/Applications/Nils CLI Peekaboo.app`, with bundle identifier
`boo.peekaboo.mac`. The CLI authority is the signed executable at
`$HOME/Library/Application Support/nils-cli/macos-agent/stable/peekaboo`.
Daemon and process runtimes use this CLI. App runtime uses the stable GUI
app's Bridge. Cached version directories are for verification and recovery;
they are not runtime launch locations.

Screen Recording and Accessibility grants belong to the effective app or CLI
authority. The GUI grant does not prove that a daemon's CLI grant is ready.
macOS tracks signed code using its designated requirement, including signing
identifier and signer. See Apple's
[code identity reference](https://developer.apple.com/documentation/technotes/tn3127-inside-code-signing-requirements).
Official v4.4.0 and v4.6.0 assets use team `FWJYW4S8P8`; the app identifier is
unchanged. The strict lock and installed signatures still need verification.
Do not re-sign assets, edit TCC databases, reset grants, or disable trust gates.

## Establish the accepted baseline

Build the exact reviewed candidate adapter commit in an isolated checkout:

```bash
cargo build --locked --release -p nils-macos-agent
candidate_target_dir="$(cargo metadata --locked --no-deps --format-version 1 | jq -er '.target_directory')"
candidate_bin="$candidate_target_dir/release/macos-agent"
"$candidate_bin" backend status --format json
"$candidate_bin" backend verify --strict --format json
"$candidate_bin" doctor --strict --format json
```

The candidate lock authorizes the exact v4.4.0 predecessor. Before installing
v4.6.0, confirm that the current receipt is still v4.4.0. `backend verify`
establishes the stable CLI from that authenticated receipt when migrating a
version-specific CLI layout, and restarts one owned app. It does not
install the candidate. Changed stable files are refused.

Inspect the designated requirements at the two stable paths with
`codesign --display -r -`. Keep this output in private trial evidence. The
maintainer confirms any initial canonical-path grants in System Settings >
Privacy & Security for the named authority, restarts it, then repeats strict
doctor. Require a ready accepted baseline before beginning the upgrade.
Also probe the CLI authority before candidate installation, using the
authenticated stable executable:

```bash
stable_cli="$HOME/Library/Application Support/nils-cli/macos-agent/stable/peekaboo"
"$stable_cli" permissions status --no-remote --json
```

Confirm the CLI's initial canonical-path grants if needed and repeat this
probe. GUI doctor readiness alone does not establish CLI permission readiness.

## Upgrade and resident acceptance

```bash
"$candidate_bin" backend install --dry-run --strict --format json
"$candidate_bin" backend install --strict --format json
"$candidate_bin" backend verify --strict --format json
"$candidate_bin" doctor --strict --format json
"$candidate_bin" capabilities --strict --format json
```

The resident tester checks the following before the maintainer selects the
candidate or merges its lock update:

- With unchanged signing identity and stable paths, no new permission prompt
  appears during upgrade, restart, or interaction, and strict doctor is ready.
- Exactly one owned backend GUI app remains after install and verify.
- Observe, click, type, key, and screenshot work through the selected runtime.
- Three independent Calculator reset/result trials pass.
- Explicitly recheck `BRIDGE_UNAVAILABLE` for app runtime and `CAPTURE_FAILED`
  for daemon runtime. Use `exec --runtime app` or `exec --runtime daemon`;
  `doctor` and `capabilities` have no `--runtime` option.
- Test rollback with the authenticated predecessor; permission readiness and
  the single-app invariant survive the rollback too.

Fixture tests cannot establish native signing assessment, TCC continuity, or
real interaction. Keep failed candidates unselected and retain the accepted
predecessor. Record the exact adapter commit, active receipt, missing service
and authority, and actionable failure in private acceptance evidence.

## Permission recovery and rollback

When doctor names a missing or pending Screen Recording or Accessibility
grant, the maintainer inspects the matching app or CLI entry in System
Settings > Privacy & Security. Confirm that exact authority, restart it using
`backend verify --strict`, and repeat doctor. A failed permission probe means
the grant could not be assessed; it does not prove a locked GUI session. If
Bridge or capture still fails with grants ready, retain the original upstream
failure and diagnose it separately instead of resetting privacy settings.

```bash
"$candidate_bin" backend rollback --dry-run --strict --format json
"$candidate_bin" backend rollback --strict --format json
"$candidate_bin" backend verify --strict --format json
"$candidate_bin" doctor --strict --format json
"$candidate_bin" backend status --format json
```

Rollback requires the complete reviewed predecessor tuple and verified cached
assets. An unowned app, failed quit, changed executable, or failed strict
assessment stops the operation. The maintainer quits an unowned instance
manually; the adapter does not terminate unrelated applications.

## Bounded cache cleanup

After acceptance or rollback, inspect and execute the same bounded plan:

```bash
"$candidate_bin" backend prune --dry-run --strict --format json
"$candidate_bin" backend prune --strict --format json
```

Each invocation removes at most 32 inactive cached versions and keeps the
authenticated current and previous receipts. Repeat the dry run if more
inactive entries remain. A pending activation, invalid receipt, symlinked
entry, or executable still in use blocks deletion; recover or quit the named
backend first. Prune does not delete stable runtime paths or receipt files.
