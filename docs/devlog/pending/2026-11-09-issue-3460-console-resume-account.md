## 2026-11-09 - Carry Console resume account through both providers
<!-- devlog-id: 2026-11-09-issue-3460-console-resume-account -->

### Result

- Console start and resume use one account field across preflight and target
  admission.

### Why / context

- Codex resume requires the retained broker binding and the original explicit
  selection as consent.

### Evidence

- Regression tests showed the dropped pin and target refusal before the fix.

### Links

- <https://github.com/sympoies/nils-cli/issues/3460>
