## 2026-11-18 - Identify Claude native mailbox admission refusals
<!-- devlog-id: 2026-11-18-identify-claude-native-mailbox-admission-refusals-18e92e318e51591c-1233d0 -->

### Result

- Queued native wakes preserve the failing identity, idle, transcript,
  attachment, or coordination guard in mail audit.

### Why / context

- The previous generic refusal concealed why a provider could be idle while its
  notice generation remained queued.

### Evidence

- Modeled native mailbox tests passed for all five guard families and verified
  that the refused generation released no notice text.

### Links

- <https://github.com/sympoies/nils-cli/issues/3821>
