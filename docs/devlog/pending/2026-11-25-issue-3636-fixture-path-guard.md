## 2026-11-25 - Guard test fixtures against long temp and checkout paths
<!-- devlog-id: 2026-11-25-issue-3636-fixture-path-guard -->

### Result

- The shared short-socket helper now creates its directory 0700 regardless of
  umask, and a guard test binds a socket from a long temp root, a /repo working
  directory and an open umask.
- The receipt cancellation test rejects the fixture directory name alone, and
  the upload-ownership stress fixture no longer scales with the process
  temp-root length.

### Why / context

- Fixtures that embed temp or checkout paths passed on short roots but failed
  or leaked on hosts with long ones.

### Evidence

- Under a 290-character TMPDIR the helper directory was 0777 and the 3,000-path
  transcript row failed to project; both pass after the change, and the rest of
  the workspace suite passed under the same root.

### Links

- <https://github.com/sympoies/nils-cli/issues/3636>
