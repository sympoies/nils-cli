## 2026-11-06 - Define explicit owner prompt intent before implementation
<!-- devlog-id: 2026-11-06-issue-3114-owner-prompt-intent-design -->

### Result

- Recorded a proposed explicit text/command request contract, typed refusals,
  capability-selected caller migration and synthetic HTTP acceptance plan.

### Why / context

- Provider argument wording cannot reliably distinguish prose from commands. A
  caller choice removes that inference while refusing literal command-shaped
  text that existing transports cannot safely preserve.
- Independently deployed consumers require the approved bounded cross-host
  protocol exception; exact release/date fences remain implementation gates.
  Legacy unsent outbox rows become text, never inferred command intent.

### Evidence

- Inspected the owner prompt HTTP handler, command classifier, guarded native
  transport, receipt fingerprints and existing HTTP fixtures; no provider or
  shared-device probes.

### Links

- <https://github.com/sympoies/nils-cli/issues/3114>
