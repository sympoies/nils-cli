# Test-change admission v1

Designated GitHub review admission checks the PR comparison for test changes.
It runs in `pr review-handoff check` (including `--dry-run`) and the existing
assigned delivery/merge publication gates. Unassigned PRs, GitLab, and the local
provider do not use this designated-review gate.

The check reads PR head, base, author and body, requests the immutable
`base...head` diff, and rereads that metadata. It requires complete evidence;
head/base/body drift, malformed or truncated hunks, binary source, unsupported
quoted paths, comparisons of 300 or more files, or diffs over 4 MiB fail with
`test_changes_incomplete`. Binary assets are ignored. The existing backend
limits and redaction apply. No patch source text is included in diagnostics.

## Findings

Each finding names an exact repository-relative file and one of:

| Kind | Signal |
| --- | --- |
| `deleted-test` | Deleted test source file or removed recognizable test signature |
| `assertion-change` | Removed or edited assertion, including potential weakening |
| `new-skip` | Added skip, ignore or expected-failure marker |

The detector is a conservative diff heuristic, not a proof of logical
implication or a language parser. Assertion strengthening and test renames can
also be flagged. Identical assertions moved within a file and whitespace-only
assertion edits are canceled; new assertions alone are permitted. Detection
covers common Rust, Python, JavaScript/TypeScript, Go and Java test syntax,
including Rust inline `#[test]`, Python `test_` functions, JavaScript `it`/`test`,
Go `Test` functions, and common `assert`/`expect` calls. Skip signals include
Rust `#[ignore]`, pytest skip/skipif/xfail, unittest skip, JavaScript skip calls,
Go `Skip`, and Java `Ignore`/`Disabled`. Source extensions include Rust, Python,
JavaScript/TypeScript, Go, Java/Kotlin, shell, Ruby, C#, and C/C++.

Custom frameworks, dynamically generated tests, and changes outside the visible
hunk context can require manual review. Existing test-first, CI, review ledger,
identity and merge gates continue to apply.

## Author declarations and intentional-change allowlist

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
`retry_complete_comparison`. Only incomplete comparison evidence is marked
retryable. Readiness checks remain read-only; they never add declarations or
publish confirmation automatically.
