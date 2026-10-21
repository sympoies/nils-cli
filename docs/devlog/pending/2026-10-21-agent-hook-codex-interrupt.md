## 2026-10-21 - agent-hook: accept the Codex Interrupt hook event
<!-- devlog-id: 2026-10-21-agent-hook-codex-interrupt -->

### Result

- agent-hook accepts Codex `Interrupt` in policy and dispatch and normalizes
  its activity to an observed `stop_observed` turn event, so a later turn start
  closes the interrupted turn through the existing path.
- Interrupt requires `session_id` and `turn_id` and fails closed with the
  existing typed errors; it admits only allow, activity, and runtime-kit
  handler capabilities, and a closed block renders as `systemMessage`.

### Why / context

- The upstream Interrupt output schema admits only `systemMessage`, and the
  managed hook policy could not register the event for admitted consumers.

### Evidence

- Schema pinned to openai/codex rust-v0.162.1 (identical at rust-v0.150.0):
  codex-rs/hooks/schema/generated/interrupt.command.input.schema.json.
- cargo test -p nils-agent-hook --test codex_interrupt failed 6 of 7 before the
  change and passes 7 of 7 after.

### Links

- Refs #2695
