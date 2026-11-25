## 2026-11-24 - Guard Codex destination moves with a no-writer abort
<!-- devlog-id: 2026-11-24-agent-session-cross-host-resume-codex-destination -->

### Result

- Added read-only destination checks, exact staged history validation, and
  durable admission and abort outcomes. Runtime readiness is checked before
  writer admission, so an unready destination refuses and stays abortable. The
  production move selector remains gated during the Claude slice.

### Why / context

- The source must be able to distinguish a destination abort before writer
  admission from an unresolved launch.

### Evidence

- All thirty-nine focused resume tests passed, including the destination
  checkout and writer checks and the admission-versus-abort race.

### Links

- <https://github.com/sympoies/nils-cli/issues/3981>
