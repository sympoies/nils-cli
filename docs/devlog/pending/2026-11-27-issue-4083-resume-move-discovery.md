## 2026-11-27 - Harden Console resume-move destination discovery
<!-- devlog-id: 2026-11-27-issue-4083-resume-move-discovery -->

### Result

- Send Cache-Control: no-store for GET /sessions and run the native capability
  probe in the existing blocking list collection.

### Why / context

- Session listings vary by credential. The probe checks verifier paths through
  synchronous filesystem calls. The v1.1 relay grant remains the sole owner
  authority.

### Evidence

- The loopback cache-control test failed on the baseline and passed with the
  fix. All six Console destination tests passed.

### Links

- <https://github.com/sympoies/nils-cli/issues/4083>
