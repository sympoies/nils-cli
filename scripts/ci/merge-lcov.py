#!/usr/bin/env python3
"""Merge LCOV line data from partitioned test runs and enforce a line floor.

Each input is an LCOV report from one nextest partition. A line's hit count in
the merged report is the sum of its counts across inputs, so a line covered by
any partition is covered. Only line records (SF/DA/LF/LH) are written; the
floor and the coverage summary read nothing else.
"""
import argparse
import sys
from collections import defaultdict


def read_lines(path, lines):
    source = None
    with open(path, encoding='utf-8') as report:
        for raw in report:
            record = raw.strip()
            if record.startswith('SF:'):
                source = record[3:]
            elif record.startswith('DA:') and source is not None:
                number, count = record[3:].split(',')[:2]
                lines[source][int(number)] += int(count)
            elif record == 'end_of_record':
                source = None


def merge(paths):
    lines = defaultdict(lambda: defaultdict(int))
    for path in paths:
        read_lines(path, lines)
    return lines


def write_report(lines, path):
    with open(path, 'w', encoding='utf-8') as out:
        for source in sorted(lines):
            hits = lines[source]
            out.write(f'SF:{source}\n')
            for number in sorted(hits):
                out.write(f'DA:{number},{hits[number]}\n')
            out.write(f'LF:{len(hits)}\nLH:{sum(1 for c in hits.values() if c > 0)}\n')
            out.write('end_of_record\n')


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('inputs', nargs='+', help='partition LCOV reports')
    parser.add_argument('--output', required=True, help='merged LCOV report')
    parser.add_argument('--fail-under-lines', type=float, required=True,
                        help='minimum merged line coverage percent')
    args = parser.parse_args(argv)

    lines = merge(args.inputs)
    write_report(lines, args.output)
    found = sum(len(hits) for hits in lines.values())
    hit = sum(1 for hits in lines.values() for count in hits.values() if count > 0)
    percent = 100.0 * hit / found if found else 0.0
    print(f'merged {len(args.inputs)} reports: {hit}/{found} lines, {percent:.2f}%')
    if percent < args.fail_under_lines:
        print(f'error: line coverage {percent:.2f}% is below the {args.fail_under_lines:g}% floor',
              file=sys.stderr)
        return 1
    return 0


if __name__ == '__main__':
    sys.exit(main())
