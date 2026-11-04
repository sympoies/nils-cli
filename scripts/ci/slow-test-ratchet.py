#!/usr/bin/env python3
"""Advisory slow-test ratchet over a nextest JUnit report.

Names every test that takes THRESHOLD seconds or more and is not listed in the
baseline, which holds tests down to BASELINE_FLOOR seconds. The baseline may only shrink, like file-size-baseline.tsv.

  slow-test-ratchet.py --junit junit.xml [--summary FILE] [--strict]
  slow-test-ratchet.py --check-baseline [--base REF]
  slow-test-ratchet.py --junit junit.xml --update-baseline

Baseline: scripts/ci/slow-test-baseline.tsv (classname<TAB>test).
"""
import argparse
import subprocess
import sys
import xml.etree.ElementTree as ET
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
BASELINE_REL = 'scripts/ci/slow-test-baseline.tsv'
BASELINE = ROOT / BASELINE_REL
HEADER = 'classname\ttest'
THRESHOLD = 10.0
# The baseline also holds tests down to this floor so run-to-run timing noise
# near THRESHOLD does not name tests that were not touched.
BASELINE_FLOOR = 6.0


def read_baseline(text):
    return {tuple(line.split('\t', 1)) for line in text.splitlines()[1:] if line.strip()}


def slow_tests(junit, floor):
    found = {}
    for case in ET.parse(junit).getroot().iter('testcase'):
        seconds = float(case.get('time') or 0)
        if seconds >= floor:
            key = (case.get('classname', ''), case.get('name', ''))
            found[key] = max(seconds, found.get(key, 0))
    return found


def render(found, baseline, threshold):
    new = sorted((k for k in found if k not in baseline and found[k] >= threshold),
                 key=lambda k: -found[k])
    stale = baseline - set(found)
    lines = ['## Slow-test ratchet (advisory)', '']
    if new:
        lines += [f'{len(new)} test(s) took {threshold:g} s or more and are not in '
                  f'`{BASELINE_REL}`. Speed them up; do not add them to the baseline.', '',
                  '| Test | Seconds |', '| --- | ---: |']
        lines += [f'| `{c}` `{n}` | {found[(c, n)]:.1f} |' for c, n in new]
    else:
        lines.append(f'No new test took {threshold:g} s or more.')
    lines += ['', f'Baselined tests at or above {BASELINE_FLOOR:g} s in this run: '
              f'{len(set(found) & baseline)} of {len(baseline)}.']
    if stale:
        lines.append(f'{len(stale)} baseline row(s) are below {BASELINE_FLOOR:g} s or absent '
                     'in this run; remove them once the test is fixed or gone.')
    return '\n'.join(lines) + '\n', new


def baseline_additions(base, head):
    """Rows added without an equal number removed; the baseline may only shrink."""
    added = head - base
    return sorted(added) if len(base - head) < len(added) else []


def check_baseline(base_ref):
    show = subprocess.run(['git', 'show', f'{base_ref}:{BASELINE_REL}'], cwd=ROOT,
                          capture_output=True, text=True)
    if show.returncode != 0:
        print(f'slow-test baseline check skipped: no baseline at {base_ref}')
        return 0
    base = read_baseline(show.stdout)
    head = read_baseline(BASELINE.read_text())
    bad = baseline_additions(base, head)
    for c, n in bad:
        print(f'error: slow-test baseline row added without a removal: {c} {n}', file=sys.stderr)
    if not bad:
        print(f'slow-test baseline ok: +{len(head - base)} -{len(base - head)}')
    return 1 if bad else 0


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument('--junit')
    ap.add_argument('--threshold', type=float, default=THRESHOLD)
    ap.add_argument('--summary', help='append the Markdown report to this file')
    ap.add_argument('--strict', action='store_true', help='exit 1 when a new slow test is found')
    ap.add_argument('--update-baseline', action='store_true')
    ap.add_argument('--check-baseline', action='store_true')
    ap.add_argument('--base', default='origin/main')
    args = ap.parse_args()
    if args.check_baseline:
        return check_baseline(args.base)
    if not args.junit:
        ap.error('--junit is required')
    found = slow_tests(args.junit, BASELINE_FLOOR)
    if args.update_baseline:
        BASELINE.write_text('\n'.join([HEADER] + ['\t'.join(k) for k in sorted(found)]) + '\n')
        return 0
    report, new = render(found, read_baseline(BASELINE.read_text()), args.threshold)
    print(report, end='')
    if args.summary:
        with open(args.summary, 'a') as out:
            out.write(report)
    return 1 if new and args.strict else 0


if __name__ == '__main__':
    sys.exit(main())
