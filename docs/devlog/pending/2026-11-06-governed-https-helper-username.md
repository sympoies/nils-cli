## 2026-11-06 - Request the selected login from HTTPS credential helpers
<!-- devlog-id: 2026-11-06-governed-https-helper-username -->

### Result

- Pass the selected profile login to trusted HTTPS credential helpers while
  retaining authenticated actor verification.

### Why / context

- Multi-account helpers need the requested login to select a credential without
  switching the active account.

### Evidence

- The username-aware helper regression failed before the fix and the four
  focused HTTPS fixture tests passed after it.

### Links

- <https://github.com/sympoies/nils-cli/pull/3334>
