## 2026-11-05 - Explain unacquired hosted macOS jobs at CI gates
<!-- devlog-id: 2026-11-05-issue-3332-macos-capacity -->

### Result

- CI gates identify the proven hosted runner acquisition failure without
  admitting unexecuted macOS validation.

### Why / context

- Provider cancellations can represent missing capacity rather than executed
  tests; timing and cancellation alone are insufficient. Apple Silicon
  execution and strict gate outcomes are preserved instead of replacing
  architecture coverage or rerunning after queue rejection.

### Evidence

- Runs 41861170500 and 41871190427 show zero steps, no assigned runner and the
  explicit acquisition failure annotation; the pre-fix regression fails because
  the gate omits that reason.

### Links

- <https://github.com/sympoies/nils-cli/issues/3332>
