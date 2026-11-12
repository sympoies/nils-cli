## 2026-11-12 - Reuse a verified forge actor within a process
<!-- devlog-id: 2026-11-12-forge-identity-actor-verification-cache -->

### Result

- Managed gh calls in one forge-cli process verify the actor once and reuse it
  for 10 minutes instead of probing before every call; a 401/403 from a managed
  call, a different credential, host, expected identity or state root, or
  expiry verifies again. Mismatches are never cached.
- The authorized audit record notes actor_verification (cached, verified_at).
  The cache is in memory only and keyed by a one-way digest; no credential is
  stored.

### Why / context

- Actor probes were about 80% of identity-token REST calls; a long pr
  wait-checks or merge poll issued dozens.

### Evidence

- Test-first record retained with the red runs; focused nils-common and
  forge-cli identity tests pass.

### Links

- <https://github.com/sympoies/nils-cli/issues/3575>
