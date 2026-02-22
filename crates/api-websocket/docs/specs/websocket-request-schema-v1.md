# WebSocket Request Schema v1

## Scope
This schema defines deterministic, file-based WebSocket request execution used by:
- `api-websocket call`
- `api-websocket report --run`
- `api-test` suite cases with `type: "websocket"`

## Top-level object
A request file must be a JSON object.

Supported top-level fields:
- `url` (string, optional): explicit WebSocket target.
- `headers` (object, optional): handshake headers.
- `connectTimeoutSeconds` (integer/string, optional): reserved timeout input (accepted for contract parity).
- `steps` (array, required): ordered scripted session steps.
- `expect` (object, optional): assertion against the last received message.

## Step schema
Each `steps[i]` must include `type`.

### `type: "send"`
- `text` or `json` or `payload` (required)
- payload is serialized to text before send.

### `type: "receive"`
- `timeoutSeconds` (optional)
- `expect` (optional):
  - `textContains` (string)
  - `jq` (string, evaluated against JSON-parsed receive text)

### `type: "close"`
- no extra fields required.

## Expect object
Top-level or step-level `expect` supports:
- `textContains`: substring match
- `jq`: jq expression evaluated against JSON message text

Validation behavior:
- if both are omitted/empty, expect is ignored.
- jq assertions fail when receive text is not valid JSON.

## Error behavior
Deterministic schema errors include:
- request file is not valid JSON
- request root is not a JSON object
- `steps` is missing/empty
- unsupported `steps[i].type`
- missing send payload fields

## Fixture matrix
| Fixture | Purpose | Expected outcome |
| --- | --- | --- |
| `health.ws.json` | send/receive success (`jq` true) | pass |
| `expect-fail.ws.json` | receive message fails `textContains`/`jq` | failure with assertion message |
| `invalid-json.ws.json` | malformed JSON file | schema load failure |
| `missing-steps.ws.json` | no `steps` field | schema validation failure |
| `connect-fail.ws.json` | unreachable target URL | connection failure |

## Reusable fixture pattern
A minimal reusable fixture for both CLI and suite tests:
```json
{
  "steps": [
    {"type": "send", "text": "ping"},
    {"type": "receive", "expect": {"jq": ".ok == true"}},
    {"type": "close"}
  ],
  "expect": {"textContains": "ok"}
}
```
