## 2026-11-15 - Keep local-fast sccache startup paths short
<!-- devlog-id: 2026-11-15-keep-local-fast-sccache-startup-paths-short-18e83dc554922cd2-17f126 -->

### Result

- Local-fast warms sccache with a stable private temporary directory and nested
  build-info fixtures ignore ancestor compiler wrappers.

### Why / context

- Sccache startup uses TMPDIR for its notification socket, while Cargo also
  discovers wrapper settings in fixture ancestors.

### Evidence

- The long-root linked-worktree reproduction failed before isolation; the
  focused fixture and cold-start tests passed afterward.

### Links

- <https://github.com/sympoies/nils-cli/issues/3717>
