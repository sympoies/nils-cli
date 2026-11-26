## 2026-11-27 - Specify fenced Console succession stop
<!-- devlog-id: 2026-11-27-issue-4100-console-succession-stop -->

### Result

- Define separate successor authorization, a native guarded stop and retained
  exact-operation evidence.

### Why / context

- The Console input fence has to survive ambiguous stop outcomes and outlive
  any receipt that can expire while old writes are still possible.

### Evidence

- The contract records the issuer UTC deadline and the conditions for releasing
  the fence and compacting evidence.

### Links

- <https://github.com/sympoies/nils-cli/issues/4100>
