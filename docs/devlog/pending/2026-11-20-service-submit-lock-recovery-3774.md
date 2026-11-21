## 2026-11-20 - Preserve retryable service submission lock failures
<!-- devlog-id: 2026-11-20-service-submit-lock-recovery-3774 -->

### Result

- Service-send reports coordination-unavailable with same-key recovery when a
  coordination lock times out. The daemon warns on the fallback and relay
  admission refusal using a fixed cause and a service fingerprint.

### Why / context

- The generic operator error hid a reproducible transient lock timeout and
  service refusal diagnostics were absent from the default daemon output.

### Evidence

- The synthetic service mailbox fixture holds a real registry lock, retries and
  verifies one stored copy, then checks diagnostic fields and redaction for a
  bad recipient record and relay HTTP 403.

### Links

- <https://github.com/sympoies/nils-cli/issues/3774>
