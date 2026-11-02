#!/usr/bin/env python3
"""Reject committed credentials and absolute user-home paths in tracked files.

Cargo-free and platform-independent: scan every tracked text file for secret
shapes (the nils-scrub v1 token patterns) and for `/home/<user>` or
`/Users/<user>` paths. The audit holds no word list of private terms; it names
only public placeholder users and exact exempt paths, with reasons, in
scripts/ci/public-boundary-allowlist.tsv (kind<TAB>value<TAB>reason):

  credential-file  exact tracked path that may carry synthetic credential shapes
  home-user        user component that is a placeholder or a system directory
"""
import argparse
from pathlib import Path
import re
import subprocess
import sys

ALLOWLIST = 'scripts/ci/public-boundary-allowlist.tsv'
KINDS = {'credential-file', 'home-user'}

# Same v1 token shapes as crates/nils-scrub (generic key=value is excluded: it
# matches ordinary configuration prose). A token never continues an identifier.
START = r'(?<![A-Za-z0-9_])'
CREDENTIALS = (
    ('github token',
     re.compile(START + r'(?:ghs_[0-9]+_[A-Za-z0-9_.-]{28,}|gh[opusr]_[A-Za-z0-9_]{36,})')),
    ('gitlab token', re.compile(START + r'glpat-[A-Za-z0-9_-]{20,}')),
    ('bitbucket app password', re.compile(START + r'ATBB[A-Za-z0-9]{16,}')),
    ('aws access key id',
     re.compile(START + r'(?:AKIA|ASIA|AGPA|AIDA|AROA|AIPA|ANPA|ANVA|A3T[A-Z0-9])[A-Z0-9]{16}')),
    ('private key block', re.compile(r'-----BEGIN (?:[A-Z ]+ )?PRIVATE KEY-----')),
)
# An absolute path starts here: not glued to a longer path or a variable.
HOME = re.compile(r'(?<![A-Za-z0-9_.~}$-])(/(?:home|Users))/([A-Za-z0-9](?:[A-Za-z0-9_.-]*[A-Za-z0-9_])?)')


def load_allowlist(root):
    exempt_files, home_users, errors = set(), set(), []
    path = root / ALLOWLIST
    for number, line in enumerate(path.read_text().splitlines(), 1):
        if not line.strip() or line.startswith('#'):
            continue
        row = line.split('\t')
        if len(row) != 3 or row[0] not in KINDS or not row[1].strip() or not row[2].strip():
            errors.append(f'{ALLOWLIST}:{number}: expected <{"|".join(sorted(KINDS))}>'
                          '<TAB>value<TAB>reason')
        elif row[0] == 'credential-file':
            exempt_files.add(row[1])
        else:
            home_users.add(row[1])
    return exempt_files, home_users, errors


def tracked_files(root):
    listing = subprocess.run(['git', 'ls-files', '-z'], cwd=root, check=True,
                             capture_output=True).stdout
    return [name for name in listing.decode().split('\0') if name]


def findings(root, exempt_files, home_users):
    for name in tracked_files(root):
        data = (root / name).read_bytes()
        if b'\0' in data:
            continue
        for number, line in enumerate(data.decode('utf-8', 'replace').splitlines(), 1):
            if name not in exempt_files:
                for label, pattern in CREDENTIALS:
                    if pattern.search(line):
                        yield f'{name}:{number}: committed credential ({label})'
            for match in HOME.finditer(line):
                if match[2] not in home_users:
                    # The user component may itself be a credential: report the prefix only.
                    yield f'{name}:{number}: absolute home path {match[1]}/<user>'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', type=Path, default=Path(__file__).resolve().parents[2])
    args = parser.parse_args()
    exempt_files, home_users, errors = load_allowlist(args.root)
    if errors:
        print('\n'.join(errors), file=sys.stderr)
        return 2
    results = list(findings(args.root, exempt_files, home_users))
    for result in results:
        print(result)
    print(f'{"FAIL" if results else "PASS"}: public boundary audit ({len(results)} findings)')
    return 1 if results else 0


if __name__ == '__main__':
    raise SystemExit(main())
