## 2026-11-06 - Keep mention matches in full session ID searches
<!-- devlog-id: 2026-11-06-history-full-id-mentions -->

### Result

- Full Claude session ID searches retain the directly resolved session first
  and include bounded metadata matches without duplication.

### Why / context

- Direct lookup must not hide other sessions whose searchable metadata mentions
  the ID.

### Evidence

- Regression test reproduces the missing mention on the previous behavior.

### Links

- <https://github.com/sympoies/nils-cli/issues/3363>
