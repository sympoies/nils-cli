## 2026-11-06 - Suppress Git hooks, fsmonitor, and alternate-refs callbacks during pinned HTTPS
<!-- devlog-id: 2026-11-06-issue-3342-pinned-https-children -->

### Result

- Pinned remote operations disable repository hooks, fsmonitor, and
  alternate-refs commands through command-scope Git configuration. The spec
  names that closed list and the residual operator-owned executable paths.

### Why / context

- The verified token must reach the scoped credential helper, but repository
  hooks, fsmonitor, and alternate-refs commands must not run with that token.
  Stored Git configuration and commit authoring remain unchanged.

### Evidence

- Before the fix, the fake-token local-bare fixture completed push/fetch and
  then failed because both the pre-push hook and fsmonitor observed the token.
  The security-review repair reproduced the alternate-refs exposure on the
  reviewed head before adding its override. All 36 focused identity tests then
  passed, including all three canaries and local transport success.

### Links

- <https://github.com/sympoies/nils-cli/issues/3342>
