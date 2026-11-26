## 2026-11-26 - Separate Rust action and toolchain pins
<!-- devlog-id: 2026-11-26-separate-rust-action-and-toolchain-pins-18eba324bc350c63-3e2c6b -->

### Result

- CI pins the action implementation independently and explicitly selects the
  workspace Rust version.

### Why / context

- Numbered upstream action refs select a Rust release and Dependabot can
  propose an unavailable toolchain.

### Evidence

- Dependabot PR 4087 failed toolchain setup; toolchain pin audit fixtures cover
  the explicit input.

### Links

- <https://github.com/sympoies/nils-cli/issues/4094>
