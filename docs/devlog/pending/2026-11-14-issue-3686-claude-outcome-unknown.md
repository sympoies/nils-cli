## 2026-11-14 - Replace Claude idle interrupt inference with fixed uncertainty
<!-- devlog-id: 2026-11-14-issue-3686-claude-outcome-unknown -->

### Result

- Claude structured idle over an unmatched open turn now projects unknown
  immediately with claude_turn_outcome_unknown, while preserving durable
  attention and late completion handling.

### Why / context

- The Claude Code 2.1.317 audit found no documented unambiguous interactive
  interrupt or manual-denial outcome; background stopped and undocumented
  transcript labels cannot supply that distinction. Elapsed idle must not
  invent an interruption.

### Evidence

- Five retained red assertions preceded production edits; all 51 focused shadow
  tests passed after the change, including late Stop settlement and pending
  approval preservation.

### Links

- <https://github.com/sympoies/nils-cli/issues/3686#issuecomment-6868576668>
