#!/usr/bin/env python3
"""Report surviving names and metadata without reading leaked file contents."""
import datetime
import fnmatch
import json
import os
from pathlib import Path
import stat
import sys

root, index = map(Path, sys.argv[1:3])
patterns = sys.argv[3:]
leaked = False


def report(path):
    metadata = path.lstat()
    mode = metadata.st_mode
    kind = ('directory' if stat.S_ISDIR(mode) else 'file' if stat.S_ISREG(mode)
            else 'symlink' if stat.S_ISLNK(mode) else 'other')
    mtime = datetime.datetime.fromtimestamp(metadata.st_mtime, datetime.timezone.utc).isoformat()
    print(f'  {kind} {json.dumps(str(path.relative_to(root)))} mtime={mtime}', file=sys.stderr)
    if stat.S_ISDIR(mode):
        for child in sorted(path.iterdir()):
            report(child)


for entry in sorted(root.iterdir()):
    identity = index / entry.name
    if identity.is_file() and entry.is_dir() and not entry.is_symlink():
        children = list(entry.iterdir())
        survivors = [child for child in children if not any(fnmatch.fnmatchcase(child.name, p) for p in patterns)]
        if not survivors:
            continue
        # Only the test binary name and nextest test name are stored in this index.
        binary, test = identity.read_text().splitlines()[:2]
        print(f'tempdir-leak-probe: test={json.dumps(test)} binary={json.dumps(binary)}', file=sys.stderr)
        for child in sorted(survivors):
            report(child)
        leaked = True
    elif not any(fnmatch.fnmatchcase(entry.name, p) for p in patterns):
        print('tempdir-leak-probe: test=unknown', file=sys.stderr)
        report(entry)
        leaked = True
if leaked:
    print('tempdir-leak-probe: test run left entries behind; see docs/specs/test-temp-directory-policy.md', file=sys.stderr)
    sys.exit(1)
