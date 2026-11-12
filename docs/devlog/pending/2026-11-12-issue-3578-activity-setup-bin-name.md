## 2026-11-12 - Activity setup follows the session binary build
<!-- devlog-id: 2026-11-12-issue-3578-activity-setup-bin-name -->

### Result

- `activity setup` runs the `agent-hook` beside its own executable unless
  `AGENT_SESSION_BIN` pins another, so it renders the same configured session
  binary name as `agent-hook setup`. `agent-hook` stays the authoritative writer.

### Why / context

- Setup forwards to a selected `agent-hook`; a default-build hook found in a
  trusted root rendered the plain name for a renamed session build. The name
  already comes from one shared constant, so no second copy was needed.

### Evidence

- A test with a sibling hook and a different trusted-root hook failed on main
  and passes now. Default-name output is unchanged.

### Links

- <https://github.com/sympoies/nils-cli/issues/3578>
