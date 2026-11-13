## 2026-11-13 - Observe executing gate cgroup limits
<!-- devlog-id: 2026-11-13-issue-2725-effective-gate-limits -->

### Result

- Gate resource receipts distinguish requested caps from executing-cgroup and
  constraining ancestor memory/swap observations, with explicit mismatch and
  unavailable reasons.

### Why / context

- A requested systemd property does not prove the budget enforced on the
  workload; readback must occur inside its scope after slice configuration.
  Observation is limited to visible cgroup v2 ancestors and does not
  alter admission or impose another cap.

### Evidence

- Synthetic executing-cgroup fixture suite passed; the pre-fix receipt
  assertion failed because effective fields were absent.

### Links

- <https://github.com/sympoies/nils-cli/issues/2725>
