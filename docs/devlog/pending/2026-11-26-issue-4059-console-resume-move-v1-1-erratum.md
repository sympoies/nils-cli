## 2026-11-26 - Erratum: Console resume-move v1.1 drops the body owner
<!-- devlog-id: 2026-11-26-issue-4059-console-resume-move-v1-1-erratum -->

### Result

- Removed `caller.owner` from the Console resume-move start request, so the
  relay's caller grant is the only owner authority.
- Stated that `resume_move` lineage appears on `GET /sessions` only for
  bearer-authenticated requests.

### Why / context

- A managed caller cannot know its Console owner, and the body assertion
  protected nothing the relay grant does not already decide.
- The open listing would otherwise expose the source host, session and
  provider conversation IDs.

### Evidence

- Spec-only change, checked against the design answers on the tracking issue.
  The Salon relay parser changes in the same release.

### Links

- <https://github.com/sympoies/nils-cli/issues/4059>
