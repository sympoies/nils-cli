## 2026-10-31 - Reserve the baseline reason for a clean transcript not-found
<!-- devlog-id: 2026-10-31-issue-3129-clean-not-found -->

### Result

- notice-native-baseline-unavailable is now returned only when the provider
  transcript lookup completed and found no candidate; ambiguous, unsafe,
  incomplete or unprovable lookups return notice-native-unavailable.

### Why / context

- Waiting for an ordinary turn cannot fix a duplicate transcript, an unreadable
  entry, an exhausted scan or a transcript without the session id and working
  directory, so the new code was misleading for them. It stays pre-claim with
  no retry permission either way.

### Evidence

- A modeled notice with a duplicate transcript, and with a transcript lacking
  the working directory, returned notice-native-baseline-unavailable before the
  change and notice-native-unavailable after it; the absent-transcript case
  keeps the new code.

### Links

- <https://github.com/sympoies/nils-cli/issues/3129>
- <https://github.com/sympoies/nils-cli/pull/3137>
