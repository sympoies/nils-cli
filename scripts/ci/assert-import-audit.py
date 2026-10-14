#!/usr/bin/env python3
"""Reject ambiguous pretty_assertions macros inherited through use super::*.

Cargo-free and platform-independent: inspect all cfg branches, inline modules,
literal #[path] modules and literal include! files under crates/. This is a
source authoring guard, not a Rust name resolver; generated include! expressions
and macro-generated modules are outside its scope.
"""
import argparse
import ast
from dataclasses import dataclass, field
from pathlib import Path
import re
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent / 'lib'))
from rust_file_size import SourceFile, blank

MOD = re.compile(r'(?:#\[[^\]]*\]\s*)*(?:pub(?:\([^)]*\))?\s+)?'
                 r'\bmod\s+(\w+)\s*([;{])')
USE = re.compile(r'\buse\s+([^;]+);')
CALL = re.compile(r'\b(assert_eq|assert_ne)\s*!')
INCLUDE = re.compile(r'\binclude\s*!\s*\(\s*[^)]*\)\s*;?')
LITERAL = r'(r\#*".*?"\#*|"(?:[^"\\]|\\.)*")'
MACROS = {'assert_eq', 'assert_ne'}


def literal_path(text):
    """Read a literal Rust path without evaluating expressions or macros."""
    text = text.strip()
    if text.startswith('r'):
        match = re.fullmatch(r'r(\#*)"(.*)"\1', text, re.DOTALL)
        return match[2] if match else None
    try:
        value = ast.literal_eval(text)
        return value if isinstance(value, str) else None
    except (ValueError, SyntaxError):
        return None


def use_paths(text, prefix=''):
    """Flatten grouped use trees, preserving aliases in the leaf spelling."""
    depth, start = 0, 0
    for index, char in enumerate(text + ','):
        if char == '{':
            depth += 1
        elif char == '}':
            depth -= 1
        elif char == ',' and depth == 0:
            part = text[start:index].strip()
            start = index + 1
            if '{' in part:
                head, tail = part.split('{', 1)
                yield from use_paths(tail.rsplit('}', 1)[0], prefix + head)
            elif part:
                yield prefix + part


@dataclass
class Module:
    imports: set = field(default_factory=set)
    glob: bool = False
    calls: list = field(default_factory=list)
    children: list = field(default_factory=list)


class Audit:
    def __init__(self, root):
        self.root = root.resolve()
        self.files = {path.resolve(): path.read_text() for path in
                      sorted((self.root / 'crates').rglob('*.rs'))}
        self.modules = {}

    def file_module(self, path, stack):
        node = Module()
        if path not in self.files or path in stack:
            return node
        if path in self.modules:
            return self.modules[path]
        module_dir = path.parent if path.name in {'lib.rs', 'main.rs', 'mod.rs'} else path.with_suffix('')
        self.scan(node, path, 0, len(self.files[path]), module_dir, path.parent, stack | {path})
        self.modules[path] = node
        return node

    def scan(self, node, path, start, end, module_dir, path_base, stack):
        text = self.files[path][start:end]
        source = SourceFile(text)
        masked = source.masked
        pieces, last = [], 0
        for match in MOD.finditer(masked):
            if match.start() < last or source.depth_at(match.start()) != 0:
                continue
            name, boundary = match.groups()
            if boundary == '{':
                stop = source._block_end(match.end() - 1) + 1
                child = Module()
                self.scan(child, path, start + match.end(), start + stop - 1,
                          module_dir / name, module_dir / name, stack)
            else:
                stop = match.end()
                attrs = text[match.start():match.end()]
                explicit = re.search(r'#\[\s*path\s*=\s*' + LITERAL + r'\s*\]', attrs, re.DOTALL)
                if explicit:
                    value = literal_path(explicit[1])
                    candidates = [path_base / value] if value is not None else []
                else:
                    candidates = [module_dir / (name + '.rs'), module_dir / name / 'mod.rs']
                child_path = next((p.resolve() for p in candidates if p.resolve() in self.files), None)
                child = self.file_module(child_path, stack)
            node.children.append(child)
            pieces.extend((masked[last:match.start()], blank(masked[match.start():stop])))
            last = stop
        pieces.append(masked[last:])
        own = ''.join(pieces)
        own_source = SourceFile(own)
        for match in USE.finditer(own):
            if own_source.depth_at(match.start()) != 0:
                continue
            for imported in use_paths(re.sub(r'\s+', '', match[1])):
                imported = imported.removeprefix('::')
                node.glob |= imported == 'super::*'
                if imported.startswith('pretty_assertions::'):
                    macro = imported.removeprefix('pretty_assertions::')
                    if macro in MACROS:
                        node.imports.add(macro)
        for match in CALL.finditer(own):
            if own[:match.start()].rstrip().endswith('::'):
                continue
            line = self.files[path].count('\n', 0, start + match.start()) + 1
            node.calls.append((path.relative_to(self.root).as_posix(), line, match[1]))
        for match in INCLUDE.finditer(own):
            if own_source.depth_at(match.start()) != 0:
                continue
            original = text[match.start():match.end()]
            argument = re.fullmatch(r'include\s*!\s*\(\s*' + LITERAL + r'\s*\)\s*;?', original, re.DOTALL)
            value = literal_path(argument[1]) if argument else None
            if value is None:
                continue
            included = (path.parent / value).resolve()
            if included in self.files and included not in stack:
                self.scan(node, included, 0, len(self.files[included]), module_dir,
                          included.parent, stack | {included})

    def findings(self):
        findings = set()

        def visit(node, parent_imports):
            inherited = parent_imports if node.glob else set()
            for path, line, macro in node.calls:
                if macro in inherited - node.imports:
                    findings.add((path, line, macro))
            for child in node.children:
                visit(child, node.imports | inherited)

        # Also start at disconnected test helpers: not every integration test has
        # a lib.rs/main.rs root. Findings reached from multiple roots deduplicate.
        for path in self.files:
            visit(self.file_module(path, set()), set())
        return sorted(findings)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', type=Path, default=Path(__file__).resolve().parents[2])
    args = parser.parse_args()
    findings = Audit(args.root).findings()
    for path, line, macro in findings:
        print(f'{path}:{line}: ambiguous {macro}! via use super::*; '
              f'explicitly import pretty_assertions::{macro} in this module')
    print(f'{"FAIL" if findings else "PASS"}: assertion import audit ({len(findings)} findings)')
    return 1 if findings else 0


if __name__ == '__main__':
    raise SystemExit(main())
