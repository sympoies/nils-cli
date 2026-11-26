## 2026-11-26 - Keep the failed account-apply fixture alive through proxy shutdown
<!-- devlog-id: 2026-11-26-issue-4067-account-apply-fixture-cleanup -->

### Result

- Use the runtime-scoped fixture guard so the failed account-apply proxy test
  removes its state only after in-flight blocking writes complete.

### Why / context

- Upstream disconnect can start projection fail-close before the subsequent
  proxy abort, and joining the proxy does not join its blocking state writer.

### Evidence

- Under a test-only forced interleaving, the no-exemption probe reported the
  unhealthy marker and session lock before the fix and found no residue after
  it. The original test assertions passed in both runs.

### Links

- <https://github.com/sympoies/nils-cli/issues/4067>
