## 2026-10-24 - Bound proxy transport fixture socket paths
<!-- devlog-id: 2026-10-24-issue-2852-proxy-socket-roots -->

### Result

- Place the proxy transport fixture in a short private temporary directory and
  retain cleanup ownership and existing proxy assertions.
- Add a subprocess regression exercising the real proxy diagnostic test under
  short and long temporary roots.

### Why / context

- The upstream Unix socket bind failed before behavior under an over-budget
  inherited temporary root; the shadow-writer controls did not reproduce the
  proposed socket failure.

### Evidence

- Test-first regression failed at UnixListener::bind with a 201-byte socket
  pathname and SUN_LEN; focused proxy tests pass after constraining the fixture
  root.

### Links

- Refs #2852 and #2894.
