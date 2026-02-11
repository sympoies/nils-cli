# crates.io Publish Report

## Summary

- Mode: `<publish|dry-run-only>`
- Retry behavior: `<stop-on-rate-limit|wait-and-retry>`
- Started (UTC): `<YYYY-MM-DDTHH:MM:SSZ>`
- Ended (UTC): `<YYYY-MM-DDTHH:MM:SSZ>`
- Selected crates: `<N>`
- Published: `<N>`
- Skipped existing: `<N>`
- Dry-run only: `<N>`
- Failed: `<N>`
- Not attempted: `<N>`
- Next eligible publish time (UTC): `<optional YYYY-MM-DDTHH:MM:SSZ>`
- Status snapshot: `<ok|failed|skipped>`
- Status snapshot exit code: `<optional int>`
- Status JSON: `<optional path>`
- Status text: `<optional path>`

## Successful Uploads

| Crate | Version | Status | Start (UTC) | End (UTC) | Attempts | Note |
|---|---:|---|---|---|---:|---|
| `<crate-name>` | `<x.y.z>` | `published` | `<time>` | `<time>` | `<n>` | `<details>` |

## Failed Uploads

| Crate | Version | Status | Start (UTC) | End (UTC) | Attempts | Note |
|---|---:|---|---|---|---:|---|
| `<crate-name>` | `<x.y.z>` | `failed` | `<time>` | `<time>` | `<n>` | `<error or rate-limit details>` |

## Full Attempts

| Crate | Version | Status | Start (UTC) | End (UTC) | Attempts | Note |
|---|---:|---|---|---|---:|---|
| `<crate-name>` | `<x.y.z>` | `<published|failed|skipped|pending|dry-run-ok>` | `<time>` | `<time>` | `<n>` | `<details>` |
