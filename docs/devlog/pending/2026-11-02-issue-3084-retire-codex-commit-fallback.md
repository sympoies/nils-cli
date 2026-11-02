## 2026-11-02 - Retire the codex-cli inherited commit fallback
<!-- devlog-id: 2026-11-02-issue-3084-retire-codex-commit-fallback -->

### Result

- codex-cli agent commit in the inherited runtime now stops with a
  missing-dependency error and exit 1 when semantic-commit is not on PATH,
  instead of running an interactive raw git commit; the fallback, its prompt
  helpers, three duplicate scope-helper unit tests and the allowlist row are
  removed.

### Why / context

- The fallback bypassed semantic-commit validation and its signing policy, and
  the isolated runtime already required semantic-commit; both modes now share
  one rule, documented in the README.

### Evidence

- The new missing-dependency integration test failed before the change and
  passes after; agent_commit integration tests, codex-cli lib tests, fmt,
  clippy for the crate, the own-compat audit and the file-size audit pass.

### Links

- <https://github.com/sympoies/nils-cli/issues/3084>
