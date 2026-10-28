# Test-change admission v1

Designated GitHub review admission checks the PR comparison for test changes.
It runs in `pr review-handoff check` (including `--dry-run`) and the existing
assigned delivery/merge publication gates. Unassigned PRs, GitLab, and the local
provider do not use this designated-review gate.

The check reads PR head, base, author and body, enumerates complete PR-file
metadata, inspects source patches for that comparison, and rereads the PR metadata. It requires complete evidence;
head/base/body drift, malformed or truncated hunks and binary source fail with
`test_changes_incomplete`. Deterministic bounds fail with the non-retryable
`test_changes_limit_exceeded` error; see [limits and recovery](#limits-and-recovery).
Ordinary comparisons discard non-source patches before backend stdout capture;
asset deletions and binaries are represented by file metadata and blob IDs.
Source paths include ordinary application files because they can contain inline
tests. The existing backend limits and redaction apply. No patch source text is included in diagnostics.

## Findings

Each finding names an exact repository-relative file and one of:

| Kind | Signal |
| --- | --- |
| `deleted-test` | Deleted test source file, rename of an old test source path, removed recognizable test signature or Rust test-module declaration |
| `assertion-change` | Removed or edited assertion, including potential weakening |
| `new-skip` | Added skip, ignore or expected-failure marker |
| `import-addition` | A declared new mapped file has no upstream counterpart; inspect its complete ordinary patch |
| `import-adaptation` | A mapped added file differs from its declared upstream blob; review the full adaptation against upstream |

The detector is a conservative diff heuristic, not a proof of logical
implication or a language parser. Assertion strengthening and test renames can
also be flagged. Identical assertions moved within a file and whitespace-only
assertion edits are canceled; new assertions alone are permitted.

Detection covers common Rust, Python, JavaScript/TypeScript, Go and Java test syntax,
including Rust inline `#[test]`, Python `test_` functions, JavaScript `it`/`test`,
Go `Test` functions, and common `assert`/`expect` calls. Skip signals include
Rust `#[ignore]`, `#[cfg(any())]`, `#![cfg(any())]` and `#[cfg_attr(..., ignore)]`,
pytest skip/skipif/xfail, unittest skip, JavaScript skip calls,
Go `Skip`, and Java `Ignore`/`Disabled`. Source extensions include Rust, Python,
JavaScript/TypeScript (`.js`, `.jsx`, `.ts`, `.tsx`, `.cjs`, `.mjs`, `.cts`, `.mts`),
Go, Java/Kotlin, shell, Ruby, C#, and C/C++.

A rename uses the old test source
path for its `deleted-test` declaration, even if the destination is still a test
path or has a disabled extension. Removing `#[cfg(test)] mod tests;` (including
an attribute on the preceding line) also requires a declaration.

Custom frameworks, dynamically generated tests, and changes outside the visible
hunk context can require manual review. Existing test-first, CI, review ledger,
identity and merge gates continue to apply.

## Author declarations and intentional-change allowlist

Before requesting a review, run the read-only preview against the provider head:

```sh
forge-cli pr review-handoff test-changes 123 --expected-head <full-head-sha>
```

It requires no assignment or existing review and writes no provider state. Text
output prints exact file/kind rows and `Test changes confirmed: sha256:<digest>`.
With `--format json`, `data` contains `number`, `head_sha`, `base_sha`, `findings`
(`path`/`kind` objects), `digest`, `confirmation` and nullable `provenance_import`.
The latter contains the upstream source, mappings, adaptation and addition paths, complete
file count, verified-file count, manifest digest, comparison digest and declaration
digest when the mode below is active. Reconstructed old sides also expose their
`merge_base` SHA and bind it into the confirmation digest. This is inspection evidence,
not review admission. It uses the same complete comparison, metadata re-read
and digest calculation as admission.

For every flagged file/kind, add a row under the level-two `Test changes`
heading in the PR body. Give a concrete reason and describe the retained
coverage or replacement owner. The reason must contain at least 12 characters.
The diagnostic lists the required rows.

```markdown
## Test changes

- tests/test_api.py | assertion-change | The replacement assertion protects the supported response contract.
- tests/test_old.py | deleted-test | The consolidated API test retains the obsolete case's contract.
```

The per-PR allowlist uses the same exact file/kind format with `allow`
before the path. Use it for intentional removals, quarantines, or false positives:

```markdown
## Test changes

- allow tests/test_old.py | deleted-test | Intentional removal; tests/test_api.py retains the response contract.
```

There is no file/kind pair or declaration row count cap. Globs and repository-wide
exemptions are not accepted.
Rows inside fenced examples do not count. An allowlisted finding still requires
reviewer confirmation. Missing declarations fail with `test_changes_undeclared`.
No command-line override bypasses this requirement.

Rerun the preview after editing declarations. Give its updated confirmation line
to the designated reviewer, who independently inspects the changes and includes
the line in the first passing review when satisfied.

## Provenance imports

A complete large initial import may declare immutable upstream provenance in
one PR-body section. This mode applies only to mapped **added** files; existing
modified, removed and renamed files always retain normal test-change inspection.
The upstream repository must be on the selected GitHub host. Its reads resolve
through the normal identity policy under that repository's own `api_read` scope,
then restore the PR command scope. The principal must be authorized to read the
upstream repository; PR authorization alone grants no upstream access.

````markdown
## Provenance import

```json
{
  "repository": "example/upstream-library",
  "commit": "0123456789abcdef0123456789abcdef01234567",
  "mappings": [{"destination": "vendor/library/", "source": "packages/library/"}],
  "adaptations": ["vendor/library/tests/api.test.ts"],
  "additions": ["vendor/library/NOTICE.txt"]
}
```
````

Use a full 40-character commit SHA. Directory prefixes end in `/`; an empty
`source` maps from the upstream root. Destination prefixes cannot overlap.
Paths must be normalized relative printable ASCII without whitespace, control,
quote, pipe or backslash bytes. Adaptations are exact destination paths, without
globs. Optional `additions` lists exact genuinely new paths under a mapping
that have no upstream counterpart, such as added license notices. These paths
must be distinct from adaptations. Duplicate sections, unknown JSON fields,
invalid mappings, duplicate paths and stale adaptation/addition entries are
refusals. A heading inside a fenced example does not activate the mode. Remove the section to use ordinary comparison admission.

The tool enumerates PR files in 100-file pages and requires each page's size
and unique paths to agree with the PR's `changed_files`. It reads the pinned
upstream commit and its complete recursive tree, checking object identities and
refusing `truncated` trees. Equal full Git blob IDs prove identical blob bytes;
no checkout, user-provided manifest or downloaded patch excerpt substitutes for
these provider reads. Only these verified additions may omit their inline patch.
Missing upstream blobs outside declared `additions`, unsupported upstream object
types and undeclared byte mismatches fail closed. Declared additions and unmapped
files remain in ordinary inspection; additions are never counted as verified
identical files. Every declared addition produces `import-addition` and needs a
`## Test changes` row and the same independent confirmation as other findings.

Verified additions are represented by a SHA-256 digest of sorted JSON tuples
`[destination_path, upstream_path, blob_sha]`. The confirmation digest binds that
manifest, complete provider file records, upstream source/mappings, full PR body,
and exact provider base/head. Text preview prints the verified count and manifest
digest; JSON exposes the same summary. The independent review report must include
the exact `Test changes confirmed: sha256:<digest>` line even when every file is
identical and findings are empty. The existing designated publication and ledger
gates bind the report to the assignment and exact head; there is no alternate
approval route.

Every declared byte adaptation produces `import-adaptation`, including changes
without recognizable test syntax. Give it a concrete `## Test changes` reason
and retained-coverage explanation, alongside any ordinary findings. The reviewer
must compare the adaptation with upstream: an added-file patch alone cannot
reveal assertions removed from the upstream version. Residual patches must be
readable and complete; both hunk lengths and total additions/deletions are checked.
Omitted whole hunks are refusals.

When a residual provider patch is absent, the
tool resolves the three-dot merge base from the immutable base/head comparison,
reads file metadata at that merge base and the exact head, and fetches the named
Git blobs, and verifies their sizes and full Git SHA-1 identities after Base64
decoding. Added/removed files use an empty missing side; renames read the old
path at the merge base and the new path at head. Complete UTF-8 text without NUL bytes is
compared locally with Git, with a 15-second process deadline and bounded output,
then inspected by the same detector. No local checkout content or mutable branch
ref is used. The existing backend response ceiling still applies to every blob
response. See the provider's [blob contract](https://docs.github.com/en/rest/git/blobs#get-a-blob).

A binary residual without a provider patch uses `test_changes_incomplete` with
`details.cause.code=residual_binary_patch_unavailable`, the exact path,
`retryable=false` and `next_action=correct_provenance_declaration`.
Keep upstream-identical binary additions mapped; binary patch omissions cannot
be declared away. The residual comparison retains the ordinary 4 MiB /
fewer-than-300-files bounds, including locally reconstructed patches.

The complete import must contain fewer than 3,000 changed paths (the provider
PR-files ceiling is not accepted as proof of completeness). Each provider response
also retains the backend's 8 MiB capture ceiling. The complete upstream recursive
tree must fit GitHub's tree limits (100,000 entries / 7 MB); splitting only the
downstream PR does not shrink that tree. Use ordinary smaller comparisons when
the import cannot satisfy these requirements; no partial-list or environment
bypass exists. See the provider's [PR-files](https://docs.github.com/en/rest/pulls/pulls#list-pull-requests-files)
and [tree](https://docs.github.com/en/rest/git/trees#get-a-tree) contracts.

Provenance refusals use exit 65 and typed recovery hints:
`provenance_import_invalid` and `provenance_import_mismatch` require
`correct_provenance_declaration`; `provenance_import_incomplete` requires
`retry_complete_upstream`. Backend failures preserve their typed cause and recovery
metadata, including identity-policy and provider refusals. Incomplete PR pages/patches remain
`test_changes_incomplete`. Re-run preview after correcting the source, mappings,
adaptations or declarations; stale confirmations cannot admit a changed import.

## Limits and recovery

Deterministic bounds never become retryable just because provider evidence is
fetched again. They use exit 65, `test_changes_limit_exceeded`,
`details.retryable=false` and `details.next_action=split_comparison_or_rename_paths`.
The diagnostic names the bound and recovery; oversized ordinary source patches
also name the affected paths. There is no silent bypass.

| Limit name | Accepted comparison | Recovery |
| --- | --- | --- |
| `file_count` | Fewer than 300 source paths in ordinary inspection; fewer than 300 residual files for provenance imports | Split the PR into smaller comparisons |
| `diff_bytes` | At most 4 MiB (4,194,304 UTF-8 bytes) of ordinary source patches or provenance residual patches | Split the PR into smaller comparisons |
| `path_bytes` | Unquoted printable ASCII paths without whitespace, control, quote or pipe bytes | Rename affected paths to supported names, splitting the PR if needed |

Complete PR-file enumeration enforces `provider_file_count` below 3,000 in both
modes. Ordinary inspection reads 100-file metadata pages with patches removed,
then requests a bounded single-file patch only when either the old or new path
has a supported source extension. Missing source patches are reconstructed from
verified immutable blobs using the same procedure as residual provenance patches.
File metadata is rechecked before accepting each patch; the confirmation binds
all enumerated file metadata, inspected patches and any resolved merge base.
Unrelated assets consume neither the 4 MiB patch budget nor the 300-source-path
budget. With a provenance declaration, `file_count` and `diff_bytes` still apply
to the residual preview. A large
initial import can use [verified provenance](#provenance-imports) instead of
splitting identical upstream additions solely to fit the preview.

Both Git-quoted headers (including escaped non-ASCII paths) and unsupported
`---`/`+++` or rename paths use `path_bytes`. A retry cannot clear these bounds;
use supported names before the comparison base and rerun the preview. Renaming
within a PR still exposes the old path; if the base already contains unsupported
names, request an explicit maintainer recovery rather than retrying or bypassing
admission. Splitting alone cannot repair unsupported base paths. Missing or
truncated evidence remains `test_changes_incomplete` with `retryable=true`.

## Reviewer confirmation

After inspecting the declared and allowlisted changes, the designated reviewer
includes the diagnostic's exact line in their published passing review report:

```text
Test changes confirmed: sha256:<digest>
```

The digest binds the immutable head/base comparison and the declaration section.
A changed head, base, diff or declaration invalidates confirmation. The ordinary
handoff gate selects and authenticates the review report first; a PR-body claim,
mailbox message, stale review, superseded report or another review author's
confirmation does not count. A PR author cannot confirm their own test changes.
A missing or stale confirmation fails with `test_changes_unconfirmed` and the
exact line the reviewer must publish. A carried source review across a base
sync must also satisfy the current comparison's confirmation.

Errors use exit 65 and typed `details.next_action` values:
`declare_test_changes`, `request_reviewer_confirmation`, and
`retry_complete_comparison`, plus the deterministic recovery above. Only incomplete comparison evidence is marked
retryable. Readiness checks remain read-only; they never add declarations or
publish confirmation automatically.
