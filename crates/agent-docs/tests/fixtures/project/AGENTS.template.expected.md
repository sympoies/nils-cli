# AGENTS.md

## Startup Policy

Resolve required startup policies before task execution:

```bash
agent-docs resolve --context startup
```

## Project Development Policy

Resolve project development docs before implementing changes:

```bash
agent-docs resolve --context project-dev
```

## Extension Point

Use `AGENT_DOCS.toml` to register additional required documents by context and scope.
