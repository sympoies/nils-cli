#!/usr/bin/env python3
"""Fail when a workflow or helper names a Rust toolchain other than the pinned one.

`rust-toolchain.toml` is the single source of truth and must name an exact
version. Every workflow toolchain step, `rustup`/`cargo +channel` command,
toolchain environment override and local helper under `scripts/` must name that
same version, never a floating channel (`stable`, `beta`, `nightly`) or another
release, and every workflow job that runs cargo must install the pinned
toolchain explicitly before its first cargo step (sympoies/nils-cli#3624).
"""
import argparse
from pathlib import Path
import re
import sys

SETUP_ACTION = 'dtolnay/rust-toolchain'
EXACT_VERSION = re.compile(r'^\d+\.\d+\.\d+$')
CHANNEL_LINE = re.compile(r'^\s*channel\s*=\s*"([^"]*)"')
# Each pattern captures the toolchain a command or setting selects; the first
# group is the value, absent when the command names none.
LINE_PATTERNS = (
    re.compile(r'\brustup\s+(?:default|override\s+set|toolchain\s+install|run)\s+(?:-\S+\s+)*(\S+)'),
    re.compile(r'\brustup\s+update\b(?:\s+(?:-\S+\s+)*([^\s-]\S*))?'),
    re.compile(r'--default-toolchain[\s=]+(\S+)'),
    re.compile(r'\b(?:cargo|rustc)\s+\+(\S+)'),
    re.compile(r'\bRUSTUP_TOOLCHAIN\s*[:=]\s*(\S+)'),
)
TOOLCHAIN_INPUT = re.compile(r'^\s*toolchain:\s*(\S+)')
USES = re.compile(r'^\s*(?:-\s+)?uses:\s*(\S+)')
RUST_ACTION_NAME = re.compile(r'rust-toolchain|setup-rust|rustup', re.IGNORECASE)
CARGO_WORD = re.compile(r'(?<![\w./+-])(?:cargo|rustc)(?![\w-])')
RUN_KEY = re.compile(r'^(\s*)(-\s+)?run:\s*(.*)$')
HELPER_SUFFIXES = {'.sh', '.bash', '.py', '.cjs', '.js'}
HELPER_EXCLUDED_DIRS = ('scripts/ci/tests/',)


def clean(token):
    return token.strip('"\'`,;')


def is_expected(token, pin):
    """A literal must be the pin; a shell/expression reference is resolved elsewhere."""
    token = clean(token)
    return token == pin or token.startswith(('$', '${{'))


def toolchain_pin(root, problems):
    path = root / 'rust-toolchain.toml'
    if not path.is_file():
        problems.append('rust-toolchain.toml: missing')
        return None
    for number, line in enumerate(path.read_text().splitlines(), 1):
        match = CHANNEL_LINE.match(line)
        if match:
            if not EXACT_VERSION.match(match.group(1)):
                problems.append(
                    f'rust-toolchain.toml:{number}: channel "{match.group(1)}" is not an exact '
                    'X.Y.Z version')
                return None
            return match.group(1)
    problems.append('rust-toolchain.toml: no exact toolchain channel')
    return None


def scan_lines(relative, text, pin, problems, workflow):
    for number, line in enumerate(text.splitlines(), 1):
        if line.lstrip().startswith('#'):
            continue
        for pattern in LINE_PATTERNS:
            for match in pattern.finditer(line):
                value = match.group(1) if match.groups() else None
                if value is None or not is_expected(value, pin):
                    problems.append(f'{relative}:{number}: {line.strip()} (expected {pin})')
        if not workflow:
            continue
        match = TOOLCHAIN_INPUT.match(line)
        if match and not is_expected(match.group(1), pin):
            problems.append(f'{relative}:{number}: {line.strip()} (expected {pin})')
        match = USES.match(line)
        if match:
            name, _, ref = match.group(1).partition('@')
            if name == SETUP_ACTION:
                if ref != pin:
                    problems.append(f'{relative}:{number}: {match.group(1)} (expected {SETUP_ACTION}@{pin})')
            elif RUST_ACTION_NAME.search(name):
                problems.append(
                    f'{relative}:{number}: {match.group(1)} (toolchain setup must use {SETUP_ACTION}@{pin})')


def jobs(text):
    """Map each job name to its lines (1-based number, text)."""
    found, current, in_jobs = {}, None, False
    for number, line in enumerate(text.splitlines(), 1):
        if re.match(r'^jobs:\s*$', line):
            in_jobs = True
        elif in_jobs and re.match(r'^\S', line):
            in_jobs = False
        header = re.match(r'^  ([A-Za-z0-9_-]+):\s*$', line)
        if in_jobs and header:
            current = header.group(1)
            found[current] = []
        elif in_jobs and current:
            found[current].append((number, line))
    return found


def steps(lines):
    """Split a job's lines into its steps, each a list of lines."""
    in_steps, dash_indent, result = False, None, []
    for _, line in lines:
        if re.match(r'^\s{4}steps:\s*$', line):
            in_steps = True
            continue
        if not in_steps:
            continue
        dash = re.match(r'^(\s*)- ', line)
        if dash and (dash_indent is None or len(dash.group(1)) == dash_indent):
            dash_indent = len(dash.group(1))
            result.append([line])
        elif result:
            result[-1].append(line)
    return result


def run_text(step):
    """The shell text of a step's `run:` key, without comments."""
    collected, key_indent = [], None
    for line in step:
        match = RUN_KEY.match(line)
        if key_indent is None and match:
            key_indent = len(match.group(1)) + (len(match.group(2)) if match.group(2) else 0)
            inline = match.group(3)
            if inline and inline not in ('|', '>', '|-', '>-', '|+', '>+'):
                collected.append(inline)
        elif key_indent is not None:
            if line.strip() and len(line) - len(line.lstrip()) <= key_indent:
                break
            collected.append(line)
    return '\n'.join(l for l in collected if not l.lstrip().startswith('#'))


def check_job_setup(relative, text, pin, problems):
    for name, lines in jobs(text).items():
        setup_at = cargo_at = None
        for index, step in enumerate(steps(lines)):
            joined = '\n'.join(step)
            if re.search(rf'uses:\s*{re.escape(SETUP_ACTION)}@{re.escape(pin)}\s*$', joined, re.MULTILINE):
                setup_at = index if setup_at is None else setup_at
            if cargo_at is None and CARGO_WORD.search(run_text(step)):
                cargo_at = index
        if cargo_at is not None and (setup_at is None or setup_at > cargo_at):
            problems.append(
                f'{relative}: job {name} runs cargo without a pinned toolchain step '
                f'({SETUP_ACTION}@{pin} before the first cargo step)')


def helper_files(root):
    scripts = root / 'scripts'
    if not scripts.is_dir():
        return
    for path in sorted(scripts.rglob('*')):
        relative = path.relative_to(root).as_posix()
        if (path.is_file() and path.suffix in HELPER_SUFFIXES
                and not relative.startswith(HELPER_EXCLUDED_DIRS)
                and relative != 'scripts/ci/toolchain-pin-audit.py'):
            yield path


def audit(root):
    problems = []
    pin = toolchain_pin(root, problems)
    if pin is None:
        return problems
    workflows = root / '.github'
    if workflows.is_dir():
        for path in sorted(workflows.rglob('*.y*ml')):
            relative = path.relative_to(root).as_posix()
            text = path.read_text()
            scan_lines(relative, text, pin, problems, workflow=True)
            check_job_setup(relative, text, pin, problems)
        for path in sorted(workflows.rglob('*')):
            if path.is_file() and path.suffix in HELPER_SUFFIXES:
                scan_lines(path.relative_to(root).as_posix(), path.read_text(), pin, problems, workflow=False)
    for path in helper_files(root):
        scan_lines(path.relative_to(root).as_posix(), path.read_text(), pin, problems, workflow=False)
    return problems


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', default=str(Path(__file__).resolve().parents[2]),
                        help='repository root to audit (default: this checkout)')
    root = Path(parser.parse_args().root)
    problems = audit(root)
    if problems:
        for problem in problems:
            print(f'error: {problem}', file=sys.stderr)
        print(f'error: toolchain pin audit failed: {len(problems)} problem(s); every toolchain '
              'must be the exact version in rust-toolchain.toml', file=sys.stderr)
        sys.exit(1)
    print(f'ok: toolchain pin audit passed ({toolchain_pin(root, [])})')


if __name__ == '__main__':
    main()
