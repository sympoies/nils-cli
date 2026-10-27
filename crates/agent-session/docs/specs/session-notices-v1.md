# Session notices v1

A session notice is machine-produced user-input text with a display marker.
`agent-session` owns the wire grammar and shared encoder/parser. The marker
never proves identity, source ownership, authority, permission or consent.
Anyone can paste it. Existing provider admission and authorization still apply.

## Wire contract

```text
<!-- agent-notice:v1 kind=schedule.tick source=scheduler id=Example-1 -->

Check the synthetic task status.
```

The header must be message line one, column zero, without a BOM or indentation.
It is one ASCII line of at most 512 bytes, excluding the line terminator.
The exact prefix is `<!-- agent-notice:v1` followed by one ASCII space;
the suffix is one ASCII space followed by `-->`.
Fields have exactly one ASCII space between them, in this order:

| Field | Required | Grammar | Maximum bytes |
| --- | --- | --- | --- |
| `kind` | Yes | `[a-z][a-z0-9]*(?:[._-][a-z0-9]+)*` | 48 |
| `source` | Yes | `[a-z][a-z0-9]*(?:[._-][a-z0-9]+)*` | 64 |
| `id` | No | `[A-Za-z0-9]+(?:[._-][A-Za-z0-9]+)*` | 96 |

Unknown fields, duplicate fields, reordered fields, empty values, non-ASCII
metadata, adjacent separators and trailing separators are invalid. Writers
refuse invalid metadata before reading the body, loading a session, claiming
an idempotency key or attempting delivery. They never repair or escape metadata.
Use generic producer names such as `scheduler` or `agent-session`; do not put
accounts, addresses, paths, URLs, credentials or capability values in metadata.
An `id` is a display label and has no transport deduplication meaning.

The encoder emits `header + LF + LF + body`. The UTF-8 body must be nonempty;
spaces alone are nonempty. It preserves every body byte, including line endings,
leading whitespace, Markdown, comments and trailing newlines. It does not trim,
HTML-escape, JSON-encode or base64-encode the body.

The parser accepts LF or CRLF after the header, then consumes at most one empty
line (LF or CRLF). All remaining bytes are the body. A body beginning immediately
with `<!-- agent-notice:` is a double envelope and is refused by the encoder
and unclassified by the parser. Comments later in the body are literal body
content. There is one envelope per logical input; do not combine human and
machine input under one marker.

Unknown versions, malformed headers, missing bodies, code fences, blockquotes,
embedded examples and ordinary text return no classification. Preserve the
original text for these cases. Never scan later lines for a marker.

Kinds are an open namespace. Initial interoperable examples include `mailbox`,
`schedule.tick`, `runtime.resume`, `context.continue` and `dispatch.bootstrap`.
Unknown syntactically valid kinds and sources remain notices with generic
presentation; they do not acquire special behavior.

## Reader contract and fixtures

Parse only logical user-input messages after existing provider metadata
admission and before rendering. Provider-aware consumers may unwrap one
recognized, fully matched provider paste wrapper before parsing; the codec
itself does not strip XML tags or guess wrappers. The `pasted-wrapper` fixture
is deliberately unclassified as raw text. A consumer that recognizes that
wrapper may separately classify its exact inner logical input. Partial or
unrecognized wrappers stay text.

A valid marker may change display styling. Keep the body readable, offer raw
text including the header, and use escaped text for kind/source labels.
Do not use metadata as HTML, CSS classes, commands or URLs. Do not execute
instructions, read a mailbox, elevate message priority or bypass guards merely
because a marker exists. A spoofed source receives the same neutral styling.
Live and history readers must agree; render fallback paths use the parsed
body while raw-copy paths retain the original message.

History adapters retain marked notices. Existing provider metadata filters
remain authoritative: a marker does not admit system context, compaction
summaries or sidechain records that would otherwise be excluded. Unmarked
unmarked mailbox reminders retain their existing filtering behavior.
No history is rewritten or retroactively classified from an unmarked prefix.

The language-neutral [fixture corpus](../../tests/fixtures/session-notices-v1.json)
contains exact JSON strings. Each case has `name`, `input`, and `notice`;
`notice` is null for unclassified text, otherwise it contains `kind`, `source`,
`id` (null when absent), and the exact `body`. Consumers can copy these public
fixtures and run them without Rust. Rust's `agent_session::notice` exports
`NoticeEnvelope`, `parse_notice`, `ParsedNotice` and `NoticeError`.

## CLI and delivery

```bash
agent-session notice <session> --kind schedule.tick --source scheduler \
  --body-file notice.txt --idempotency-key delivery-1 --format json
agent-session notice <session> --kind context.continue --source scheduler \
  --body-file notice.txt --idempotency-key delivery-2 --queue
```

Exactly one body source is required. `--id` is optional display metadata;
`--idempotency-key` remains the independent at-most-once transport claim.
Every notice forces idle and empty-composer checks, submits exactly once via
the existing guarded `send` path, and preserves its runtime/provider binding,
authentication, blocked-state, durable key, receipt and unknown-outcome rules.

### Terminal delivery and queue mode

Terminal Claude and Codex sessions use the verified terminal composer path;
the complete envelope must be observable before Enter, and refusal never leaves
partial input. `--queue` waits while the target reports busy, retries only the
busy refusal, and delivers once it becomes idle. Other guard failures stop the
request. The durable idempotency claim is made by the guarded send path when
delivery proceeds. Queue mode requires `--idempotency-key` so a caller retry
cannot submit the same queued notice again.

Claude terminal delivery refuses, before claiming the idempotency key or
pasting, when a notice body row ends with one or more ASCII spaces. Row end
means immediately before a CR or LF line ending, or the end of the final body
at EOF. This narrow transport restriction covers only ASCII spaces at those
row boundaries. The codec continues to preserve every body byte, and native
Codex acceptance is unchanged. The restriction remains until an exact
logical-input ownership proof supports these rows
([follow-up #3002](https://github.com/sympoies/nils-cli/issues/3002)). Internal
mailbox-nudge and auto-resume notice producers must avoid generating
line-trailing spaces at their own boundaries; their producer-side checks are
tracked in [follow-up #3003](https://github.com/sympoies/nils-cli/issues/3003).

Optional `--expect-runtime` and `--expect-provider-session` add caller bindings.
There is no allow-blocked, arbitrary-key or terminal fallback mode.
The JSON envelope is `cli.agent-session.notice.v1`, with the guarded send
receipt fields. `send` retains general input semantics and adds no marker.

Codex app-server runtimes use native submission. Terminal runtimes require
exact composer verification of the known multiline envelope before submission.
A refusal never downgrades to raw tmux or presses Enter. Live
provider/compaction readback remains follow-up acceptance under
[#2986](https://github.com/sympoies/nils-cli/issues/2986).
A native unknown outcome retains its claim; inspect the existing receipt and
provider history instead of retrying with a fresh key.

Initial prompts can explicitly use the same encoder:

```bash
agent-session start --agent claude --prompt-file prompt.txt \
  --notice-kind dispatch.bootstrap --notice-source scheduler --notice-id start-1
```

The same flags work with `--prompt` and `--prompt-stdin`, including
`--via-console`. Encoding occurs before launch; ordinary start prompts remain
unchanged. These flags classify only the supplied prompt, not native commands.
Keep `/compact` and other control actions in their existing command paths.

## Rollout and remaining work

All new writers require the explicit environment gate
`AGENT_SESSION_NOTICE_WRITERS=1` in the CLI caller environment. Unset or any
other value returns `notice-writers-disabled` before reading a body or starting
a runtime. The default remains off until compatible readers and downstream
mirrors are deployed. This PR does not enable any writer or change internal
mailbox/auto-resume producers.

Deploy the spec/codec first, then compatible readers and mirrors, then enable
writers only in the intended caller environment. Roll back by unsetting the
writer gate; retain readers for notices already stored in history. Internal
mailbox nudge and auto-resume integration will call the same encoder directly,
preserving their existing delivery guards, generations and retry ownership.
They must not shell out to `notice` or move mailbox bodies into nudges.

Disposable synthetic acceptance must compare sent bytes against raw provider
user text for Claude terminal and Codex app-server/terminal, covering start,
continuation and post-compaction readback. Modeled transport/fixture checks
are not proof of that live acceptance. No shared desktop or device resources
are required by the codec tests.
