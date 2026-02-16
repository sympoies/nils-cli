# api-websocket Rollout Runbook

## Goal
Adopt `api-websocket` incrementally while keeping existing REST/GraphQL/gRPC workflows stable.

## Phased rollout
1. Local validation only
   - Add request fixtures under `setup/websocket/requests/`.
   - Run `api-websocket call` and `api-websocket report` locally.
2. Suite opt-in
   - Add isolated `type: "websocket"` cases to non-critical suites.
   - Monitor `api-test run` output artifacts and failure rates.
3. Mixed suite adoption
   - Add websocket cases into mixed protocol suites.
   - Keep fail-fast disabled initially for better triage.
4. CI gate
   - Enable websocket suites in required CI jobs after stability baseline is met.

## Rollback triggers
- Repeated handshake failures across environments.
- Flaky timeout/assertion failures with no deterministic root cause.
- Unexpected regressions in non-websocket suite paths.

## Rollback steps
1. Remove or skip websocket cases in suite manifests.
2. Keep `api-websocket` CLI available for local-only diagnostics.
3. Re-enable suite cases only after fixture/runtime stabilization.

## Troubleshooting

### Handshake/connection failures
- Verify resolved URL (`--url` vs `--env` vs defaults).
- Check `setup/websocket/endpoints.env` keys: `WS_URL_<PROFILE>`.
- Confirm target is reachable from CI network context.

### Auth failures
- Verify token profile name (`--token` or `WS_TOKEN_NAME`).
- Check `WS_TOKEN_<PROFILE>` in `tokens.env`/`tokens.local.env`.
- Validate fallback envs (`ACCESS_TOKEN`, `SERVICE_TOKEN`) when profile is omitted.

### Timeout/receive issues
- Ensure test server sends deterministic responses in expected order.
- Prefer explicit `receive` steps and avoid ambiguous message sequencing.
- Tune fixture expectations (`textContains`/`jq`) before widening timeout budgets.

### Assertion failures
- Inspect per-case stdout transcript artifact from `api-test` run directory.
- Re-run with text mode first, then validate JSON-mode contract outputs.

## Local and CI commands
Local smoke:
```bash
api-websocket call --env local setup/websocket/requests/health.ws.json
api-websocket report --case ws-health --request setup/websocket/requests/health.ws.json --run
```

CI-style suite run:
```bash
api-test run --suite websocket-smoke --out out/api-test-runner/ws-smoke.json
```
