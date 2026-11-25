## 2026-11-25 - Settle verified writer-free move failures
<!-- devlog-id: 2026-11-25-settle-writer-free-moves -->

### Result

- Durable writer admission can settle to a no-writer abort after a verified
  pre-launch Claude preparation failure.

### Why / context

- A credential or pre-publication failure could leave the original source
  conversation fenced after admission.

### Evidence

- CLI fixtures cover signed recovery, refusal to replay, uncertain transcript
  publication, and ambiguous launches.

### Links

- <https://github.com/sympoies/nils-cli/issues/4042>
