#!/usr/bin/env python3
"""Count implementation and test lines of Rust sources for the file-size ratchet.

Reads repo-relative `.rs` paths on stdin (run from the repository root) and
prints `path<TAB>kind<TAB>lines` rows, where kind is `impl` or `test`. Zero
counts are omitted.

Rules:
  - A file under a `tests/` or `benches/` directory is test-only.
  - A file reached through an out-of-line `#[cfg(test)] mod x;` declaration is
    test-only, and so is every file that a test-only file declares with `mod y;`.
  - In any other file, each top-level `#[cfg(test)]` item counts as test lines,
    from its attribute through the end of its braced block or its `;`.
  - Implementation lines are the total lines minus the test lines.

Comments, string literals, raw strings and char literals are masked before brace
matching, so braces inside them never change where a test module ends.
"""

import bisect
import pathlib
import re
import sys
from pathlib import PurePosixPath

TEST_DIR_NAMES = {"tests", "benches"}
MOD_FILE_NAMES = {"mod.rs", "lib.rs", "main.rs"}

# Alternatives are tried left to right at each position, so a comment or literal
# is consumed as one token before its interior can be read as code.
TOKEN_RE = re.compile(
    r"""
      (?P<line_comment>//[^\n]*)
    | (?P<block_open>/\*)
    | (?P<raw>(?<![0-9A-Za-z_])b?r\#*")
    | (?P<string>b?"(?:[^"\\]|\\.)*")
    | (?P<char>'(?:\\(?:u\{[^}\n]*\}|x[0-9A-Fa-f]{2}|[^\n])|[^'\\\n])')
    | (?P<brace>[{}])
    """,
    re.VERBOSE | re.DOTALL,
)
BLOCK_DELIM_RE = re.compile(r"/\*|\*/")
BRACE_RE = re.compile(r"[{}]")
CFG_TEST_RE = re.compile(r"#\[cfg\(test\)\]")
ITEM_END_RE = re.compile(r"[{;]")
MOD_DECL_RE = re.compile(r"\bmod\s+([A-Za-z_][A-Za-z0-9_]*)\s*;")
CFG_TEST_MOD_RE = re.compile(
    r"#\[cfg\(test\)\]\s*(?:#\[[^\]]*\]\s*)*(?:pub(?:\([^)]*\))?\s+)?"
    r"mod\s+([A-Za-z_][A-Za-z0-9_]*)\s*;"
)


def blank(fragment):
    return re.sub(r"[^\n]", " ", fragment)


def block_comment_end(text, pos):
    """Return the offset just past the block comment whose body starts at pos."""
    depth = 1
    for match in BLOCK_DELIM_RE.finditer(text, pos):
        depth += 1 if match.group() == "/*" else -1
        if depth == 0:
            return match.end()
    return len(text)


def mask_source(text):
    """Replace comments and literals with spaces, keeping newlines and braces."""
    pieces = []
    last = 0
    pos = 0
    while True:
        match = TOKEN_RE.search(text, pos)
        if match is None:
            break
        pos = match.end()
        kind = match.lastgroup
        if kind == "brace":
            continue
        start, end = match.start(), match.end()
        if kind == "block_open":
            end = block_comment_end(text, match.end())
        elif kind == "raw":
            close = '"' + "#" * match.group().count("#")
            found = text.find(close, match.end())
            end = len(text) if found < 0 else found + len(close)
        pieces.append(text[last:start])
        pieces.append(blank(text[start:end]))
        last = pos = end
    pieces.append(text[last:])
    return "".join(pieces)


class SourceFile:
    """Masked source plus the line and brace facts the ratchet needs."""

    def __init__(self, text):
        self.masked = mask_source(text)
        self.total = self.masked.count("\n")
        if self.masked and not self.masked.endswith("\n"):
            self.total += 1
        self.line_breaks = [m.start() for m in re.finditer("\n", self.masked)]

        self.events = [
            (m.start(), 1 if m.group() == "{" else -1)
            for m in BRACE_RE.finditer(self.masked)
        ]
        self.event_positions = [pos for pos, _ in self.events]
        self.depths = []
        depth = 0
        for _, delta in self.events:
            depth += delta
            self.depths.append(depth)

        self.test_lines = self._in_file_test_lines()
        self.mod_decls = self._top_level_mods(MOD_DECL_RE)
        self.cfg_test_mods = self._top_level_mods(CFG_TEST_MOD_RE)

    def line_of(self, pos):
        return bisect.bisect_left(self.line_breaks, pos) + 1

    def depth_at(self, pos):
        index = bisect.bisect_left(self.event_positions, pos)
        return self.depths[index - 1] if index else 0

    def _in_file_test_lines(self):
        lines = set()
        for attr in CFG_TEST_RE.finditer(self.masked):
            if self.depth_at(attr.start()) != 0:
                continue
            item_end = self._item_end(attr.end())
            first = self.line_of(attr.start())
            last = min(self.line_of(item_end), self.total)
            lines.update(range(first, last + 1))
        return lines

    def _item_end(self, pos):
        boundary = ITEM_END_RE.search(self.masked, pos)
        if boundary is None:
            return len(self.masked)
        if boundary.group() == ";":
            return boundary.start()
        index = bisect.bisect_left(self.event_positions, boundary.start())
        for close in range(index, len(self.events)):
            if self.events[close][1] == -1 and self.depths[close] == 0:
                return self.event_positions[close]
        return len(self.masked)

    def _top_level_mods(self, pattern):
        names = []
        for match in pattern.finditer(self.masked):
            if self.depth_at(match.start()) == 0:
                names.append(match.group(1))
        return names


def child_candidates(path, name):
    """Return the two files a `mod <name>;` declared in `path` may resolve to."""
    rel = PurePosixPath(path)
    if rel.name in MOD_FILE_NAMES:
        module_dir = rel.parent
    else:
        module_dir = rel.parent / rel.stem
    return [str(module_dir / f"{name}.rs"), str(module_dir / name / "mod.rs")]


def resolve_child(path, name, files):
    for candidate in child_candidates(path, name):
        if candidate in files:
            return candidate
    return None


def test_only_files(files, sources):
    test_only = set()
    queue = []

    def mark(path):
        if path is not None and path in files and path not in test_only:
            test_only.add(path)
            queue.append(path)

    for path in files:
        if TEST_DIR_NAMES.intersection(PurePosixPath(path).parent.parts):
            mark(path)

    for path in files:
        if path in test_only:
            continue
        for name in sources[path].cfg_test_mods:
            mark(resolve_child(path, name, files))

    while queue:
        path = queue.pop()
        for name in sources[path].mod_decls:
            mark(resolve_child(path, name, files))

    return test_only


def main():
    paths = [line.strip() for line in sys.stdin if line.strip()]
    files = set()
    sources = {}
    for path in paths:
        target = pathlib.Path(path)
        if not target.is_file():
            # A tracked file deleted in the working tree is handled by the caller
            # as a missing file, so it contributes no rows here.
            continue
        text = target.read_text(encoding="utf-8", errors="replace")
        files.add(path)
        sources[path] = SourceFile(text)

    test_only = test_only_files(files, sources)
    for path in sorted(files):
        source = sources[path]
        if path in test_only:
            impl, test = 0, source.total
        else:
            test = len(source.test_lines)
            impl = source.total - test
        if impl:
            print(f"{path}\timpl\t{impl}")
        if test:
            print(f"{path}\ttest\t{test}")


if __name__ == "__main__":
    main()
