#!/usr/bin/env python3
"""Measure over-limit prose runs in tracked Markdown files."""

from __future__ import annotations

import re
import sys
from pathlib import Path


LIMIT = 12
LIST_ITEM = re.compile(r"^\s*(?:[-*+]|\d+[.)])\s+")
FENCE = re.compile(r"^\s*(```|~~~)")


def measure(path: str) -> tuple[int, int]:
    runs: list[int] = []
    current = 0
    fence: str | None = None

    with Path(path).open(encoding="utf-8") as markdown:
        for line in markdown:
            stripped = line.strip()
            fence_match = FENCE.match(line)

            if fence is not None:
                if fence_match and stripped.startswith(fence):
                    fence = None
                continue

            if fence_match:
                fence = fence_match.group(1)
                current = 0
                continue

            if (
                not stripped
                or stripped.startswith("|")
                or stripped.startswith("#")
                or stripped.startswith("<!--")
            ):
                current = 0
                continue

            if current == 0 or LIST_ITEM.match(line):
                current = 0
                runs.append(0)
            current += 1
            runs[-1] += 1

    over_limit = [run for run in runs if run > LIMIT]
    return len(over_limit), max(runs, default=0)


def main() -> int:
    for raw_path in sys.stdin:
        path = raw_path.rstrip("\n")
        if not path:
            continue
        count, longest = measure(path)
        if count:
            print(f"{path}\t{count}\t{longest}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
