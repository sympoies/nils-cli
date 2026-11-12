## 2026-11-10 - Project stopped Codex runtimes when turn completion is missing
<!-- devlog-id: 2026-11-10-issue-3521-codex-runtime-stopped -->

### Result

- Codex activity exposes unknown with codex_runtime_stopped after an exit
  observation matches the runtime incarnation and generation.

### Why / context

- Plain provider launches can end a usage-limited turn without a completion
  notification; runtime exit proves inactivity but cannot prove successful
  completion.

### Evidence

- The retained baseline regression failed with Working versus Unknown; the
  stopped-runtime projection passed the focused regression.

### Links

- <https://github.com/sympoies/nils-cli/issues/3521>
