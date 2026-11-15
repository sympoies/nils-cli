#!/usr/bin/env python3
"""Report surviving names and metadata without reading leaked file contents."""
import datetime
import fnmatch
import json
from pathlib import Path
import stat
import sys

root, index = map(Path, sys.argv[1:3])
patterns = sys.argv[3:]
leaked = False
remaining = 8192
entries = 0
truncated = False


def emit(line):
    global remaining, truncated
    if len(line) + 1 > remaining:
        truncated = True
        return False
    print(line, file=sys.stderr)
    remaining -= len(line) + 1
    return True


def report(path, depth=0):
    global entries, truncated
    if entries >= 200 or depth > 32:
        truncated = True
        return
    entries += 1
    metadata = path.lstat()
    mode = metadata.st_mode
    kind = ('directory' if stat.S_ISDIR(mode) else 'file' if stat.S_ISREG(mode)
            else 'symlink' if stat.S_ISLNK(mode) else 'other')
    mtime = datetime.datetime.fromtimestamp(metadata.st_mtime, datetime.timezone.utc).isoformat()
    if not emit(f'  {kind} {json.dumps(str(path.relative_to(root)))} mtime={mtime}'):
        return
    if stat.S_ISDIR(mode):
        for child in path.iterdir():
            report(child, depth + 1)
            if truncated:
                return


for entry in root.iterdir():
    identity = index / entry.name
    if identity.is_file() and entry.is_dir() and not entry.is_symlink():
        announced = False
        for child in entry.iterdir():
            if any(fnmatch.fnmatchcase(child.name, p) for p in patterns):
                continue
            leaked = True
            if not announced:
                binary, test = identity.read_text().splitlines()[:2]
                announced = True
                if not emit(f'tempdir-leak-probe: test={json.dumps(test)} binary={json.dumps(binary)}'):
                    break
            report(child)
            if truncated:
                break
    elif not any(fnmatch.fnmatchcase(entry.name, p) for p in patterns):
        leaked = True
        if emit('tempdir-leak-probe: test=unknown'):
            report(entry)
    if truncated:
        break
if truncated:
    print('tempdir-leak-probe: metadata truncated', file=sys.stderr)
if leaked:
    print('tempdir-leak-probe: test run left entries behind; see docs/specs/test-temp-directory-policy.md', file=sys.stderr)
    sys.exit(1)
