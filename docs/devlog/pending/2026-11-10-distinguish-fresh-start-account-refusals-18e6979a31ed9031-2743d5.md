## 2026-11-10 - Distinguish fresh start account refusals
<!-- devlog-id: 2026-11-10-distinguish-fresh-start-account-refusals-18e6979a31ed9031-2743d5 -->

### Result

- Fresh direct starts with an account report their supported usage instead of a
  resume refusal.

### Why / context

- The request names no resume, so a resume error misdirects diagnosis while
  real resume refusals retain their contract.

### Evidence

- A focused CLI regression reproduced the previous error before the fix.

### Links

- <https://github.com/sympoies/nils-cli/issues/3496>
