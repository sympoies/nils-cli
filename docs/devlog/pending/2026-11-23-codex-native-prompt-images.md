## 2026-11-23 - Codex prompts send managed images as native app-server input
<!-- devlog-id: 2026-11-23-codex-native-prompt-images -->

### Result

- Encode owned raster uploads as ordered turn/start image inputs and refuse
  unsupported or oversized images before dispatch.

### Why / context

- The provider can use attached images directly while the existing managed
  app-server owns the session thread.

### Evidence

- A fake app-server regression first observed the quoted path on the baseline,
  then verified native image input, guarded confirmation, and pre-dispatch
  refusals.

### Links

- <https://github.com/sympoies/nils-cli/issues/3583>
