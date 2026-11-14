## 2026-11-15 - Keep remote Claude approvals outside the hook audit fence
<!-- devlog-id: 2026-11-15-issue-3718-permission-fence -->

### Result

- Known Claude versions outside the 2.1.317 permission audit can answer a
  visible dialog with one guarded key; audited sessions remain hook-only.

### Why / context

- A narrow hook payload fence must not also disable the existing terminal
  answer path for a differently versioned external provider. Unreadable
  versions refuse retryably and are rechecked on the next attempt.

### Evidence

- Synthetic regression first refused both 2.1.316 and 2.1.318 with
  claude-permission-hook-required. Boundary, dialog and retry fixtures protect
  the changed contract.

### Links

- <https://github.com/sympoies/nils-cli/issues/3718>
