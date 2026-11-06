## 2026-11-06 - Disable repository executables during pinned HTTPS Git transport
<!-- devlog-id: 2026-11-06-issue-3342-pinned-https-children -->

### Result

- Pinned remote operations disable repository hooks and fsmonitor through
  command-scope Git configuration; the identity-policy spec describes token
  environment inheritance precisely.

### Why / context

- The verified token must reach the scoped credential helper, but repository
  hooks and fsmonitor must not run with that token. Stored Git configuration
  and commit authoring remain unchanged.

### Evidence

- Before the fix, the fake-token local-bare fixture completed push/fetch and
  then failed because both the pre-push hook and fsmonitor observed the token.
  The same regression protects the fixed isolation behavior.

### Links

- <https://github.com/sympoies/nils-cli/issues/3342>
