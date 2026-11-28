## 2026-11-28 - Wait a bounded time for the source lock before refusing a resume as live
<!-- devlog-id: 2026-11-28-resume-source-lock-wait -->

### Result

- Resume admission waits up to 250 ms for a held source session lock before it
  refuses with resume-source-live.

### Why / context

- Serve housekeeping and boot recovery take the same record lock for short
  sections, so a resume that arrived during one was refused as live with
  safe_to_retry false.

### Evidence

- A holder released after 50 ms was refused before the change and is acquired
  after it; a holder kept past the wait still refuses with resume-source-live.
  The console resume integration test passed 20 of 20 under 8 nice 19 hogs.

### Links

- <https://github.com/sympoies/nils-cli/issues/4206>
