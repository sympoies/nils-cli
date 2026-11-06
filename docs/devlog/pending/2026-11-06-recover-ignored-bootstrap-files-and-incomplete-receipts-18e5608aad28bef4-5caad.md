## 2026-11-06 - Recover ignored bootstrap files and incomplete receipts
<!-- devlog-id: 2026-11-06-recover-ignored-bootstrap-files-and-incomplete-receipts-18e5608aad28bef4-5caad -->

### Result

- Explicit bootstrap files bypass Git ignore rules, and incomplete receipts can
  be abandoned without deleting the remote.

### Why / context

- An ignored explicit input previously left a created empty repository and an
  input-bound receipt that could not resume.

### Evidence

- The focused GitHub bootstrap integration tests passed against fake provider
  responses and a local bare Git repository.

### Links

- <https://github.com/sympoies/nils-cli/issues/3401>
