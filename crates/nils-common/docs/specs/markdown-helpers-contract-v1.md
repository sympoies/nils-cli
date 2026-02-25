# markdown Helpers Contract v1

## Purpose
Define the shared markdown helper behavior in `nils-common::markdown` used by multiple CLI crates.

## APIs

### `validate_markdown_payload(markdown: &str)`
- Rejects literal escaped-control artifacts (`\\n`, `\\r`, `\\t`) in markdown payloads.
- Accepts real control characters (`\n`, `\r`, `\t`).
- Returns structured violations via `MarkdownPayloadError`.

### `canonicalize_table_cell(value: &str) -> String`
- Produces markdown-table-safe cell text for render/compare round-trips.
- Normalizes:
  - `|` -> `/`
  - `\n` / `\r` runs -> single space
- Idempotent: applying it multiple times yields the same output.

## Intended Call Sites
- Markdown table renderers before writing rows.
- Drift/contract comparisons for table-parsed values.
- Notes fields that must survive markdown table serialization without false-positive diffs.

## Ownership
- `nils-common` owns the canonical production implementation.
- Consuming crates should not duplicate table-cell canonicalization logic.

## Examples
- `A|B` -> `A/B`
- `"line1\r\nline2"` -> `"line1 line2"`
- `"x\n\n y"` -> `"x  y"` is not guaranteed; callers should not rely on whitespace collapsing beyond line-break run normalization.
