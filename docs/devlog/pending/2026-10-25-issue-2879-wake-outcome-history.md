## 2026-10-25 - Retain bounded wake outcome history
<!-- devlog-id: 2026-10-25-issue-2879-wake-outcome-history -->

### Result

- Broker status retains up to eight content-free wake outcomes across
  notification generations, with a sticky v5 registry fence and compatible
  current projection readers.

### Why / context

- A later successful wake previously replaced the only visible deferral reason.
  Older registry writers must refuse before discarding newly persisted history;
  retained history does not reconstruct past incidents or prove authentication
  recovery.

### Evidence

- The retained regression evidence verifies with the released evidence tool.
  The current-main integration preserves the diagnostic patch byte for byte;
  strict file-size and diff checks pass.

### Links

- <https://github.com/sympoies/nils-cli/issues/2879>

### Follow-ups

- Controlled authentication-recovery acceptance remains outstanding; the
  historical deferral cause is unknown.
