# Test-change admission v1

Designated GitHub review admission checks the PR comparison for test changes.
It runs in `pr review-handoff check` (including `--dry-run`) and the existing
assigned delivery/merge publication gates. Unassigned PRs, GitLab, and the local
provider do not use this designated-review gate.

The check reads PR head, base, author and body, requests the immutable
`base...head` diff, and rereads that metadata. It requires complete evidence;
head/base/body drift, malformed or truncated hunks and binary source fail with
`test_changes_incomplete`. Deterministic bounds fail with the non-retryable
`test_changes_limit_exceeded` error; see [limits and recovery](#limits-and-recovery).
Binary assets produce no findings but count toward comparison bounds. The existing backend
limits and redaction apply. No patch source text is included in diagnostics.

## Findings

Each finding names an exact repository-relative file and one of:

| Kind | Signal |
| --- | --- |
| `deleted-test` | Deleted test source file, rename of an old test source path, removed recognizable test signature or Rust test-module declaration |
| `assertion-change` | Removed or edited assertion, including potential weakening |
| `new-skip` | Added skip, ignore or expected-failure marker |

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
(`path`/`kind` objects), `digest` and `confirmation`. This is inspection evidence,
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

The small per-PR allowlist uses the same exact file/kind format with `allow`
before the path. Use it for intentional removals, quarantines, or false positives:

```markdown
## Test changes

- allow tests/test_old.py | deleted-test | Intentional removal; tests/test_api.py retains the response contract.
```

There are at most 32 accepted rows, with no globs or repository-wide exemptions.
Rows inside fenced examples do not count. An allowlisted finding still requires
reviewer confirmation. Missing declarations fail with `test_changes_undeclared`.
No command-line override bypasses this requirement.

Rerun the preview after editing declarations. Give its updated confirmation line
to the designated reviewer, who independently inspects the changes and includes
the line in the first passing review when satisfied.

## Limits and recovery

Deterministic bounds never become retryable just because provider evidence is
fetched again. They use exit 65, `test_changes_limit_exceeded`,
`details.retryable=false` and `details.next_action=split_comparison_or_rename_paths`.
The diagnostic names the bound and recovery; there is no silent bypass.

| Limit name | Accepted comparison | Recovery |
| --- | --- | --- |
| `file_count` | Fewer than 300 files, including binary assets | Split the PR into smaller comparisons |
| `diff_bytes` | At most 4 MiB (4,194,304 UTF-8 bytes) | Split the PR into smaller comparisons |
| `declaration_pairs` | At most 32 distinct finding file/kind pairs and at most 32 accepted declaration rows | Split the PR; consolidate duplicate declaration rows if those alone exceed the bound |
| `path_bytes` | Unquoted printable ASCII paths without whitespace, control, quote or pipe bytes | Rename affected paths to supported names, splitting the PR if needed |

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
