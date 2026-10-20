## 2026-10-20 - Keep gate reports outside Cargo targets
<!-- devlog-id: 2026-10-20-gate-report-routing -->

### Result

- Gate nextest reports and history use configurable owner artifact directories;
  CI consumers read the same external JUnit path.

### Why / context

- Test evidence must survive independently of build caches and must not
  populate the default target during isolated gate runs.

### Evidence

- Regression failed for external report output and CI consumers before the fix;
  routed nextest smoke test and report routing fixtures pass.

### Links

- <https://github.com/sympoies/nils-cli/issues/2401>
