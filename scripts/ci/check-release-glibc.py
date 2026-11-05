#!/usr/bin/env python3
"""Reject Linux release binaries requiring glibc newer than the supported baseline."""

import argparse
import re
import subprocess
import sys

# Ubuntu 24.04 is the oldest supported Linux release environment.
GLIBC_BASELINE = "2.39"


def check_binaries(binaries):
    baseline = tuple(map(int, GLIBC_BASELINE.split(".")))
    failed = False
    for binary in binaries:
        try:
            result = subprocess.run(
                ["readelf", "--version-info", "--wide", str(binary)],
                capture_output=True, text=True, check=True,
            )
        except (OSError, subprocess.CalledProcessError) as error:
            print(f"error: {binary}: cannot inspect ELF symbol versions: {error}", file=sys.stderr)
            failed = True
            continue
        versions = set(re.findall(r"\bGLIBC_(\d+(?:\.\d+)+)\b", result.stdout))
        newer = sorted(
            (version for version in versions if tuple(map(int, version.split("."))) > baseline),
            key=lambda version: tuple(map(int, version.split("."))),
        )
        if newer:
            for version in newer:
                print(
                    f"error: {binary}: requires GLIBC_{version}, "
                    f"newer than supported GLIBC_{GLIBC_BASELINE}", file=sys.stderr,
                )
            failed = True
        else:
            print(f"ok: {binary}: GLIBC requirements <= {GLIBC_BASELINE}")
    return int(failed)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binaries", nargs="+", help="Linux ELF release binaries to inspect")
    return check_binaries(parser.parse_args().binaries)


if __name__ == "__main__":
    sys.exit(main())
