## 2026-11-26 - Run the isolation guard under a canonical temp root
<!-- devlog-id: 2026-11-26-env-guard-symlinked-temp-root -->

### Result

- The env-isolation guard resolves symlinks in the caller temp root before
  handing it to the tests, so --local-fast passes on hosts whose temp root is a
  symlink.
- The shared-daemon exclusion fixture canonicalizes its excluded root like its
  allowed roots.

### Why / context

- Production code refuses symlinked state paths; only the test environment
  changed.

### Evidence

- The guard self-test fails without the canonical root; the guarded agent-hook
  and agent-session selections pass under a symlinked TMPDIR.

### Links

- <https://github.com/sympoies/nils-cli/issues/4073>
