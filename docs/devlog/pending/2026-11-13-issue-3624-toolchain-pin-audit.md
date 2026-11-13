## 2026-11-13 - Toolchain pin audit and layout-aware target guard
<!-- devlog-id: 2026-11-13-issue-3624-toolchain-pin-audit -->

### Result

- CI now fails when a workflow, setup step or helper names a Rust toolchain
  other than rust-toolchain.toml or a floating one, and when a job runs cargo
  without an explicit pinned setup step. The two workflows that lacked a setup
  step gained one. The shared-target guard scans test executables under both
  the deps/ and the build/<pkg>/<hash>/out/ layouts. The bump procedure is
  documented in the workspace maintenance reference.

### Why / context

- A floating stable toolchain moved from 1.99.1 to 1.100.0 under CI without a
  repository change and broke macOS clippy and sibling-binary lookups; the pin
  stays on 1.99.1 until a reviewed bump PR.

### Evidence

- toolchain-pin-audit tests fail against a pass-everything baseline for each
  floating form and pass on the current tree; the audit failed the current tree
  on the two workflows without setup before they were fixed; target-guard tests
  fail without the new layout scan.

### Links

- <https://github.com/sympoies/nils-cli/issues/3624>
