## 2026-11-27 - Retain the first Console attestation refusal
<!-- devlog-id: 2026-11-27-issue-4154-console-receipts -->

### Result

- Return the first verified attested start refusal on later reconciliation,
  including after a daemon restart.

### Why / context

- A refusal during a retry cannot resolve an earlier dispatch. Keep the receipt
  ambiguous when another attempt has started.

### Evidence

- The loopback regression failed on the baseline, then passed in the 90-test
  scoped suite. Coverage also checks delegated lookup denial and ordinary
  duplicate JSON rejection.

### Links

- <https://github.com/sympoies/nils-cli/issues/4154>
