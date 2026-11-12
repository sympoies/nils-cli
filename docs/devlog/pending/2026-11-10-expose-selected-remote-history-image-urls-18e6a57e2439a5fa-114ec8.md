## 2026-11-10 - Expose selected remote history image URLs
<!-- devlog-id: 2026-11-10-expose-selected-remote-history-image-urls-18e6a57e2439a5fa-114ec8 -->

### Result

- Remote image refusals include the selected HTTP or HTTPS URL when bounded to
  2048 bytes.

### Why / context

- Consumers can bind a fetch to the runtime selector without independently
  parsing conversation Markdown.

### Evidence

- A pre-fix route regression showed the selected URL was absent.

### Links

- <https://github.com/sympoies/nils-cli/issues/3517>
