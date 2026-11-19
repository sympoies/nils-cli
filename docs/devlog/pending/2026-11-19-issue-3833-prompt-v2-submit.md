## 2026-11-19 - Keep the Claude hook fence through structured-prompt Enter
<!-- devlog-id: 2026-11-19-issue-3833-prompt-v2-submit -->

### Result

- Structured Claude prompts hold the exclusive hook-ingress fence from before
  the paste through the Enter, so a hook writer that appears after the paste
  waits instead of refusing the Enter and leaving the text in the composer.
- A composer showing an image label followed by the prompt's remaining text is
  accepted for a prompt that starts with an existing image path; other composer
  content is still refused.
- A prompt pasted but not submitted is recorded as a content-free observation
  that agent-session diagnose shows.

### Why / context

- The Enter admission probed the ingress fence without waiting, while hook
  writers hold it shared and then wait for the record lock the delivery owns.

### Evidence

- Synthetic fake-tmux regression tests failed before the change: Enter refused
  with send-provider-event-pending after the paste, image label plus text
  refused, and no observation recorded.

### Links

- <https://github.com/sympoies/nils-cli/issues/3833>
