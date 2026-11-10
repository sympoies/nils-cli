## 2026-11-11 - Distinguish GitHub identity rate limits
<!-- devlog-id: 2026-11-11-distinguish-github-identity-rate-limits-18e6bce82eedc9c0-85b0a -->

### Result

- Identity verification reports GitHub rate limits with a retry time and a
  distinct exit code.

### Why / context

- Rate exhaustion should not be mistaken for invalid credentials.

### Evidence

- Backend identity response fixtures cover rate limits and unauthorized
  responses.

### Links

- <https://github.com/sympoies/nils-cli/issues/3539>
