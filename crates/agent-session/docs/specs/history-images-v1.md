# History images v1

History message pages add optional `images: [{ id: string }]`. There are at most
16 descriptors per message. The opaque id is at most 96 ASCII bytes and binds a
provider transcript segment, byte offset, image index and source-row digest. It
contains no file path, external URL or image bytes. Text and paging fields keep
their existing meaning. Image-only messages have empty `text`; they are visible
without becoming human text evidence for retitling.

Supported source forms are Codex `input_image` / `output_image` content blocks,
Claude `image` blocks (base64, URL or file source), nested tool-result images,
and Markdown image references in visible message text. A source that cannot be
read safely still gets a descriptor, so clients can display an unavailable
placeholder. Injected/meta context remains excluded. Other providers retain
their existing text-only contract.

## Authenticated read

`GET /history/sessions/{history_id}/images/{image_id}` requires the server bearer,
including on loopback. Consumers proxy this route through their normal
session/history ownership boundary; they must not expose a public file URL.
The caller supplies only ids minted by the daemon. The daemon resolves the
selected transcript again, checks the row boundary and digest, and selects the
referenced image from that row. A changed/deleted row cannot silently replace
an image behind an old id. No external HTTP request is performed.

Success is a raw raster body with `Content-Type: image/png`, `image/jpeg`,
`image/gif` or `image/webp`, `Cache-Control: private, no-store` and
`X-Content-Type-Options: nosniff`. The maximum decoded/file size is 8 MiB;
provider records remain bounded by 16 MiB. MIME signatures must agree with an
embedded source's declared type. SVG and other active/unsupported formats are
unavailable. Clients should also replace decode failures with an unavailable
placeholder rather than a broken-image icon.

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
`<state>/sessions/<id>/attachments/`. Archive/delete logically removes the whole
session directory into a private tombstone and attempts recursive cleanup;
physical cleanup can remain pending. Provider transcripts and generated files
live in provider/tool-owned storage. This API adds no image cache, copy or TTL,
and does not guarantee that a referenced file remains available after session
removal or external cleanup.
