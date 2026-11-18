## 2026-11-17 - Validate resolved executable directories during toolchain selection
<!-- devlog-id: 2026-11-17-issue-3601-hook-directory -->

### Result

- Trusted roots, generated selectors and automatic sibling-hook preflight
  reject resolved directories owned by neither caller nor root, or writable by
  group or world.

### Why / context

- Directory trust follows the resolved executable, preserving package-manager
  links through writable bin directories. Rust and generated POSIX shell retain
  runtime-local checks with one policy; no additional runtime helper is
  introduced.

### Evidence

- Retained pre-fix failures showed all paths accepting writable and
  foreign-owned directories. Focused Linux tests now pass, including a real
  foreign-owner fixture in an unprivileged user namespace and a writable-bin
  symlink into a 0755 release directory.

### Links

- <https://github.com/sympoies/nils-cli/issues/3601>
