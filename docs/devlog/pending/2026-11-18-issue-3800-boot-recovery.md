## 2026-11-18 - Recover marked sessions after planned reboot
<!-- devlog-id: 2026-11-18-issue-3800-boot-recovery -->

### Result

- Add an explicit pre-shutdown mark and one-shot daemon recovery with
  per-session receipts and opt-outs.

### Why / context

- Unmarked or ambiguous runtime state must not revive stopped sessions.

### Evidence

- Synthetic prior-boot and private tmux integration fixtures.

### Links

- <https://github.com/sympoies/nils-cli/issues/3800>
