## 2026-11-06 - Edit issue and PR timeline comments
<!-- devlog-id: 2026-11-06-issue-3272-comment-edit -->

### Result

- Added timeline comment editing for GitHub and GitLab.

### Why / context

- GitLab requires the parent IID for its REST note endpoint; GitHub addresses
  timeline comments directly by ID. GitLab may normalize the saved body, so
  its response is returned without comparing it to the request.

### Evidence

- Provider fake integration tests cover both providers, permission refusals,
  exact bodies, and a missing GitLab parent.

### Links

- <https://github.com/sympoies/nils-cli/issues/3272>
