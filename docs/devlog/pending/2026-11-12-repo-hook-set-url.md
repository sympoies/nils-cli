## 2026-11-12 - forge-cli: list repository webhooks and switch their URL safely
<!-- devlog-id: 2026-11-12-repo-hook-set-url -->

### Result

- Added `repo hook` and `org hook` (list, set-url, set-active, batch) and
  `org hook create` for GitHub webhooks; every write is a compare-and-swap on
  the current URL or active flag, sends only the changed field, and is read
  back; refs #3613
- URL changes use the hook configuration endpoint with only `url`; the active
  flag is sent without `config`, and the read-back detects a dropped secret
- `org hook create` reads the secret only from `--secret-env` and sends it in a
  private request-body file; a hook already delivering to the URL is refused
- Organization calls authorize through a new organization identity target that
  only an `org` rule can match; a missing `admin:org_hook` scope gets a typed
  error

### Why / context

- Moving webhook receivers meant editing each repository hook by hand, and raw
  `gh api` writes are blocked for agents

### Evidence

- Stub-provider integration tests cover redaction, field-only writes, refusal
  on mismatch, dry-run, batch per-row reporting, a single actor probe per
  batch, secret handling for create, and the org identity rule
