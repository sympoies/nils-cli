## 2026-10-24 - Resolve private test-worker aliases before trust validation
<!-- devlog-id: 2026-10-24-issue-2528-worker-path-alias -->

### Result

- The test-only worker root is canonicalized before existing executable and
  ancestor trust validation.

### Why / context

- A private copy with trusted ownership and mode can still fail non-Linux
  validation when the temporary path includes a symlink ancestor; production
  trust checks remain unchanged.

### Evidence

- Clean macOS source baseline reproduced the supervisor setup failure through a
  temporary-directory alias; the isolated alias regression changed from RED to
  GREEN and 30 focused worker/supervisor tests passed.

### Links

- <https://github.com/sympoies/nils-cli/issues/2528>
