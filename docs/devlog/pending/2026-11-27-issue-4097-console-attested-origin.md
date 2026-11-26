## 2026-11-27 - Verify Console attested creates at the origin
<!-- devlog-id: 2026-11-27-issue-4097-console-attested-origin -->

### Result

- Accept managed attestation through the Console start API after authenticated
  capability discovery on the configured relay.
- Require the requested session ID and exact attested steward launch context
  before reporting creation.

### Why / context

- The origin must prevent an older relay from turning an explicit request into
  ordinary creation. An exact retry uses the same relay operation identity and
  launch inputs.

### Evidence

- Six loopback contract fixtures failed on the baseline and passed after
  implementation.

### Links

- <https://github.com/sympoies/nils-cli/issues/4097>
