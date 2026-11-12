## 2026-11-06 - Keep Linux release binaries within the supported glibc baseline
<!-- devlog-id: 2026-11-06-issue-3384-linux-glibc-baseline -->

### Result

- Pinned x86_64 release and cache builds to Ubuntu 24.04 and added a pre-upload
  symbol-version gate for every packaged Linux binary.

### Why / context

- A moving release runner linked image-processing float math imports against
  GLIBC_2.43, above the supported GLIBC_2.39 baseline.

### Evidence

- The checker rejected published v1.40.9 image-processing with GLIBC_2.43 and
  exit status 1 before the workflow fix.

### Links

- <https://github.com/sympoies/nils-cli/issues/3384>
