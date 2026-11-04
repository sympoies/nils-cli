# History row classes v1

Design contract for [#3225](https://github.com/sympoies/nils-cli/issues/3225),
[#3226](https://github.com/sympoies/nils-cli/issues/3226) and
[#3228](https://github.com/sympoies/nils-cli/issues/3228). This design and its
[schema fixture](fixtures/history-row-classes-v1.json) precede runtime
implementation. They do not claim that a released daemon emits these fields.
The current routes, bearer authorization and cursor grammar are owned by
[Serve history v1](serve-history-v1.md).

## Row contract

Add optional `row_class` to each normalized history message and to the matching
live transcript message. The field is independent of `parts=v1` and has exactly
these values:

| `row_class` | Meaning |
| --- | --- |
| `runtime_command` | A complete recognized provider runtime command, whoever submitted it. |
| `compaction` | One provider-recorded context replacement, with its summary in `text`. |
| `notice` | A marked notice, an existing daemon notice event, or a precisely recognized machine text form. |

An unrecognized row omits `row_class` entirely: no null, empty string,
`unknown`, `user`, or inferred class. Ordinary human and assistant messages
have no class. An unknown slash command stays an ordinary prompt. Classifying
text is display data, never proof of authorship, consent, authority or transport
admission. A person can paste any recognized form.

`role` retains its existing meaning. A runtime command recorded as user input
keeps `role: "user"`; a marked human input also keeps that role. Receipt-bound
native notices and [mailbox read events](mailbox-read-events-v1.md) keep
`role: "notice"` and gain `row_class: "notice"`. Newly exposed provider
compaction events use `role: "notice"`, `row_class: "compaction"`. Unmarked
recognized inputs keep their provider role and exact logical `text` bytes.
The class, rather than `role` or text matching, drives system-row presentation.

There is no new top-level `kind`, notice envelope field, route or cursor.
The existing notice codec still returns kind/source/id/body; `row_class` does
not replace it or the `mailbox` event object. With `parts=v1`, compaction and
plain machine-text rows have `parts: []`; their `text` is the sole body.
Existing typed tool/notice parts retain their own semantics; a part whose kind
is `notice` does not by itself classify its enclosing assistant message.

`content_item_kinds` is Codex provider metadata, not a row class or a public
field added by this contract. Preserve its provider admission rules:
`user.text`/`user.image` identify candidate user input; known injected context
and unknown internal kinds do not become human input. Existing metadata-filtered
rows stay filtered. Only older, otherwise visible Codex user frames lacking
that metadata use the complete context-block recognizers below. Never derive
a class merely from absent metadata, an unfamiliar kind, or a leading slash.

All three classes are excluded from human outbox acknowledgement, human prompt
association, first/last-user previews, stuck/pending reconciliation, attention
signals and automatic title/semantic-memory input. Existing provider filters
for sidechains, tool output, unrelated system rows and invalid native notice
proof remain in force. This display addition neither changes runtime input nor
widens a send/command admission policy.

## Runtime command table

Each provider adapter owns one small exact-token table. These are display
recognizers, not aliases or executable command definitions. They recognize only
commands actually recorded by that provider; no row is synthesized from a send
receipt or a presumed provider action.

| Provider | Exact command tokens |
| --- | --- |
| Claude Code | `/compact`, `/exit`, `/quit`, `/effort`, `/model`, `/usage`, `/login`, `/clear`, `/new` |
| Codex CLI | `/compact`, `/exit`, `/quit`, `/effort`, `/model`, `/usage`, `/login`, `/clear`, `/new` |

Match the complete logical input starting at column zero: the exact token,
optionally followed by ASCII space or tab and arguments. A newline, quote,
code fence or preceding prose is not a command separator. The logical input
must be one line. Token matching is case-sensitive: `/compactness`, `/Compact`
and `/custom` have no class. Preserve argument bytes, spaces and tabs. Claude
command-markup rows first use the existing fully matched command markup
projection; classification applies to that projected command text.

Preserve [#3014](https://github.com/sympoies/nils-cli/issues/3014): Claude's
literal `/compact` input and its command-markup echo are one command invocation;
retain the echo once, including arguments, rather than resurrecting the literal.
Two genuine invocations with identical text remain two commands. The command
row is distinct from the single compaction event below: requesting compaction
and completing a context replacement are different facts. Do not manufacture a
compaction event for an unsuccessful command.

Multiple compact command-markup echoes following one nonautomatic boundary and
summary belong to that same invocation until the next non-echo logical record
or boundary. Retain the first echo's command identity and suppress the later
echoes. A separate compaction or a command not attached to that pair remains a
separate invocation, even when its command text is identical.

## Compaction summary and paging

Expose exactly one `row_class: "compaction"` row per provider compaction.
For Claude Code, the primary transcript's `system/compact_boundary` is the
event anchor; its associated `isCompactSummary` user record supplies `text`.
The boundary and summary do not become two events. For Codex CLI, the top-level
`compacted` record is the anchor and `payload.message`, when it is a string,
supplies `text`. Missing summaries yield the empty string, not invented prose.
An unrelated continuation notice has class `notice`, not `compaction`.

The compaction row uses the existing history-id/physical-offset row-id format,
anchored at the boundary or `compacted` record, and that record's timestamp when
available. It adds `compaction: {"summary_truncated": false}`. Retain the first
65,536 Unicode scalar values of the provider summary without trimming; set
`summary_truncated: true` only when more summary text exists. This is the
existing 64-Ki-character history text bound, distinct from the notice summary
input bound. There is no duplicate summary field, new original-body route, or
summary-derived cursor. `compaction` is absent on every other class.

Forward, latest/older, live transcript and card-preview projections use the same
classifier. Normalized ids, classes, text and compaction metadata must agree.
A summary on another raw line does not consume a second visible row. A Claude
boundary/summary followed by its adjacent compact command echo projects the
command first, then the compaction. The command keeps its echo-based id and
timestamp; the compaction keeps its boundary-based id and timestamp. Paging uses
the boundary position for the command and the echo position for the compaction,
independently of row identity, so pages of size one retain both rows once. Literal
duplicate suppression remains unchanged; other rows keep file order. Card
previews retain the compaction when that adjacent echo arrives.

The cursor grammar remains a decimal byte offset. A traversal spanning the
ordering upgrade can repeat or miss a command/compaction at one pair boundary
when an old reverse cursor points to the echo. That offset remains valid; it
now excludes the compaction at the echo position and includes the command at
the earlier boundary position. Restart the traversal for an exact result.

The Claude summary must be the next complete logical provider record
after its boundary; intervening bookkeeping records may be skipped, but another
message or compaction boundary ends the association. When it crosses a page
boundary, resolve it through bounded adjacent provider context, never the
requested page alone; use at most the existing 16-MiB reverse window. An association
that cannot be resolved within that bound yields an empty summary consistently
in every direction.

An unfinished tail boundary is withheld until its summary
or following non-summary record is complete, so later pages do not return a
second version of the same event. A Claude boundary alone at EOF remains pending;
a complete Codex `compacted` record can be returned immediately even without
a summary. No full-transcript scan is required.

For Claude Code transcript boundaries with `compactMetadata.trigger: "manual"`,
publication waits further for a complete command echo or a following complete
logical record. Forward polling keeps its cursor at the pending boundary,
including while the echo lacks its final newline; when it completes, the command
precedes the compaction exactly once. A following non-echo message releases an
unpaired compaction. Card previews wait for the same manual association.

Automatic, missing and unknown triggers keep immediate summary publication at
idle EOF. Missing/unknown trigger metadata cannot promise causal order across a
late echo append. A live poller can receive the compaction before the echo, then
receive that same compaction id again when the echo arrives, without receiving
the command in that ongoing cursor traversal. Restart after the pair completes
to read the command followed by its compaction. Claude Code 2.1.296 boundaries
include the trigger metadata.

The transcript trigger shape was checked in Claude Code 2.1.296; the official
[SDK boundary reference](https://code.claude.com/docs/en/agent-sdk/typescript#sdkcompactboundarymessage)
also defines manual/auto triggers, under the stream adapter's separate
`compact_metadata` field.

## Notice recognition

The sole kind-to-label table is in
[Session notices v1](session-notices-v1.md#shared-kind-to-label-table).
The summary renderer and the history recognizer use that same table.
There must not be a separate consumer title list or private producer registry
in the daemon. Recognize only logical input or an already-admitted daemon
notice, after exact provider-wrapper handling, before human preview selection.

Order of classification: admitted compaction metadata, recognized runtime
command, valid notice envelope/existing daemon notice event, then the forms
below. Invalid or unsupported notice envelopes do not enter text recognition.
Never search a later line or a quoted region for a marker. A valid envelope's
unknown syntactically valid kind still has class `notice`; unknown unmarked
titles have no class. Recognizing text does not admit excluded provider rows.

| Form | Exact recognition rule | Kind used for table lookup |
| --- | --- | --- |
| Stored notice title | At byte zero, `※` + ASCII space + an exact current or stored title from the shared table + `：`; at least one body character follows. No indentation, ASCII colon, partial title or interior-line match. Subsequent body lines are preserved. | The matched table row |
| Fixed mailbox line | Entire input is `Mailbox: agent-session message inbox`. | `mailbox.wake` |
| Dated mailbox line | Entire input is `Mailbox: <UTC>; agent-session message inbox`; `<UTC>` is canonical second-precision `YYYY-MM-DDTHH:MM:SSZ` and a valid date. | `mailbox.wake` |
| Compact count line | Entire input is `Mailbox:` + ASCII space + space-separated unique `b`, `d`, `h`, `p`, `r`, `u` keys with `=<count>`; each count is canonical decimal 1..256; total at most 51 ASCII bytes. Key order is unrestricted. | `mailbox.wake` |
| Explained count line | Entire input is `Mailbox: <count> <category>[, <count> <category>…]; agent-session message inbox`; categories are unique and ordered blocker, decision, handoff, progress, report, uncategorized; each count is canonical decimal 1..256. | `mailbox.wake` |
| Earlier unread sentence | Entire input is `Coordination mailbox has unread messages; run agent-session message inbox.` or the same with ASCII space + `(newest queued <UTC>)` immediately before the semicolon. `<UTC>` follows the dated-line rule. | `mailbox.wake` |
| Earlier compaction continuation | Entire input is `壓縮完成；執行自己角色的 session-start，接著續做。` | `compact.resume` |
| Codex instruction frame without kinds | Entire input is `# AGENTS.md instructions for <title>` + LF LF + `<INSTRUCTIONS>` + LF + nonempty content + LF + `</INSTRUCTIONS>`, optionally final LF; `<title>` is a nonempty single line. | No envelope kind is invented |
| Codex environment frame without kinds | Entire input is `<environment_context>` + LF + nonempty content + LF + `</environment_context>`, optionally final LF. | No envelope kind is invented |

The context-frame rules apply only to Codex user frames lacking
`content_item_kinds`. They require complete, balanced outer delimiters with no
extra leading/trailing prose or nested outer block. Bodies remain literal text;
classification executes nothing. Other providers and partial wrappers have no
class from these rules.

The earlier bootstrap form containing a private producer sentinel is a known
gap: this public contract neither publishes nor recognizes it. It remains an
ordinary unclassified prompt. A public anchored `※ Dispatch start：` input and
a valid `dispatch.bootstrap` envelope are recognized. There is no configurable
private-form hook. This is an explicit scope decision for #3226, not permission
to guess at bootstrap-like prose.

These rules read already stored provider input and known plain-text outcomes;
they add no writer, send retry, unmarked delivery mode or alias. Classifying a
plain row changes only its class; it preserves all logical text bytes, including
line endings, indentation inside the body and trailing whitespace. A known
title without the exact shape, a quoted reminder, an unknown `※` title and
arbitrary `Monitoring: …` text have no class. Consumers never repeat these
recognizers themselves.

## Fixture pin and implementation acceptance

The JSON fixture contains a Draft 2020-12 schema for the class projection plus
language-neutral positive/negative rows, command tokens, compaction grouping,
summary encoding and paging expectations. The schema describes only these
additions to existing message objects; it is not a replacement history schema.
Fixture cases are design expectations, not claims of passing runtime tests.

After this design merges, consumers pin the fixture by the immutable merged
commit and path, and record that SHA in their own contract test. Target:
2026-11-09 for sympoies/salon#266, #267 and #268. Do not pin a moving branch.
No implementation slice starts before the design merges.

Implementation acceptance must exercise synthetic Claude and Codex transcripts
through the authenticated HTTP history boundary: every command with/without
arguments, identical repeated invocations, one event per compaction, summaries
on page boundaries, scalar truncation, notice positives and lookalikes,
metadata admission, plain-text byte preservation, and identical forward/reverse,
live and preview classification. Send-path tests compare runtime input bytes
before/after the display change. The notice slice covers every label-table row,
unknown kinds, 70/71 scalars, invalid single-line input and exact mailbox wake
bytes. Rendered UI acceptance is scheduled separately by the coordinator.
