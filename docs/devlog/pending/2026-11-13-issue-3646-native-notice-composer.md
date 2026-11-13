## 2026-11-13 - Use native composer facts for supported Claude notices
<!-- devlog-id: 2026-11-13-issue-3646-native-notice-composer -->

### Result

- Supported attached Claude notices use the current bound module authorization
  to establish an empty composer, with a second native read before submit.

### Why / context

- An exact authenticated empty authorization was refused only by an unfamiliar
  synthetic screen; readiness selects the route but never authorizes text.

### Evidence

- T1 red retained before production edits; 17 focused Claude notice tests pass,
  including the executed embedded-module draft race and authorization identity
  negatives.

### Links

- <https://github.com/sympoies/nils-cli/issues/3646>
