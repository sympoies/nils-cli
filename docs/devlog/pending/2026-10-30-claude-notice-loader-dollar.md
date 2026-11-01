## 2026-10-30 - Claude notice module loads under the native loader
<!-- devlog-id: 2026-10-30-claude-notice-loader-dollar -->

### Result

- The embedded Claude notice hooks module no longer stores the engine handle;
  session.end uses the handle it receives, so Claude Code 2.1.296 loads the
  module.
- A source-level call-shape check and a pinned-loader validate run guard the
  module against regressing.

### Why / context

- Native acceptance of the receipt-bound notice module failed at load time
  because the loader statically rejects the engine handle being assigned.
  Refs #3068.

### Evidence

- The new call-shape scan and the pinned-loader validate run failed on the
  previous module at the engine-handle assignment and pass after the fix.
- The existing guarded-send notice tests and the local-fast check pass.
