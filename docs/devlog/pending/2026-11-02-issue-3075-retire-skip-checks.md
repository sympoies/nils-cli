## 2026-11-02 - Retire the prepare-release-bump skip-checks flag
<!-- devlog-id: 2026-11-02-issue-3075-retire-skip-checks -->

### Result

- prepare-release-bump.sh now rejects --skip-checks as an unknown argument; the
  no-op alias, its notice, its usage line, its runbook sentence and its
  own-compat allowlist row are removed.

### Why / context

- The flag only mirrored the default locked check, so keeping it silently
  accepted a retired form; the tests now assert rejection beside the other
  retired release flags.

### Evidence

- scripts/ci/tests/prepare-release-bump.test.sh failed on the rejection case
  before the change and passes after; own-compat audit and its self-test pass
  with one fewer row.

### Links

- <https://github.com/sympoies/nils-cli/issues/3075>
