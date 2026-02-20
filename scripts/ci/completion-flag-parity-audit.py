#!/usr/bin/env python3
from __future__ import annotations

import argparse
import os
import re
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path


@dataclass
class CmdResult:
    code: int
    stdout: str
    stderr: str


HELP_TIMEOUT_SECONDS = 5
COMPLETION_TIMEOUT_SECONDS = 30
MAX_COMMAND_DEPTH = 6


def run_command(args: list[str], cwd: Path, timeout_seconds: int = HELP_TIMEOUT_SECONDS) -> CmdResult:
    try:
        proc = subprocess.run(
            args,
            cwd=cwd,
            check=False,
            capture_output=True,
            text=True,
            timeout=timeout_seconds,
        )
    except subprocess.TimeoutExpired:
        return CmdResult(
            code=124,
            stdout="",
            stderr=f"command timed out after {timeout_seconds}s: {' '.join(args)}",
        )
    return CmdResult(code=proc.returncode, stdout=proc.stdout, stderr=proc.stderr)


def parse_required_bins(matrix_path: Path) -> list[str]:
    row_re = re.compile(r"^\|\s*`([^`]+)`\s*\|\s*`([^`]+)`\s*\|")
    required: list[str] = []
    for line in matrix_path.read_text(encoding="utf-8").splitlines():
        match = row_re.match(line)
        if not match:
            continue
        binary, obligation = match.group(1), match.group(2)
        if obligation == "required":
            required.append(binary)
    return sorted(set(required))


def parse_commands(help_text: str) -> list[str]:
    lines = help_text.splitlines()
    in_commands = False
    commands: list[str] = []
    row_re = re.compile(r"^\s{2,}([A-Za-z0-9][A-Za-z0-9-]*)\s{2,}")

    for line in lines:
        if line.strip() == "Commands:":
            in_commands = True
            continue
        if not in_commands:
            continue
        if not line.strip():
            break
        if line.lstrip().startswith("-"):
            continue
        match = row_re.match(line)
        if not match:
            continue
        name = match.group(1)
        if name == "help":
            continue
        commands.append(name)
    return commands


def parse_flags(help_text: str) -> list[str]:
    lines = help_text.splitlines()
    in_options = False
    flags: list[str] = []
    seen: set[str] = set()
    token_re = re.compile(r"-{1,2}[A-Za-z0-9][A-Za-z0-9-]*")

    for raw_line in lines:
        line = raw_line.strip()
        if line == "Options:":
            in_options = True
            continue
        if not in_options:
            continue
        if not line:
            break
        if not raw_line.lstrip().startswith("-"):
            continue
        spec_end = line.find("  ")
        spec = line if spec_end < 0 else line[:spec_end]
        for token in token_re.findall(spec):
            if token in {"-h", "--help"}:
                continue
            if token in seen:
                continue
            seen.add(token)
            flags.append(token)
    return flags


def parse_completion_flag_tokens(opts_text: str) -> set[str]:
    tokens = set(token for token in opts_text.split() if token.startswith("-"))
    tokens.discard("-h")
    tokens.discard("--help")
    return tokens


def bash_case_label(binary: str, path: tuple[str, ...]) -> str:
    root = binary.replace("-", "__")
    if not path:
        return root
    parts = [segment.replace("-", "__") for segment in path]
    return "__".join([root, *parts])


def bash_case_opts(script: str, label: str) -> str | None:
    lines = script.splitlines()
    marker = f"{label})"
    in_case = False
    for line in lines:
        trimmed = line.strip()
        if trimmed == marker:
            in_case = True
            continue
        if not in_case:
            continue
        if trimmed.startswith("opts=\""):
            end = trimmed.find('"', len("opts=\""))
            if end == -1:
                return None
            return trimmed[len("opts=\"") : end]
        if trimmed.endswith(")") and not trimmed.startswith("opts="):
            return None
    return None


def zsh_context_marker(binary: str, parents: tuple[str, ...]) -> str:
    if not parents:
        return f'curcontext="${{curcontext%:*:*}}:{binary}-command-$line[1]:"'
    joined = "-".join(parents)
    return f'curcontext="${{curcontext%:*:*}}:{binary}-{joined}-command-$line[1]:"'


def zsh_leaf_block(script: str, binary: str, path: tuple[str, ...]) -> str | None:
    args_marker = '_arguments "${_arguments_options[@]}" : \\'

    if not path:
        args_idx = script.find(args_marker)
        if args_idx < 0:
            return None
        end_idx = script.find("&& ret=0", args_idx)
        if end_idx < 0:
            return None
        return script[args_idx:end_idx]

    parents = path[:-1]
    leaf = path[-1]
    marker = zsh_context_marker(binary, parents)
    marker_idx = script.find(marker)
    if marker_idx < 0:
        return None
    from_marker = script[marker_idx:]

    leaf_marker = f"({leaf})"
    leaf_idx = from_marker.find(leaf_marker)
    if leaf_idx < 0:
        return None
    from_leaf = from_marker[leaf_idx:]

    args_idx = from_leaf.find(args_marker)
    if args_idx < 0:
        return None
    from_args = from_leaf[args_idx:]

    end_idx = from_args.find("&& ret=0")
    if end_idx < 0:
        return None
    return from_args[:end_idx]


def contains_token(haystack: str, token: str) -> bool:
    pattern = re.compile(rf"(?<![A-Za-z0-9-]){re.escape(token)}(?![A-Za-z0-9-])")
    return pattern.search(haystack) is not None


def gather_leaf_help(
    repo_root: Path,
    binary_path: Path,
) -> tuple[dict[tuple[str, ...], str], dict[tuple[str, ...], str]]:
    help_by_path: dict[tuple[str, ...], str] = {}
    failures: dict[tuple[str, ...], str] = {}
    root_help_text: str | None = None

    def walk(path: tuple[str, ...]) -> None:
        if len(path) > MAX_COMMAND_DEPTH:
            failures[path] = f"path depth exceeded safety limit ({MAX_COMMAND_DEPTH})"
            return

        result = run_command(
            [str(binary_path), *path, "--help"],
            cwd=repo_root,
            timeout_seconds=HELP_TIMEOUT_SECONDS,
        )
        if result.code != 0:
            failures[path] = (
                f"`{' '.join([str(binary_path), *path, '--help'])}` failed "
                f"(exit {result.code}): {result.stderr.strip()}"
            )
            return

        nonlocal root_help_text
        if path == ():
            root_help_text = result.stdout
        elif root_help_text is not None and result.stdout == root_help_text:
            failures[path] = (
                "help output fell back to root help for nested command path "
                f"`{' '.join(path)}`"
            )
            return

        help_by_path[path] = result.stdout
        commands = parse_commands(result.stdout)
        if not commands:
            return
        for command in commands:
            walk((*path, command))

    walk(())
    return help_by_path, failures


def ensure_binaries(repo_root: Path, binaries: list[str]) -> list[Path]:
    suffix = ".exe" if os.name == "nt" else ""
    target_dir = repo_root / "target" / "debug"
    paths = [target_dir / f"{binary}{suffix}" for binary in binaries]
    missing = [path for path in paths if not path.exists()]
    if not missing:
        return paths

    build = run_command(["cargo", "build", "--workspace", "--bins"], cwd=repo_root)
    if build.code != 0:
        stderr = build.stderr.strip()
        stdout = build.stdout.strip()
        details = stderr if stderr else stdout
        raise RuntimeError(f"cargo build --workspace --bins failed: {details}")

    missing_after = [path for path in paths if not path.exists()]
    if missing_after:
        missing_list = ", ".join(str(path.relative_to(repo_root)) for path in missing_after)
        raise RuntimeError(f"missing binaries after build: {missing_list}")

    return paths


def audit_binary(repo_root: Path, binary: str, binary_path: Path) -> list[str]:
    failures: list[str] = []

    bash_completion = run_command(
        [str(binary_path), "completion", "bash"],
        cwd=repo_root,
        timeout_seconds=COMPLETION_TIMEOUT_SECONDS,
    )
    if bash_completion.code != 0:
        failures.append(
            f"{binary}: completion bash failed (exit {bash_completion.code}): "
            f"{bash_completion.stderr.strip()}"
        )
        return failures
    bash_script = bash_completion.stdout

    zsh_completion = run_command(
        [str(binary_path), "completion", "zsh"],
        cwd=repo_root,
        timeout_seconds=COMPLETION_TIMEOUT_SECONDS,
    )
    if zsh_completion.code != 0:
        failures.append(
            f"{binary}: completion zsh failed (exit {zsh_completion.code}): "
            f"{zsh_completion.stderr.strip()}"
        )
        return failures
    zsh_script = zsh_completion.stdout

    help_by_path, help_failures = gather_leaf_help(repo_root, binary_path)
    if not help_by_path:
        failures.append(f"{binary}: no help paths discovered")
        return failures

    for path, details in sorted(help_failures.items()):
        case_label = bash_case_label(binary, path)
        bash_opts = bash_case_opts(bash_script, case_label) or ""
        required_flags = parse_completion_flag_tokens(bash_opts)
        if required_flags:
            joined = " ".join(path) if path else "<root>"
            failures.append(
                f"{binary}: unable to read help for `{joined}` while completion has flags "
                f"{sorted(required_flags)}; {details}"
            )

    for path, help_text in sorted(help_by_path.items()):
        flags = parse_flags(help_text)
        if not flags:
            continue

        case_label = bash_case_label(binary, path)
        bash_opts = bash_case_opts(bash_script, case_label)
        if bash_opts is None:
            joined = " ".join(path) if path else "<root>"
            failures.append(f"{binary}: missing bash completion case for {joined} ({case_label})")
            continue

        zsh_block = zsh_leaf_block(zsh_script, binary, path)
        if zsh_block is None:
            joined = " ".join(path) if path else "<root>"
            failures.append(f"{binary}: missing zsh completion block for {joined}")
            continue

        for flag in flags:
            if not contains_token(bash_opts, flag):
                joined = " ".join(path) if path else "<root>"
                failures.append(
                    f"{binary}: bash completion missing flag `{flag}` for command `{joined}`"
                )
            if not contains_token(zsh_block, flag):
                joined = " ".join(path) if path else "<root>"
                failures.append(
                    f"{binary}: zsh completion missing flag `{flag}` for command `{joined}`"
                )

    return failures


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Audit completion flag parity between --help and bash/zsh completions."
    )
    parser.add_argument(
        "--strict",
        action="store_true",
        help="Compatibility flag. The audit is always strict.",
    )
    args = parser.parse_args()
    _ = args.strict

    repo_root = Path(__file__).resolve().parents[2]
    matrix_path = repo_root / "docs" / "reports" / "completion-coverage-matrix.md"
    if not matrix_path.exists():
        print(f"FAIL: missing completion matrix: {matrix_path}", file=sys.stderr)
        return 2

    required_bins = parse_required_bins(matrix_path)
    if not required_bins:
        print(f"FAIL: no required binaries found in matrix: {matrix_path}", file=sys.stderr)
        return 2

    try:
        binary_paths = ensure_binaries(repo_root, required_bins)
    except RuntimeError as err:
        print(f"FAIL: {err}", file=sys.stderr)
        return 2

    failures: list[str] = []
    for binary, binary_path in zip(required_bins, binary_paths):
        failures.extend(audit_binary(repo_root, binary, binary_path))

    if failures:
        for failure in failures:
            print(f"FAIL: {failure}")
        print(
            "FAIL: completion flag parity audit "
            f"(required={len(required_bins)}, failures={len(failures)})"
        )
        return 1

    print(
        "PASS: completion flag parity audit "
        f"(required={len(required_bins)}, failures=0)"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
