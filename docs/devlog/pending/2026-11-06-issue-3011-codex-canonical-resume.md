## 2026-11-06 - Admit canonical Codex new-record resume
<!-- devlog-id: 2026-11-06-issue-3011-codex-canonical-resume -->

### Result

- Codex start --resume reuses the shared provider-resume path with a new
  managed ID, exact conversation and original broker account. Read-only
  preflight retains the exact v1 envelope without runtime or credential
  preparation.

### Why / context

- Canonical admission must guard every prior writer and preserve ordinary
  account binding and bootstrap delivery instead of exposing raw resume
  arguments or a fresh-start fallback.

### Evidence

- Four Codex CLI fixture groups reproduced resume-unsupported before production
  changes. The focused integration resume suite passes 27 tests with synthetic
  providers and brokers.

### Links

- <https://github.com/sympoies/nils-cli/issues/3011>
