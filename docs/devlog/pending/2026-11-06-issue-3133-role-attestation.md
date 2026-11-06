## 2026-11-06 - Attest operator-launched session roles
<!-- devlog-id: 2026-11-06-issue-3133-role-attestation -->

### Result

- Add explicit per-session operator role provenance to the authenticated broker
  binding; managed child and new-root launches cannot inherit it.

### Why / context

- An ordinary session role and an inherited initiator do not distinguish a
  supervisor from its descendants. The accepted boundary remains the existing
  same-OS-user trusted launcher.

### Evidence

- Contract tests cover operator launch, managed self-assignment refusal, child
  provenance clearing, authenticated audit tuple projection and generic HTTP
  forgery refusal.

### Links

- <https://github.com/sympoies/nils-cli/issues/3133>
