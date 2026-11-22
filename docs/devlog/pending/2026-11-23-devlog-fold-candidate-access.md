## 2026-11-23 - Read release-fold queue policy through effective branch rules
<!-- devlog-id: 2026-11-23-devlog-fold-candidate-access -->

### Result

- Release fold admission uses the workflow read client's effective branch rules
  for squash queue policy and reports sanitized GraphQL denial paths.

### Why / context

- An authorization error in the initial candidate read blocked the first
  admission step without naming the denied field.

### Evidence

- Controlled fold fixtures cover a denied queue-policy read, a non-squash
  branch rule, and sanitized candidate errors.

### Links

- <https://github.com/sympoies/nils-cli/issues/3879>
