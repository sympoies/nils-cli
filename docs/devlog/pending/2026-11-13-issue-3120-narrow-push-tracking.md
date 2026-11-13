## 2026-11-13 - Resolve narrow-clone tracking after branch publication
<!-- devlog-id: 2026-11-13-issue-3120-narrow-push-tracking -->

### Result

- Publishing a branch from a single-branch clone adds an exact fetch mapping
  and materializes its remote-tracking ref when needed.

### Why / context

- Git push can configure an upstream without creating a resolvable
  remote-tracking ref when the clone fetches only the default branch; the
  existing PR head guard then rejects the published commit.

### Evidence

- The retained regression failed after successful publication at upstream
  resolution; all 29 publish/sync integration tests pass with the fix.

### Links

- <https://github.com/sympoies/nils-cli/issues/3120>
