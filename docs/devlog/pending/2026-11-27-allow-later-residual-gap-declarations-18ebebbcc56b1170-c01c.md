## 2026-11-27 - Allow later residual gap declarations
<!-- devlog-id: 2026-11-27-allow-later-residual-gap-declarations-18ebebbcc56b1170-c01c -->

### Result

- Evidence records retain gap declaration history and reflect the latest state.

### Why / context

- Later validation can uncover a gap after a no-gaps declaration.

### Evidence

- Focused regression reproduces the refusal and checks the updated delivery
  state.

### Links

- <https://github.com/sympoies/nils-cli/issues/4135>
