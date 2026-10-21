#!/usr/bin/env python3
"""Detect executables replaced in a cargo target directory during a gate run.

Lanes that share one `CARGO_TARGET_DIR` can overwrite the binaries a running
gate is testing, so a product test fails against code from another worktree
(sympoies/nils-cli#2697). `snapshot` records the executables a gate built;
`verify` fails with a "shared target replaced" error when any was rewritten or
removed since.
"""
import argparse
import json
import os
from pathlib import Path
import stat
import sys

PROFILE_DIR = 'debug'
SCAN_DIRS = ('', 'deps')


def executables(target_dir):
    """Map each built executable to its size and mtime, relative to the target dir."""
    found = {}
    for scan in SCAN_DIRS:
        directory = target_dir / PROFILE_DIR / scan
        if not directory.is_dir():
            continue
        for entry in os.scandir(directory):
            # Build artifacts with an extension (.d, .rlib, .rmeta, .so, ...) are not
            # executables under test.
            if '.' in entry.name or not entry.is_file(follow_symlinks=False):
                continue
            info = entry.stat(follow_symlinks=False)
            if info.st_mode & (stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH):
                relative = Path(entry.path).relative_to(target_dir).as_posix()
                found[relative] = [info.st_size, info.st_mtime_ns]
    return found


def snapshot(args):
    target_dir = Path(args.target_dir)
    record = {'target_dir': str(target_dir), 'executables': executables(target_dir)}
    Path(args.out).write_text(json.dumps(record, sort_keys=True))
    return 0


def verify(args):
    record = json.loads(Path(args.snapshot).read_text())
    target_dir = Path(record['target_dir'])
    current = executables(target_dir)
    changed = sorted(
        path for path, identity in record['executables'].items()
        if current.get(path) != identity)
    if not changed:
        return 0
    print(f'error: shared target replaced during the gate: {target_dir}', file=sys.stderr)
    print('Executables built by this gate were rewritten or removed, so test results '
          'may not reflect this head. Rerun with a CARGO_TARGET_DIR no other lane '
          'builds into.', file=sys.stderr)
    for path in changed:
        state = 'removed' if path not in current else 'replaced'
        print(f'  {state}: {path}', file=sys.stderr)
    return 1


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest='command', required=True)
    snap = commands.add_parser('snapshot', help='record the executables built so far')
    snap.add_argument('--target-dir', required=True)
    snap.add_argument('--out', required=True)
    snap.set_defaults(func=snapshot)
    check = commands.add_parser('verify', help='fail if a snapshotted executable changed')
    check.add_argument('snapshot')
    check.set_defaults(func=verify)
    args = parser.parse_args()
    sys.exit(args.func(args))


if __name__ == '__main__':
    main()
