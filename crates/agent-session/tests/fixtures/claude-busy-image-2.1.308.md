# Busy image queue recording

The adjacent JSONL fixture preserves the shape recorded by Claude Code 2.1.308
in a disposable session with a synthetic PNG pasted as a quoted path while a
foreground tool was running. Session identifiers, UUIDs, paths, timestamps and
image bytes are replaced with generic fixture values. No conversation content
is retained beyond the provider's image label.

Observed order, measured from the paste:

- Around 0.51 seconds: `queue-operation` / `enqueue`, with `content: "[Image #2]"`.
- Around 40.78 seconds, after the tool returned: `attachment` / `queued_command`,
  with text and image content blocks. Its timestamp still described the original
  input time, so the timestamp did not reveal the delayed file append.

The structured request contained the quoted image path; the composer and both
queue records contained the image label. A regression must preserve that rewrite
and acknowledge the early enqueue without waiting for the later attachment.
