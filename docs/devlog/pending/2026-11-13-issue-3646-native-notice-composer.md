## 2026-11-13 - Use native composer facts for supported Claude notices
<!-- devlog-id: 2026-11-13-issue-3646-native-notice-composer -->

### Result

- Supported attached Claude notices use the current bound module authorization
  to establish an empty composer, with a second native read before submit.
- Native human drafts remain until human clearing, with an occupied-composer
  deferral reason; ordinary terminal stale-draft admission is unchanged.
- The updated module requires a fresh launch, resume or restart through the
  existing mount flow; an older mounted module hash fails closed.

### Why / context

- An exact authenticated empty authorization was refused only by an unfamiliar
  synthetic screen; readiness selects the route but never authorizes text.

### Evidence

- T1 red retained before production edits; 18 focused Claude notice tests pass,
  including the executed embedded-module draft race and authorization identity
  negatives. Review repairs retain idle-fence mutation red and native diagnostic
  red/green evidence.

### Links

- <https://github.com/sympoies/nils-cli/issues/3646>
