## 2026-11-12 - secrets set stores values bare
<!-- devlog-id: 2026-11-12-issue-3544-secrets-set-bare-values -->

### Result

- secrets set now stores the stdin value bare as KEY=value (LF as the SOPS
  dotenv newline form); a value with a literal backslash followed by n is
  refused. secrets add already stored file bytes verbatim, so both share one
  contract.

### Why / context

- Quoted storage leaked the wrapping quotes to plain sops dotenv readers. SOPS
  dotenv keeps quotes, hash, dollar, backslash and edge whitespace as written,
  so the quoted form was unnecessary. No fallback reader for the old form:
  existing entries are migrated by re-running secrets set per key.

### Evidence

- Pre-edit RED setter tests; real SOPS 3.13 dotenv round-trip probe with a
  disposable key; focused secrets crate tests.

### Links

- <https://github.com/sympoies/nils-cli/issues/3544>
