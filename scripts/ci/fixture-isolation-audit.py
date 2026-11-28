#!/usr/bin/env python3
"""Ratchet direct Git, Unix socket, and TMPDIR operations in Rust tests."""

import argparse
from collections import Counter
from pathlib import Path
import re
import sys

sys.dont_write_bytecode = True
from lib.rust_file_size import SourceFile, test_only_files


ALLOWLIST = Path("scripts/ci/fixture-isolation-allowlist.tsv")
RULES = {
    "git": re.compile(
        r'\b(?:\w*Command|cmd)::new\s*\(\s*(?:r\#*)?"(?:[^"\n]*/)?git(?:\.exe)?"\#*\s*[,)]'
    ),
    "unix-socket": re.compile(
        r"\b(?:UnixListener\s*::\s*bind|UnixStream\s*::\s*connect|"
        r"UnixDatagram\s*::\s*bind)\s*\("
    ),
    "tmpdir": re.compile(
        r'\b(?:var(?:_os)?|(?:option_)?env!)\s*\(\s*"TMPDIR"\s*[,)]'
        r"|\benv\s*::\s*temp_dir\s*\("
    ),
}
FUNCTION = re.compile(r"\bfn\s+\w+\s*\(")
SOCKET_IN_FIXTURE = re.compile(r"&?\s*\w+\s*\.\s*runtime_dir\s*\(\s*\)\s*\.\s*join\s*\(")
SOCKET_ARG = re.compile(r"&?\s*(\w+)\s*\)")

def socket_in_fixture(masked, start, argument):
    if SOCKET_IN_FIXTURE.match(masked, argument):
        return True
    name = SOCKET_ARG.match(masked, argument)
    if not name:
        return False
    # Recognize the standard short-socket helper within the same function.
    function = list(FUNCTION.finditer(masked, 0, start))
    if not function:
        return False
    setup = masked[function[-1].end():start]
    binding = (
        rf"\blet\s*\(\s*\w+\s*,\s*{name[1]}\s*\)\s*=\s*(?:\w+::)*short_socket_path\s*\("
        rf"|\blet\s+{name[1]}\s*=\s*{SOCKET_IN_FIXTURE.pattern}"
    )
    return bool(re.search(binding, setup))


def rust_findings(root):
    texts = {
        path.relative_to(root).as_posix(): path.read_text(encoding="utf-8")
        for path in sorted((root / "crates").rglob("*.rs"))
        if not path.is_symlink()
    }
    sources = {path: SourceFile(text) for path, text in texts.items()}
    test_files = test_only_files(texts, sources)
    for path, text in texts.items():
        source = sources[path]
        name = Path(path).name
        test_only = path in test_files or name == "tests.rs" or name.endswith(("_test.rs", "_tests.rs"))
        if not test_only and not source.test_lines:
            continue
        lines = text.splitlines()
        for rule, pattern in RULES.items():
            for match in pattern.finditer(text):
                start = match.start()
                # The match must start in code, outside comments and string literals.
                if source.masked[start].isspace():
                    continue
                line = source.line_of(start)
                if not test_only and line not in source.test_lines:
                    continue
                if rule == "unix-socket" and socket_in_fixture(source.masked, start, match.end()):
                    continue
                # Keep the expression and the rest of its last line. This also
                # distinguishes multiline invocations without pinning line numbers.
                last_line = source.line_of(match.end() - 1)
                signature = " ".join(" ".join(lines[line - 1:last_line]).split())
                yield (path, rule, signature), line


def load_allowlist(root):
    path = root / ALLOWLIST
    counts = Counter()
    for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        if not line.strip() or line.startswith("#"):
            continue
        file, rule, signature, count, reason = line.split("\t")
        key = (file, rule, signature)
        if key in counts or rule not in RULES or int(count) < 1 or not reason.strip():
            raise ValueError(f"{path}:{number}: invalid or duplicate allowlist entry")
        counts[key] = int(count)

    return counts


def audit(root):
    remaining = load_allowlist(root)
    failed = False
    for key, line in rust_findings(root):
        if remaining[key] > 0:
            remaining[key] -= 1
            continue
        path, rule, _ = key
        failed = True
        print(f"{path}:{line}: {rule} fixture bypasses isolation", file=sys.stderr)
    for (path, rule, signature), count in remaining.items():
        if count > 0:
            failed = True
            print(f"{path}: {rule} has {count} unused allowance(s): {signature}", file=sys.stderr)
    if failed:
        print(
            f"Use nils_test_support::fixture::Fixture and keep exact counts in {ALLOWLIST}. "
            f"Include a concrete reason for reviewed exceptions.", file=sys.stderr
        )
        return 1
    print("fixture-isolation-audit: ok")
    return 0


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[2])
    args = parser.parse_args()
    try:
        return audit(args.root)
    except (OSError, ValueError) as error:
        print(f"fixture-isolation-audit: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
