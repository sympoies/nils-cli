## 2026-11-22 - Verify governed release heads through their protected squash
<!-- devlog-id: 2026-11-22-issue-3879-governed-release-fold-source -->

### Result

- Post-release folds verify the annotated tag, signed release head, and exact
  merged release PR, and retain the protected squash as the replay anchor.

### Why / context

- The governed release preserves the signed PR head as its tag target. The
  squash creates a different commit on protected main.

### Evidence

- A real split-history Git fixture reproduced the original publication failure.
  The updated publication check resolved the actual release and protected
  squash through read-only GitHub requests.

### Links

- <https://github.com/sympoies/nils-cli/issues/3879>
