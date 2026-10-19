#!/usr/bin/env python3
"""Enforce fragment PRs; admit only an exact fold on the maintenance PR path."""
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile

LOG = 'docs/devlog'


def git(*args):
    return subprocess.check_output(['git', *args], text=True).strip()


def snapshot(ref):
    files = {}
    for row in git('ls-tree', '-rz', ref, '--', LOG).split('\0'):
        if not row:
            continue
        metadata, path = row.split('\t')
        mode, kind, oid = metadata.split()
        if kind != 'blob' or mode != '100644':
            raise ValueError('devlog fold requires regular devlog files')
        files[path] = subprocess.check_output(['git', 'cat-file', 'blob', oid])
    return files


def working_snapshot():
    files = {}
    for path in Path(LOG).rglob('*'):
        if path.is_symlink():
            raise ValueError('devlog fold requires regular devlog files')
        if path.is_file():
            files[path.as_posix()] = path.read_bytes()
    return files


def is_fold_pr():
    event = os.environ.get('GITHUB_EVENT_NAME')
    branch = os.environ.get('GITHUB_HEAD_REF') or git('branch', '--show-current')
    if event != 'merge_group' and not re.fullmatch(r'chore/devlog-fold-[a-z0-9][a-z0-9-]*', branch):
        return False
    expected = 'chore(devlog): fold accepted fragments'
    parents = git('show', '-s', '--format=%P', 'HEAD').split()
    candidate = parents[1] if event in ('pull_request', 'merge_group') and len(parents) == 2 else 'HEAD'
    subject = git('show', '-s', '--format=%s', candidate)
    return subject == expected or (event == 'merge_group' and
                                  re.fullmatch(re.escape(expected) + r' \(#[1-9]\d*\)', subject) is not None)


def exact_fold(base, devlog, env):
    before = snapshot(base)
    after = working_snapshot()
    retained = {p: data for p, data in before.items() if p.startswith(LOG + '/pending/') and p in after}
    removed = {p for p in before if p.startswith(LOG + '/pending/')} - set(retained)
    if not removed:
        if after != before or snapshot('HEAD') != before or git('diff', '--cached', '--', LOG):
            raise ValueError('fold PR devlog changes must be an exact fold of baseline fragments')
        return
    # Replay just the retired fragments. This remains valid if review crosses
    # midnight: older retained fragments need not be folded by this maintenance PR.
    with tempfile.TemporaryDirectory(prefix='devlog-fold-check-') as directory:
        root = Path(directory)
        for path, data in before.items():
            if path not in retained:
                target = root / path
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_bytes(data)
        replay_env = {k: v for k, v in env.items() if not k.startswith('GIT_')}
        subprocess.run(['git', 'init', '-q', directory], env=replay_env, check=True)
        subprocess.run([devlog, 'fold'], cwd=root, env=replay_env, check=True)
        for path, data in retained.items():
            target = root / path
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(data)
        subprocess.run([devlog, 'index'], cwd=root, env=replay_env, check=True)
        expected = {p.relative_to(root).as_posix(): p.read_bytes()
                    for p in (root / LOG).rglob('*') if p.is_file()}
    if after != expected:
        raise ValueError('fold PR devlog changes differ from deterministic fold')
    # Check the staged and committed views too: an unstaged restoration cannot
    # hide a disallowed change that would otherwise be shipped.
    index_files = git('ls-files', '-s', '-z', '--', LOG).split('\0')
    index = {}
    for row in filter(None, index_files):
        metadata, path = row.split('\t')
        mode, oid, stage = metadata.split()
        if stage != '0' or mode != '100644':
            raise ValueError('fold PR devlog index must contain regular resolved files')
        index[path] = subprocess.check_output(['git', 'cat-file', 'blob', oid])
    for view in (snapshot('HEAD'), index):
        if view != before and view != expected:
            raise ValueError('staged or committed devlog differs from deterministic fold')


def main():
    env = dict(os.environ, DEVLOG_LAYOUT='fragments')
    devlog = shutil.which('devlog')
    if devlog is None:
        raise ValueError('devlog >=1.31.16 is required; build nils-devlog or install the released tools')
    base = os.environ.get('NILS_CLI_DEVLOG_BASE') or 'origin/main'
    branch = git('branch', '--show-current')
    event = os.environ.get('GITHUB_EVENT_NAME')
    if (event and event not in ('pull_request', 'merge_group')) or (not event and branch == 'main'):
        subprocess.run([devlog, 'check', '--base', 'HEAD'], env=env, check=True)
    else:
        baseline = git('merge-base', 'HEAD', base)
        if is_fold_pr():
            exact_fold(baseline, devlog, env)
            subprocess.run([devlog, 'check', '--base', baseline], env=env, check=True)
        else:
            subprocess.run([devlog, 'check', '--base', base, '--fragments-only'], env=env, check=True)


if __name__ == '__main__':
    try:
        main()
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        print(f'devlog policy: {error}', file=sys.stderr)
        sys.exit(1)
