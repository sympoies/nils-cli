## 2026-11-10 - Order lifecycle exits after session creation
<!-- devlog-id: 2026-11-10-issue-3031-lifecycle-creation-order -->

### Result

- Startup disappearance probes no longer consume the exit marker before
  creation; explicit early exits are retained until creation is recorded.

### Why / context

- The runtime can be temporarily invisible while the creation response is still
  in flight, so an observation during that interval is not reliable evidence of
  termination.

### Evidence

- Lifecycle fixture regression reproduced the premature observation and marker
  suppression on the pre-fix revision.

### Links

- <https://github.com/sympoies/nils-cli/issues/3031>
