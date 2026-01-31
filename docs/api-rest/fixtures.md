# api-rest fixtures

These fixtures define deterministic scenarios for integration tests. Tests should use a local HTTP server and temporary
`setup/rest` directories (do not rely on external network).

## call: 2xx success (no expect)
- Setup:
  - Start a local HTTP server that returns `200` with JSON body: `{"ok":true}` for `GET /health`.
  - Create `setup/rest/requests/health.request.json` with `method=GET`, `path=/health`.
- Command: `api-rest call --url http://127.0.0.1:<port> setup/rest/requests/health.request.json`
- Expect:
  - exit `0`
  - stdout is the response body JSON
  - stderr is empty

## call: non-2xx fails when expect is absent
- Setup:
  - Server returns `500` with JSON body (or plain text body).
  - Request file targets that endpoint.
- Command: `api-rest call --url http://127.0.0.1:<port> setup/rest/requests/fail.request.json`
- Expect:
  - exit `1`
  - stdout still contains the response body (parity with `rest.sh`)
  - stderr contains an HTTP failure message

## call: expect.status mismatch
- Setup:
  - Server returns `200` for `GET /health`.
  - Request includes `"expect": { "status": 201 }`.
- Command: `api-rest call --url http://127.0.0.1:<port> setup/rest/requests/health.request.json`
- Expect:
  - exit `1`
  - stderr contains `Expected HTTP status 201 but got 200.`

## call: expect.jq failure
- Setup:
  - Server returns `200` with JSON body: `{"ok":false}`.
  - Request includes `"expect": { "status": 200, "jq": ".ok == true" }`.
- Command: `api-rest call --url http://127.0.0.1:<port> setup/rest/requests/health.request.json`
- Expect:
  - exit `1`
  - stderr contains `expect.jq failed: .ok == true`

## call: strict query encoding
- Setup:
  - Request includes `.query` with:
    - scalar values, arrays of scalars, nulls (omitted), and at least one invalid object value.
- Command: run `api-rest call ...`
- Expect:
  - valid query produces deterministic ordering and encoding
  - invalid query produces a hard-fail with a clear error (objects rejected)

## call: user headers validation and filtering
- Setup:
  - Request includes `.headers` containing:
    - a valid header key/value,
    - an invalid header key (`"Bad Key": "x"`),
    - an `Authorization` header (must be rejected/ignored),
    - a `Content-Type` header (must be rejected/ignored).
- Command: run `api-rest call ...`
- Expect:
  - invalid header key hard-fails with an actionable message
  - forbidden headers are not applied from the request file

## call: multipart file upload (filePath)
- Setup:
  - Create a temp file `setup/rest/requests/fixtures/upload.bin`.
  - Server accepts multipart at `POST /upload` and echoes metadata as JSON.
  - Request includes `multipart` part with `{ "name": "file", "filePath": "setup/rest/requests/fixtures/upload.bin" }`.
- Command: `api-rest call --url http://127.0.0.1:<port> setup/rest/requests/upload.request.json`
- Expect:
  - exit `0`
  - server receives a multipart body with the file

## call: multipart file upload (base64)
- Setup:
  - Request includes `multipart` part with `{ "name": "file", "base64": "<base64 payload>", "filename": "x.bin" }`.
  - Server accepts the upload.
- Command: run `api-rest call ...`
- Expect:
  - exit `0`
  - base64 payload is decoded and uploaded

## call: cleanup success
- Setup:
  - Main request returns JSON body containing a cleanup key, for example `{ "key": "abc" }`.
  - Request includes:
    - `cleanup.pathTemplate="/files/{{key}}"`
    - `cleanup.vars={ "key": ".key" }`
  - Server returns `204` for `DELETE /files/abc`.
- Command: run `api-rest call ...`
- Expect:
  - exit `0`
  - cleanup request is executed after a successful main request

## call: cleanup failure
- Setup:
  - Cleanup endpoint returns a non-expected status.
- Command: run `api-rest call ...`
- Expect:
  - exit `1`
  - stderr contains `cleanup failed: expected ... but got ...`

## history: append and command-only output
- Setup:
  - Run a successful call with `--config-dir setup/rest` so the setup dir is unambiguous.
  - Ensure `.rest_history` is created under that setup dir.
- Command:
  - `api-rest history --config-dir setup/rest --last`
  - `api-rest history --config-dir setup/rest --command-only`
- Expect:
  - history entry starts with a metadata line beginning with `#`
  - command-only output omits the metadata line
  - command snippet includes `--config-dir` and ends with `| jq .`
  - token values are never present; only token profile names may appear

## history: rotation
- Setup:
  - Set `REST_HISTORY_MAX_MB=0` to disable rotation (control case), then set a very small value to force rotation.
  - Run enough calls to exceed the threshold.
- Command: run repeated calls.
- Expect:
  - rotated files exist (for example `.rest_history.1`) and are bounded by `REST_HISTORY_ROTATE_COUNT`.
