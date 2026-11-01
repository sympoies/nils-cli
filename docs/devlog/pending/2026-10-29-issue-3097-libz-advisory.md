## 2026-10-29 - Patch libz-sys advisory blocking dependency validation
<!-- devlog-id: 2026-10-29-issue-3097-libz-advisory -->

### Result

- Updated only libz-sys from 1.1.29 to the smallest patched release 1.1.31 and
  regenerated dependency attribution inventories.

### Why / context

- RUSTSEC-2026-0418 blocked the required dependency gate; existing parent
  constraints allow the patch without source or policy changes.

### Evidence

- The existing cargo-deny advisories/bans command failed on the original
  lockfile and passed after the update using the same current RustSec database
  containing the advisory. Strict third-party inventory audit passed.

### Links

- <https://github.com/sympoies/nils-cli/issues/3097>
