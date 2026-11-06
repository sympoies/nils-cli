## 2026-11-06 - Refuse repository proxy and CA overrides for pinned HTTPS
<!-- devlog-id: 2026-11-06-issue-3341-pinned-https-network -->

### Result

- Governed remote commands refuse repository/worktree proxy and CA entries,
  including URL-specific forms, while retaining system/global values.

### Why / context

- Credential pinning must not let repository configuration select the
  authenticated request proxy or CA trust store. The guard reuses helper-scope
  inspection and keeps the refusal list closed.

### Evidence

- Verified RED on the pre-fix baseline: each of the three local settings stayed
  effective in the pinned child environment and local bare push succeeded; the
  system/global preservation fixture passed.

### Links

- <https://github.com/sympoies/nils-cli/issues/3341>
