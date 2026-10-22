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
Official v4.4.0, v4.8.0, and v4.9.6 assets use team `FWJYW4S8P8`; the app identifier is
unchanged. The strict lock and installed signatures still need verification.
The v4.9.6 CLI requires the archive's `libswiftCompatibilitySpan.dylib` beside
the executable. The lock binds its exact digest, signing authority/team and all
locked architectures (arm64 and x86_64). Install, verification, execution and rollback assess the
complete CLI runtime; copying the executable alone is insufficient.
Do not re-sign assets, edit TCC databases, reset grants, or disable trust gates.

## v4.9.6 review and typing expectations

The [official release](https://github.com/openclaw/Peekaboo/releases/tag/v4.9.6)
was published on 2026-10-20. Archive digests and signing metadata come from
its `ASSET-INVENTORY.json` and `SIGNING-MANIFEST.json`; executable and sidecar
digests are computed from the downloaded archives. Both app and CLI now contain
arm64 and x86_64 slices. The sidecar no longer contains an arm64e slice.
Native signature, notarization, permission continuity, and interaction checks
remain resident acceptance responsibilities.

The [v4.8.0 to v4.9.6 changelog](https://github.com/openclaw/Peekaboo/blob/v4.9.6/CHANGELOG.md)
contains relevant improvements:

- v4.9.0 fixes signed typing receipts for already-empty clears and already-satisfied
  caret moves, aligns native/Bridge literal replacement eligibility, and updates
  AXorcist modifier handling. Foreground file dialogs retain parent-window ownership.
- v4.9.0 preserves accepted dispatch evidence after unchanged readback and keeps
  response-loss/pending-operation outcomes indeterminate and unsafe to retry.
- v4.9.4–v4.9.5 improve granted app-host selection and declare route-specific
  typing/paste/key permission needs.

These changes may improve typing reliability and receipt accuracy, but the
changelog does not claim a general fix for stale focused-element refusals or
foreground typing that lands while returning indeterminate/exit 70. Do not treat
this upgrade as proof that either symptom is resolved. Existing stale-focus
and uncertain-mutation guards remain in force pending native acceptance.

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

The candidate lock authorizes the exact v4.8.0 predecessor and retains the
v4.4.0 rollback tuple. Before installing v4.9.6, confirm that the current receipt
is still the intended authenticated predecessor. `backend verify`
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

Maintenance follows the latest stable upstream release through an exact-lock
PR. The maintainer schedules native acceptance after release; merging a lock
update does not establish native interaction acceptance. The resident tester
checks the following before accepting the installed candidate:

- With unchanged signing identity and stable paths, no new permission prompt
  appears during upgrade, restart, or interaction, and strict doctor is ready.
- Exactly one owned backend GUI app remains after install and verify.
- Observe, click, type, key, and screenshot work through the selected runtime.
- Three independent Calculator reset/result trials pass.
- Background typing with freshly observed focus and foreground typing both
  receive accurate delivery receipts. If text lands with an indeterminate
  result, observe state before recovery; never replay uncertain input.
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
the grant could not be assessed; it does not prove a locked GUI session.
A missing/pending grant can also accompany an uncertain dispatched mutation.
The supplementary diagnostic does not change its `unknown_mutation` journal
classification, upstream failure exit, original JSON, or forbidden replay.
Do not repeat that mutation to test a recovered permission; observe its state
first. If
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

An interrupted rollback resumes on the next non-dry-run `backend rollback`. A
retry that finds the rollback already committed verifies the active backend first
and keeps its recovery record if verification fails. A same-release install
settles a committed rollback, and dry runs refuse while one is unresolved.

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
backend first. The refusal names the cached version and app/CLI executable;
unavailable inspection also refuses deletion. The entire bounded plan is
inspected before any deletion, including when a later entry is busy. Prune
does not delete stable runtime paths or receipt files.
