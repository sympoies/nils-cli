# History images v1

History message pages add optional `images` descriptors retaining their row-bound
`id`. Each descriptor may also carry `origin` and `attachment_ref`; the same
optional values appear on `parts=v1` image parts, whose `ref` remains the image-read
selector. There are at most 16 descriptors per message. The opaque id is at most 96 ASCII bytes and binds a
provider transcript segment, byte offset, source selector and source-row digest. It
contains no file path, external URL or image bytes. Text and paging fields keep
their existing meaning. Image-only messages have empty `text`; they are visible
without becoming human text evidence for retitling.

Supported source forms are Codex `input_image` / `output_image` content blocks,
Claude `image` blocks (base64, URL or file source), nested tool-result images,
and Markdown image references in visible message text. Image-bearing Claude tool-result
rows and Codex tool-output rows use the `assistant` role, including in incremental
history; they are not human prompt evidence. Codex user `input_text` and Claude
Code 2.1.312 user text (including a single-string quoted path, newline and caption)
also recognize single- or double-quoted absolute paths to raster images in the
same managed session's upload directory. Claude Code 2.1.316 human
`attachment/queued_command` prompt strings use the same quoted-upload rules.
Queued string and structured prompt images preserve caption/image order in
`parts=v1`; repeated `rendered` text is not a second image source.
These upload references become ordered image parts, leaving the surrounding
caption text visible. Other quoted paths
remain text: they must pass the attachment identity, path, size, extension and
raster signature checks before a descriptor is emitted. Projection does not
change the transcript or deliver native images to the model.

A structured or Markdown source that cannot be read safely still gets a descriptor, so clients can display an unavailable
placeholder. Injected/meta context remains excluded. Other providers retain
their existing text-only contract.

## Provenance and upload correlation

`origin` is `user_attachment` or `tool_result` only when recognized provider
structure proves that origin for the individual image. Nested tool-result images
keep their tool origin even inside a user envelope containing user images.
Ordinary assistant images and unknown provenance omit `origin`.

`attachment_ref` is an opaque same-history-session upload identity matching
`^[A-Za-z0-9._~-]{1,128}$`. It is emitted only after the existing managed attachment
ownership and raster checks succeed. The user image and a later tool read of that
upload share the reference while retaining different row-bound selectors.
Independent uploads with identical display names or bytes have different
references. References remain stable across ordinary paging and repeated reads
of the retained upload; they neither compare image content nor expose paths.
A proven tool image without managed upload proof emits its origin and omits the
reference. Working-directory read permission alone does not prove upload ownership.

The reference is correlation metadata, never read authority or a download target.
A consumer can collapse a tool reread only when its proven matching user attachment
is already displayed in the current history scope; otherwise the tool image remains
visible. This adds no image store, copy, arbitrary path scan or authentication surface.

## Authenticated read

`GET /history/sessions/{history_id}/images/{image_id}` requires the server bearer,
including on loopback. Consumers proxy this route through their normal
session/history ownership boundary; they must not expose a public file URL.
The caller supplies only ids minted by the daemon. The daemon resolves the
selected transcript again, checks the row boundary and digest, and selects the
referenced image from that row. Quoted uploads use their original block and
quoted-text position so removing an upload cannot shift an existing descriptor
to a neighboring image. The upload boundary is checked again on every read,
even when the working directory contains the attachment directory.
A changed/deleted row cannot silently replace
an image behind an old id. No external HTTP request is performed.

Success is a raw raster body with `Content-Type: image/png`, `image/jpeg`,
`image/gif` or `image/webp`, `Cache-Control: private, no-store` and
`X-Content-Type-Options: nosniff`. The maximum decoded/file size is 8 MiB;
provider records remain bounded by 16 MiB. MIME signatures must agree with an
embedded source's declared type. SVG and other active/unsupported formats are
unavailable. Clients should also replace decode failures with an unavailable
placeholder rather than a broken-image icon.

When an original filename is known, success also includes
`Content-Disposition: attachment; filename*=UTF-8''<percent-encoded-name>`.
Names contain only the leaf component, with control characters removed; UTF-8
bytes are percent encoded. Uploads retain bounded original display-name metadata
separately from their compatible sanitized on-disk names. That metadata is
session-owned and read through the same no-follow boundary. File-backed sources
outside managed uploads use their leaf name; embedded images and older uploads
without original-name metadata omit the header. Image bytes, MIME, length and
authorization are unchanged.

Errors use the ordinary JSON `ok: false` envelope:

| Status | Code | Display |
| --- | --- | --- |
| 401 | `unauthorized` | Existing authentication flow |
| 404 | `history-image-deleted` | Image deleted |
| 422 | `history-image-unavailable` | Image unavailable |

Missing history, transcript, referenced file, or stale row ids are deleted.
A managed attachment whose session has no matching live provider identity or
matching archive original session id is also deleted (404).
Malformed ids, remote URLs, unsupported data, excessive size, forbidden paths,
symlinks and unreadable/invalid images are unavailable. Infrastructure failures
remain ordinary server errors; clients can show unavailable and allow retry.
For a local file outside both allowed roots, the 422 envelope retains
`history-image-unavailable` and adds `error.reason: "outside-allowed-roots"` with
a message explaining the working-directory/owned-attachment boundary. It does not
include the refused path. Consumers can display this reason with the image
basename from the original Markdown instead of an unexplained placeholder.
These states are observations at read time, not promises of permanent storage.

## File boundary and retention

File references resolve only under the transcript's recorded working directory
or a daemon-owned session attachment directory. Attachment reads additionally
verify the live record's provider conversation identity or the archive's
original session id. A broad recorded working directory broadens the file
boundary; authenticated consumers retain their own history ownership checks.
Components containing `..` are refused. Reads walk below
the trusted root with descriptor-relative `O_NOFOLLOW` opens, including parent
directories; FIFOs/devices and other non-regular files are refused.

Uploads live on the upstream session machine under
`<state>/sessions/<id>/attachments/`, with display-name metadata under the sibling
`attachment-names/` directory. Archive/delete logically removes the whole
session directory into a private tombstone and attempts recursive cleanup;
physical cleanup can remain pending. Provider transcripts and generated files
live in provider/tool-owned storage. This API adds no image cache, copy or TTL,
and does not guarantee that a referenced file remains available after session
removal or external cleanup.
