## 2026-10-31 - Tested Claude Code range for the native notice module
<!-- devlog-id: 2026-10-31-issue-3136-claude-notice-tested-range -->

### Result

- The native Claude notice module now mounts on any Claude Code release in a
  reviewed range (2.1.296 through 2.1.314) instead of one exact version, and a
  launch records why the module was not attached.
- A notice to a session outside the range returns
  notice-native-version-unsupported with a native_module object (reason,
  observed version, supported range); other unattached sessions keep
  notice-transport-unsupported with the same object.

### Why / context

- Claude Code updates frequently, so an exact pin silently disabled native
  notices on every updated host. The range stays a reviewed list because the
  module depends on loader rules and API semantics; it is never widened at
  runtime.

### Evidence

- Launch and notice boundary tests failed before the change (2.1.314 did not
  mount; the unsupported receipt had no native_module). The module call-shape
  scan and claude plugin validate pass on 2.1.296 and on an integrity-verified
  2.1.314 build.

### Links

- <https://github.com/sympoies/nils-cli/issues/3136>
- <https://github.com/sympoies/nils-cli/issues/3068>

### Follow-ups

- Sessions started before the upgrade lack the new bootstrap field and stay
  unattached until restarted. A scheduled contract check for newer releases is
  tracked separately.
