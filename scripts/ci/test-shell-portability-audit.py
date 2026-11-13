#!/usr/bin/env python3
"""Reject known GNU-only shell forms in test text; see the maintenance reference."""
import argparse
import json
import os
from pathlib import Path
import re


COMMANDS = re.compile(r"\b(stat|sed|readlink|date|grep|xargs)\s+")
OPTIONS = re.compile(r"(?<!\S)(--(?=\s|$)|--[a-z][a-z-]*(?:=[^\s;|&]*)?|-[A-Za-z0-9][^\s;|&]*)")
SHORT_OPTIONS = {"stat": "c", "sed": "i", "readlink": "f", "date": "d", "grep": "P", "xargs": "r"}
LONG_OPTIONS = {
    "stat": {"--format"}, "sed": {"--in-place"},
    "readlink": {"--canonicalize", "--canonicalize-existing", "--canonicalize-missing"},
    "date": {"--date"}, "grep": {"--perl-regexp"}, "xargs": {"--no-run-if-empty"},
}
SED_SUFFIX = re.compile(r"""\s+(?:''|""|(?:['"])?\.[\w.-]+(?:['"])?)(?=\s|$)""")
ALLOWLIST = "scripts/ci/test-shell-portability-allowlist.json"
# The rule's synthetic inputs deliberately contain every rejected form.
RULE_TEST = "scripts/ci/tests/test-shell-portability-audit.test.py"


def gnu_forms(line):
    hits = set()
    for command in COMMANDS.finditer(line):
        name = command[1]
        arguments = re.split(r"[;|&]", line[command.end():], maxsplit=1)[0]
        for option in OPTIONS.finditer(arguments):
            token = option[0]
            if token == "--":
                break
            if token.startswith("--"):
                if token.split("=", 1)[0] in LONG_OPTIONS[name]:
                    hits.add(name)
                continue
            cluster = re.match(r"-([A-Za-z0-9]+)", token)[1]
            flag = SHORT_OPTIONS[name]
            if flag not in cluster:
                continue
            if name == "sed":
                suffix = token[token.index("i") + 1:]
                if suffix.startswith("."):
                    continue
                if not suffix and SED_SUFFIX.match(arguments[option.end():]):
                    continue
            hits.add(name)
    return sorted(hits)


def test_location(relative):
    return (bool({"tests", "fixtures", "testdata"} & set(relative.parts))
            or relative.name.endswith((".test.sh", ".test.py", ".test.cjs", "_test.rs"))
            or relative.name == "tests.rs")


def test_files(root):
    for directory, children, files in os.walk(root):
        children[:] = sorted(set(children) - {".git", ".cache", "target", "node_modules"})
        for name in sorted(files):
            path = Path(directory) / name
            relative = path.relative_to(root)
            if relative.as_posix() == RULE_TEST or path.is_symlink():
                continue
            if test_location(relative) or path.suffix == ".rs":
                yield path, relative


def audit(root):
    allowed = {}
    errors = []
    rows = json.loads((root / ALLOWLIST).read_text())
    for row in rows:
        key = (row["path"], row["line_number"], row["line"])
        if (row.get("kind") not in {"fallback", "classifier-input"}
                or not row.get("reason", "").strip() or key in allowed):
            errors.append(f"{ALLOWLIST}: invalid or duplicate allowlist entry: {row['path']}")
        if row["kind"] == "fallback" and not row.get("context", "").strip():
            errors.append(f"{ALLOWLIST}: fallback context required: {row['path']}")
        allowed[key] = (row, False)
    count = 0
    for path, relative_path in test_files(root):
        relative = relative_path.as_posix()
        data = path.read_bytes()
        if b"\0" in data:
            continue
        try:
            text = data.decode("utf-8")
            lines = text.splitlines()
        except UnicodeDecodeError:
            continue
        start = 0
        if not test_location(relative_path):
            # Inline Rust unit tests conventionally start a trailing cfg(test) module.
            start = next((i for i, line in enumerate(lines) if line.strip() == "#[cfg(test)]"), len(lines))
            if start == len(lines):
                continue
        count += 1
        for number, line in enumerate(lines[start:], start + 1):
            hits = gnu_forms(line)
            if not hits:
                continue
            key = (relative, number, line)
            if key in allowed:
                row, _ = allowed[key]
                context = row.get("context", "").splitlines()
                start = number - 1 - context.index(line) if line in context else -1
                if (row["kind"] == "classifier-input"
                        or start >= 0 and lines[start:start + len(context)] == context):
                    allowed[key] = (row, True)
                    continue
            errors.append(f"{relative}:{number}: {', '.join(hits)}: {line.strip()}")
    for (path, _, _), (_, seen) in allowed.items():
        if not seen:
            errors.append(f"{ALLOWLIST}: stale allowlist entry: {path}")
    if errors:
        print("\n".join(errors))
        print("Use portable shell or a reviewed exact fallback/classifier-input allowlist entry.")
        return 1
    print(f"PASS: test shell portability audit ({count} text files)")
    return 0


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[2])
    args = parser.parse_args()
    try:
        return audit(args.root.resolve())
    except (OSError, ValueError, KeyError, TypeError) as error:
        print(f"test shell portability audit: {error}")
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
