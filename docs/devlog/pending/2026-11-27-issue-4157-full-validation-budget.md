## 2026-11-27 - Budget Linux full validation from hosted timings
<!-- devlog-id: 2026-11-27-issue-4157-full-validation-budget -->

### Result

- Set the Linux full-validation job backstop to 56 minutes without changing
  per-test limits or other jobs.

### Why / context

- A release merge-group run spent 23m47s in setup and build. Adding the
  previous release validation tail of 21m43s and 10 minutes of headroom gives
  55m30s, rounded up.

### Evidence

- GitHub Actions full-validation Linux job logs and step timings from runs
  45809621111 and 45961417194.

### Links

- <https://github.com/sympoies/nils-cli/issues/4157>
