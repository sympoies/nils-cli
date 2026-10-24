## 2026-10-24 - Bind custom provider capacity and account applicability to launches
<!-- devlog-id: 2026-10-24-issue-2609-launch-facts -->

### Result

- Custom Claude capacity is projected with runtime and launch-configuration
  freshness; account applicability is distinct from current broker support.

### Why / context

- A later daemon environment cannot reconstruct an older provider launch.
  Recovered the retained candidate and repaired invalid override precedence,
  unquoted provider selection and private launch transport.

### Evidence

- Meaningful current-main regressions reproduced before repair. Focused
  context/account, profile lifecycle and isolated tmux fixtures pass;
  historical candidate failures and passing gate remain preserved separately.
  Canonical local-fast is deferred for this source lane; hosted CI and
  independent review remain delivery gates.

### Links

- <https://github.com/sympoies/nils-cli/issues/2609>
