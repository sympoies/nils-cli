## 2026-11-10 - Retain provider-owned Codex daemons during session deletion
<!-- devlog-id: 2026-11-10-issue-3521-codex-daemon-delete -->

### Result

- Deletion distinguishes shared Codex provider infrastructure from session
  processes and reports retained daemon identities.

### Why / context

- Codex 0.162.1 detaches its per-home daemon while inheriting the first session
  markers and cgroup; usage exhaustion does not make that daemon session-owned.

### Evidence

- Fixture regressions reproduced marker and cgroup deletion failures and
  rejected untrusted provider-home proof; focused provider-daemon tests passed
  after the ownership fix.

### Links

- <https://github.com/sympoies/nils-cli/issues/3521>
