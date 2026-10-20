## 2026-10-21 - Verify immediate native replies during active turns
<!-- devlog-id: 2026-10-21-native-busy-client-replies -->

### Result

- Added controlled HTTP and native-protocol coverage plus client integration
  guidance for replies delivered while Codex and Claude remain working.

### Why / context

- Ordinary replies already use immediate provider handoff when require_idle is
  false; preserving the explicit idle-only option avoids changing intentional
  guarded callers or adding a daemon queue.

### Evidence

- cargo test --locked -p nils-agent-session --lib native_busy_reply --
  --nocapture: two tests passed on unchanged production code. Protocol and
  terminal fixtures cover unknown preservation, duplicate suppression, distinct
  later input, drafts and runtime fences; they are not live provider
  acceptance.

### Links

- <https://github.com/sympoies/nils-cli/issues/2581>

### Follow-ups

- Live provider acceptance and request-owned receipt recovery remain separately
  verified; see issue #2710.
