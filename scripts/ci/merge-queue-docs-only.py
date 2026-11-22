"""Identify prose-only diffs between the exact merge-group base and candidate."""
import re
import subprocess
import sys


def is_prose(path):
    if path in {b'AGENTS.md', b'BINARY_DEPENDENCIES.md', b'DEVELOPMENT.md', b'README.md',
                b'docs/README.md'}:
        return True
    # Crate docs, specs and fixtures can be consumed by tests. This runbook
    # is also embedded by the review_specialists integration test.
    if path == b'docs/runbooks/review-specialists-primitive.md':
        return False
    parts = tuple(path.split(b'/'))
    return path.endswith(b'.md') and (
        (len(parts) == 3 and parts[:2] in {(b'docs', b'runbooks'), (b'docs', b'plans'), (b'docs', b'devlog')})
        or (len(parts) == 4 and parts[:3] == (b'docs', b'devlog', b'pending'))
    )


def docs_only(base, head):
    if not all(re.fullmatch(r'[0-9a-f]{40}', sha) for sha in (base, head)):
        return False
    # With no rename detection, the removal and addition are both classified.
    diff = subprocess.check_output([
        'git', '--no-replace-objects', 'diff', '--raw', '-z', '--no-renames', base, head, '--',
    ])
    fields = diff.split(b'\0')
    if not diff or fields[-1] or len(fields) % 2 != 1:
        return False
    for metadata, path in zip(fields[0:-1:2], fields[1:-1:2]):
        old_mode, new_mode, *_ = metadata.removeprefix(b':').split()
        if {old_mode, new_mode} - {b'000000', b'100644'} or not is_prose(path):
            return False
    return True


if __name__ == '__main__':
    print('true' if docs_only(*sys.argv[1:]) else 'false')
