#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage:
  nils-cli-release --version X.Y.Z [options]

Options:
  --version X.Y.Z   Required. Accepts vX.Y.Z and normalizes to X.Y.Z.
  --skip-checks     Skip full lint/tests; runs cargo check to refresh Cargo.lock.
  --ci-gate-main    Skip full lint/tests only when origin/main HEAD has a green CI run.
  --skip-readme     Do not update README release tag examples.
  --skip-push       Do not push commit or tag to origin.
  --allow-dirty     Allow a dirty working tree.
  --force-tag       Delete existing local/remote tag before re-tagging.
  -h, --help        Show help.
USAGE
}

die() {
  echo "error: $*" >&2
  exit 1
}

note() {
  echo "info: $*" >&2
}

version=""
skip_checks=0
ci_gate_main=0
skip_readme=0
skip_push=0
allow_dirty=0
force_tag=0

while [[ $# -gt 0 ]]; do
  case "${1:-}" in
    --version)
      if [[ $# -lt 2 ]]; then
        die "--version requires a value"
      fi
      version="${2:-}"
      shift 2
      ;;
    --skip-checks)
      skip_checks=1
      shift
      ;;
    --ci-gate-main)
      ci_gate_main=1
      shift
      ;;
    --skip-readme)
      skip_readme=1
      shift
      ;;
    --skip-push)
      skip_push=1
      shift
      ;;
    --allow-dirty)
      allow_dirty=1
      shift
      ;;
    --force-tag)
      force_tag=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      die "unknown argument: ${1:-}"
      ;;
  esac
 done

if [[ -z "$version" ]]; then
  usage >&2
  exit 2
fi

if [[ "$version" =~ ^v([0-9]+\.[0-9]+\.[0-9]+)$ ]]; then
  version="${BASH_REMATCH[1]}"
fi
if ! [[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  die "invalid --version: ${version} (expected X.Y.Z or vX.Y.Z)"
fi

tag="v${version}"

for cmd in git python3 cargo semantic-commit git-scope; do
  if ! command -v "$cmd" >/dev/null 2>&1; then
    die "missing required command: ${cmd}"
  fi
done

if [[ "$ci_gate_main" -eq 1 ]]; then
  if ! command -v gh >/dev/null 2>&1; then
    die "--ci-gate-main requires gh on PATH"
  fi
fi

repo_root="$(git rev-parse --show-toplevel 2>/dev/null || true)"
if [[ -z "$repo_root" ]]; then
  die "must run inside a git work tree"
fi

cd "$repo_root"

if [[ ! -f Cargo.toml ]]; then
  die "Cargo.toml not found in repo root"
fi

current_branch="$(git branch --show-current 2>/dev/null || true)"
if [[ -n "$current_branch" && "$current_branch" != "main" ]]; then
  note "current branch is '${current_branch}' (release tags are typically on main)"
fi

if [[ "$allow_dirty" -eq 0 ]]; then
  if [[ -n "$(git status --porcelain)" ]]; then
    die "working tree is not clean; commit/stash changes or use --allow-dirty"
  fi
fi

if [[ "$ci_gate_main" -eq 1 ]]; then
  if [[ -z "$current_branch" || "$current_branch" != "main" ]]; then
    die "--ci-gate-main requires running on branch 'main'"
  fi

  note "verifying CI status for origin/main"
  git fetch origin main --quiet

  head_sha="$(git rev-parse --verify HEAD)"
  origin_main_sha="$(git rev-parse --verify origin/main)"
  if [[ "$head_sha" != "$origin_main_sha" ]]; then
    die "--ci-gate-main requires HEAD to match origin/main (pull/rebase main first)"
  fi

  ci_run_json="$(gh run list --workflow ci.yml --branch main --event push --commit "$origin_main_sha" --limit 20 --json databaseId,status,conclusion,url,headSha 2>/dev/null)" \
    || die "failed to query CI runs from GitHub"

  ci_run_result="$(
    python3 - "$origin_main_sha" "$ci_run_json" <<'PY'
from __future__ import annotations

import json
import sys

sha = sys.argv[1]
runs = json.loads(sys.argv[2])
if not runs:
    print(f"error:no CI run found for origin/main ({sha})")
    raise SystemExit(2)

run = runs[0]
run_head_sha = run.get("headSha")
status = run.get("status")
conclusion = run.get("conclusion")
url = run.get("url", "")

if run_head_sha and run_head_sha != sha:
    print(f"error:CI run SHA mismatch ({run_head_sha} != {sha}): {url}")
    raise SystemExit(5)
if status != "completed":
    print(f"error:CI run is not completed yet ({status}): {url}")
    raise SystemExit(3)
if conclusion != "success":
    print(f"error:CI run is not green ({conclusion}): {url}")
    raise SystemExit(4)

print(url)
PY
  )" || die "$ci_run_result"

  note "main CI is green: ${ci_run_result}"
  skip_checks=1
fi

python3 - "$version" <<'PY'
from __future__ import annotations

import re
import sys
from pathlib import Path

version = sys.argv[1]

paths = [Path("Cargo.toml")] + sorted(Path("crates").glob("*/Cargo.toml"))
updated: list[str] = []

for path in paths:
    text = path.read_text("utf-8")
    lines = text.splitlines()
    section = None
    out: list[str] = []
    changed = False

    for line in lines:
        stripped = line.strip()
        if stripped.startswith("[") and stripped.endswith("]"):
            section = stripped.strip("[]")
        if section in {"package", "workspace.package"}:
            match = re.match(r"(\s*version\s*=\s*)\"[^\"]+\"(.*)", line)
            if match:
                new_line = f"{match.group(1)}\"{version}\"{match.group(2)}"
                if new_line != line:
                    line = new_line
                    changed = True
        out.append(line)

    if changed:
        new_text = "\n".join(out)
        if text.endswith("\n"):
            new_text += "\n"
        path.write_text(new_text, "utf-8")
        updated.append(path.as_posix())

if not updated:
    print("error: no version fields were updated", file=sys.stderr)
    raise SystemExit(2)

print("info: updated versions in:")
for item in updated:
    print(f"- {item}")
PY

if [[ "$skip_readme" -eq 0 ]]; then
  if [[ -f README.md ]]; then
    python3 - "$version" <<'PY'
from __future__ import annotations

import re
import sys
from pathlib import Path

version = sys.argv[1]
tag = f"v{version}"
path = Path("README.md")
text = path.read_text("utf-8")
lines = text.splitlines()
out: list[str] = []
updated = False

patterns = (
    "tag like `v",
    "git tag -a v",
    "git push origin v",
)

for line in lines:
    if any(pat in line for pat in patterns):
        new_line = re.sub(r"v\d+\.\d+\.\d+", tag, line)
        if new_line != line:
            updated = True
        out.append(new_line)
    else:
        out.append(line)

if updated:
    new_text = "\n".join(out)
    if text.endswith("\n"):
        new_text += "\n"
    path.write_text(new_text, "utf-8")
else:
    print("warning: README release tag example not updated (pattern not found)", file=sys.stderr)
PY
  else
    note "README.md not found; skipping README update"
  fi
fi

checks_script="$repo_root/.codex/skills/nils-cli-checks/scripts/nils-cli-checks.sh"
if [[ "$skip_checks" -eq 0 ]]; then
  if [[ ! -f "$checks_script" ]]; then
    die "missing checks script: $checks_script"
  fi
  checks_runner="${NILS_CLI_TEST_RUNNER:-nextest}"
  if [[ -z "${NILS_CLI_TEST_RUNNER:-}" ]]; then
    note "NILS_CLI_TEST_RUNNER not set; defaulting to nextest for release checks"
  fi
  NILS_CLI_TEST_RUNNER="$checks_runner" "$checks_script"
else
  cargo check --workspace --locked
fi

git add -A

if git diff --cached --quiet; then
  die "no changes staged for commit"
fi

changed_files="$(git diff --cached --name-only)"

body_lines=()
body_lines+=("- Bump workspace and CLI crate versions to ${version}")
if [[ "$skip_readme" -eq 0 ]] && echo "$changed_files" | grep -qx "README.md"; then
  body_lines+=("- Update README release tag example to ${tag}")
fi
if echo "$changed_files" | grep -qx "Cargo.lock"; then
  body_lines+=("- Refresh Cargo.lock for workspace package versions")
fi

{
  printf "chore(release): bump cli versions to %s\n\n" "$version"
  for line in "${body_lines[@]}"; do
    printf "%s\n" "$line"
  done
} | semantic-commit commit

if [[ "$skip_push" -eq 0 ]]; then
  git push origin HEAD
fi

if git rev-parse -q --verify "refs/tags/${tag}" >/dev/null; then
  if [[ "$force_tag" -eq 1 ]]; then
    git tag -d "$tag"
    if [[ "$skip_push" -eq 0 ]]; then
      git push origin ":refs/tags/${tag}"
    fi
  else
    die "tag already exists: ${tag} (use --force-tag to replace)"
  fi
fi

git tag -a "$tag" -m "$tag"

if [[ "$skip_push" -eq 0 ]]; then
  git push origin "$tag"
fi

note "release tag ${tag} created"
