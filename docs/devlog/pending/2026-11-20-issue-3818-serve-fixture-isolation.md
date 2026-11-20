## 2026-11-20 - Complete Agent Session serve fixture isolation
<!-- devlog-id: 2026-11-20-issue-3818-serve-fixture-isolation -->

### Result

- Route the remaining serve fixtures through the shared guard with the exact
  owning temporary directory.

### Why / context

- Admission serve launches inherited the parent environment. Other fixtures
  lacked a common assertion for resolved roots and the tmux socket.

### Evidence

- The admission restart probe observed a synthetic inherited variable in two
  daemon launches before the change and zero after it.

### Links

- <https://github.com/sympoies/nils-cli/issues/3818>
