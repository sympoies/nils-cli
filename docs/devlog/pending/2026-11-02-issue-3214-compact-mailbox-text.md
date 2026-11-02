## 2026-11-02 - Replace the compact mailbox letters with one readable total
<!-- devlog-id: 2026-11-02-issue-3214-compact-mailbox-text -->

### Result

- The plain mailbox reminder, on the typed composer route and the
  notice-gate-off fallbacks, is now one readable line: Mailbox: N unread;
  agent-session message inbox, where N is the total of live unread messages. At
  the mailbox quota it is 48 characters and still fits one input row. No
  producer writes the letter form (Mailbox: b=1 h=2) any more.
- The agent-hook recognizer accepts the new text with a canonical N from 1 to
  256 and nothing else, so a human prompt that resembles it is not a reminder.
  An unsent receipt written in the old form is rewritten to the new text when
  it is loaded, using the total of its counts; a sent attempt keeps its exact
  bytes, so an upgrade never causes a resend.

### Why / context

- Refs 3214 and 3186: the maintainer asked for the letter shorthand to be
  removed from every route. This replaces the accept-both criterion of 3180 for
  the plain route; the notice text from 3180 is unchanged.

### Evidence

- RED before the change: the composer-route, gate-off hook claim, quota-width,
  unsent-receipt and history-reader tests failed in agent-session, and the
  recognizer test and the managed hook step test failed in agent-hook, all
  because the new text did not exist. GREEN after: those tests pass.

### Follow-ups

- The one named exception is history: a reminder already in a provider
  transcript in the old form is still recognized, since provider history is
  immutable. Hosts replace the session and hook binaries together and restart
  the service; a wake can be missed during the swap. A private consumer must
  accept the new text before release, and a native check per route after
  release is still needed.
