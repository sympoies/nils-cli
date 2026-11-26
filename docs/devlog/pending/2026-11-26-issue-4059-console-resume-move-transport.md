## 2026-11-26 - Specify the Console move relay transport supplement
<!-- devlog-id: 2026-11-26-issue-4059-console-resume-move-transport -->

### Result

- Specified additive capability discovery, destination status and delegated
  binding without changing the v1 start envelope.

### Why / context

- The relay needs verified freshness and authenticated scope before dispatch,
  and a read-only way to reconcile an unknown outcome.

### Evidence

- Checked the design against the existing daemon capability, federation ingress
  and native move status contracts.

### Links

- <https://github.com/sympoies/nils-cli/issues/4059>
