## 2026-11-15 - Preserve guarded composer observation reasons
<!-- devlog-id: 2026-11-15-issue-3720-composer-observation -->

### Result

- Guarded composer refusals retain fixed observation tokens together with stage
  through post-paste cleanup.

### Why / context

- The cleanup suffix in an empty-composer incident identified a post-paste
  refusal, but discarded parser reasons prevented distinguishing escape
  parsing, composer location, and capture failures. Reconstructed NBSP, SGR and
  parameterized OSC-8 captures remain admitted while real drafts stay
  protected; the original incident normalization cause is not established.

### Evidence

- Retained pre-edit RED: the structured observation field was null instead of
  pane-unparsable. Synthetic combined-capture success and draft-preservation
  regression; focused and canonical validation are recorded in the PR.

### Links

- <https://github.com/sympoies/nils-cli/issues/3720>
