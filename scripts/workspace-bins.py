#!/usr/bin/env python3
from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path


def main() -> int:
    repo_root = Path(__file__).resolve().parent.parent
    result = subprocess.run(
        ["cargo", "metadata", "--no-deps", "--format-version", "1"],
        cwd=repo_root,
        check=False,
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        sys.stderr.write(result.stderr)
        return result.returncode

    metadata = json.loads(result.stdout)
    bins: set[str] = set()
    for package in metadata.get("packages", []):
        for target in package.get("targets", []):
            if "bin" in target.get("kind", []):
                name = target.get("name")
                if name:
                    bins.add(name)

    for name in sorted(bins):
        print(name)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

