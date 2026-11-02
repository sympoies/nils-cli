## 2026-11-02 - Name the failed step in a composer-unobservable refusal
<!-- devlog-id: 2026-11-02-issue-3201-composer-unobservable-stage -->

### Result

- A direct reply refused with composer-unobservable now says which observation
  step failed: capture-failed, pane-unparsable, composer-not-located or
  row-unrecognized. The code, status and refusal behavior are unchanged.
- The word is in error.details.observation for the unkeyed route and in the
  refused receipt for the keyed route. It is a fixed token and never carries
  pane text, paths or raw output.

### Why / context

- Refs 3201 and 3186: one refusal code covered four different causes, so a
  reported refusal could not be told apart without the pane. The reader already
  knew which step failed and discarded it.

### Evidence

- RED before the change: the new test failed because the refusal details had no
  observation. GREEN after: the direct prompt and composer classification tests
  pass.

### Follow-ups

- The mailbox terminal notice refusal recipient-composer-unobservable and the
  guarded send code send-composer-unobservable do not carry the word yet. A
  missing word does not change their meaning. This does not change the
  classifier, so the original report of 3201 still waits for a raw capture.
