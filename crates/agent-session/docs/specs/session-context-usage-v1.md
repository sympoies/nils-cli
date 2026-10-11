# Session context usage v1

`list`, session inspection (`command ID --format json` and the HTTP session
endpoint), and board records may include the additive `context_usage` object.
Existing envelope versions remain unchanged; consumers must accept a missing
object and nullable members. The board advertises `context_usage.v1`.
This is a read-time projection, never a persisted session-record update.
The board closed-record ledger may retain these allowlisted facts in its
historical snapshot.

```json
{
  "context_usage": {
    "used_tokens": 130,
    "window_tokens": 200000,
    "source": "claude-transcript",
    "observed_at": "2026-01-01T00:00:00Z",
    "model": "claude-haiku-4-5-20251001"
  }
}
```

| Member | Type | Meaning |
| --- | --- | --- |
| `used_tokens` | unsigned integer or null | Latest provider context usage, after any compaction. |
| `window_tokens` | positive unsigned integer or null | Capacity supported by provider facts or an unambiguous known model. |
| `source` | string or null | `claude-transcript` or `codex-rollout`. |
| `observed_at` | RFC3339 string or null | Provider event time; never replaced with the time of a poll. |
| `model` | string or null | Model reported by the matching provider observation. |

Claude usage sums the latest matching non-sidechain assistant message's
`input_tokens`, `cache_read_input_tokens`, and `cache_creation_input_tokens`.
Output tokens are excluded. Codex uses `last_token_usage.total_tokens`, including
that turn's input (cached input included) and output, rather than the cumulative
`total_token_usage`. Its capacity comes from `model_context_window`; its model
comes from the preceding `turn_context` in the same bounded read.

Claude capacity uses one conservative table, including explicit `[1m]`
variants. Unknown models and models whose base ID does not distinguish the
active 200K/1M variant return null. A capacity inferred from a model describes
that known variant; it cannot infer account or gateway overrides. See the
provider's [model configuration](https://code.claude.com/docs/en/model-config).

The source is bound to the managed session's provider resume identity and
configured provider root, using bounded unique-match discovery. Discovery is
cached for 30 seconds per binding. Each read considers at most the final
256 KiB, skips partial lines, and caches only the five allowlisted facts with a
file fingerprint. Within the discovery cache lifetime, unchanged files require no
transcript-body read. Both source
and observation caches are bounded to 256 entries. A command started in a new
process performs the same bounded read; it never reads a complete large file.

Missing, replaced, truncated, or unreadable files do not reuse cached usage.
A compaction boundary invalidates earlier usage until a later valid observation
appears. If no observation is present in the bounded tail, the object is absent.
A valid but old observation retains its original `observed_at`; consumers choose
their own freshness threshold. No percentage or remaining-token estimate is
added. Board projection drops unknown members and never includes transcript
paths, conversation text, or provider cache internals.
