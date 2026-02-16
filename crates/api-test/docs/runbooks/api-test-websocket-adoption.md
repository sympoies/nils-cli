# api-test WebSocket Adoption Runbook

## Purpose
Guide gradual adoption of suite cases using `type: "websocket"` in `api-test` manifests.

## Contract checklist
- Suite defaults can define:
  - `defaults.websocket.configDir` (default: `setup/websocket`)
  - `defaults.websocket.url`
  - `defaults.websocket.token`
- Case-level requirements:
  - `id`
  - `type: "websocket"` (or alias `"ws"`)
  - `request` path to a websocket request file

## URL/token resolution in suites
WebSocket case runtime precedence:
1. case `url`
2. `defaults.websocket.url`
3. `API_TEST_WS_URL`
4. case/default `env` profile lookup (`WS_URL_<ENV>`)
5. fallback `ws://127.0.0.1:9001/ws`

Token resolution:
1. case `token`
2. `defaults.websocket.token`
3. profile lookup `WS_TOKEN_<PROFILE>` in setup files

## Suggested rollout
1. Create a dedicated websocket smoke suite.
2. Run it in non-blocking CI first.
3. Add one websocket case into mixed suites.
4. Promote to required CI after stability period.

## Common failures and actions
- `websocket case '<id>' request not found`
  - fix `request` path relative to repo root.
- `Unknown env '<name>'`
  - add corresponding `WS_URL_<NAME>` endpoint entry.
- token profile missing
  - add `WS_TOKEN_<PROFILE>` in `tokens.env`/`tokens.local.env` or remove token profile selection.
- assertion failures
  - inspect `<run_dir>/<case>.response.json` transcript and `<case>.stderr.log`.

## Verification commands
```bash
cargo test -p nils-api-testing-core --test suite_runner_websocket_matrix
cargo test -p nils-api-testing-core --test suite_rest_graphql_matrix
cargo test -p nils-api-test suite_schema
```
