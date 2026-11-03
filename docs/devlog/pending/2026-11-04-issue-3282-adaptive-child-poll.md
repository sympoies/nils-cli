## 2026-11-04 - macos-agent polls child exit adaptively
<!-- devlog-id: 2026-11-04-issue-3282-adaptive-child-poll -->

### Result

- Child-process wait loops in macos-agent now sleep 1, 2, 4, 8, then 10 ms
  through one shared helper instead of a fixed 10 ms. Deadlines, kill paths,
  and errors are unchanged.

### Why / context

- The backend integration tests spawn about a thousand fake tool children each,
  and every spawn paid at least one fixed sleep. Sharing staged fixtures and
  removing stub grep spawns were measured first and gave no gain.

### Evidence

- Summed time of the 51 backend::* integration tests on Linux through the
  nextest local-fast gate: 156.0 s before, 56.0 s after. The macOS figure is
  not measured on PRs and comes from a post-merge full run.

### Links

- <https://github.com/sympoies/nils-cli/issues/3282>
