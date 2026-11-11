## 2026-11-06 - Activity broker no longer re-triggers itself from its own reads
<!-- devlog-id: 2026-11-06-issue-3423-activity-broker-read-events -->

### Result

- The serve activity broker ignores read-only filesystem access events, so a
  snapshot sweep no longer schedules the next sweep on Linux; an idle serve
  stops sweeping every session's state files at the 250 ms cadence cap.

### Why / context

- notify arms inotify with IN_OPEN, delivering every read of activity.json or
  session.json as an Access event. The relevance filter matched file names
  regardless of event kind, so each sweep's own reads queued another sweep.
  Idle cost therefore scaled with the session count. macOS FSEvents does not
  report reads, so the loop was Linux-only.

### Evidence

- A paused-clock regression over 35 idle fixture sessions counts sweeps and
  opens: 8 sweeps and 2240 opens over 8 ticks before, 1 sweep and 280 opens
  after. Created, modified, and closed-after-write state files still
  refresh the snapshot.

### Links

- <https://github.com/sympoies/nils-cli/issues/3423>
