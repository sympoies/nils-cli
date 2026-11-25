## 2026-11-25 - Drain agent-session test writers before fixture cleanup
<!-- devlog-id: 2026-11-25-issue-4026-session-test-teardown -->

### Result

- Keep direct Codex control test roots alive until the runtime drains retirement work.
  Use a write-free tmux stub for provider usage fixtures.

### Why / context

- A control task can finish before its spawned blocking retirement. Killing a
  serve fixture also leaves its incidental fake tmux helper free to finish
  logging.

### Evidence

- On main, the unmodified tempdir probe found reconnect.lock after all 750
  selected test executions passed.

### Links

- <https://github.com/sympoies/nils-cli/issues/4026>
