#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage:
  project-bump-version-tag-release --version X.Y.Z [options]

Prepares the release version bump as one local commit. It never tags, pushes,
publishes, or touches a remote; the sympoies-infra release broker owns that.

Options:
  --version X.Y.Z   Required. Accepts vX.Y.Z and normalizes to X.Y.Z.
  --full-checks     Run the full local audit stack (project-verify-required-checks.sh)
                    before commit; opt-in (slow). Default runs only the locked cargo check.
  --skip-checks     Deprecated alias of the default (locked cargo check only).
  --skip-readme     Do not update README release tag examples.
  --prepare-only    Apply the release-managed file transform, then stop before
                    validation and commit. Contract-test helper.
  --allow-dirty     Allow dirty release-managed files only.
  -h, --help        Show help.

Default behavior:
  Bumps the workspace and crate versions, refreshes Cargo.lock and the
  third-party artifacts, runs `cargo check --workspace --locked`, and commits
  `chore(release): bump cli versions to X.Y.Z` on the current branch.
USAGE
}

die() {
  echo "error: $*" >&2
  exit 1
}

note() {
  echo "info: $*" >&2
}

warn() {
  echo "warning: $*" >&2
}

refresh_lockfile() {
  # Re-pin only the workspace crate versions in Cargo.lock for the release
  # bump. `cargo update --workspace` rewrites the path/workspace members to
  # their new Cargo.toml versions and leaves every registry/transitive
  # dependency at its committed, CI-verified pin. A full `cargo
  # generate-lockfile` instead floats *all* deps to "latest compatible",
  # which both smuggles unreviewed dependency upgrades into a release bump
  # and can break the build outright (e.g. bitflags 2.12.0 hit dispatch2
  # 0.3.1's `bitflags!` recursion limit). The committed lock is the source of
  # truth for transitive pins; `cargo check --workspace --locked` below still
  # fails loudly if the workspace re-pin leaves the lock inconsistent.
  note "refreshing Cargo.lock workspace versions for release changes"
  cargo update --workspace
}

verify_workspace_locked() {
  note "verifying workspace with cargo check --workspace --locked"
  cargo check --workspace --locked
}

refresh_lockfile_and_verify_locked() {
  refresh_lockfile
  verify_workspace_locked
}

release_managed_paths() {
  printf '%s\n' Cargo.toml Cargo.lock
  for optional in README.md THIRD_PARTY_LICENSES.md THIRD_PARTY_NOTICES.md; do
    if [[ -e "$optional" || -L "$optional" ]]; then
      printf '%s\n' "$optional"
    fi
  done
  for manifest in crates/*/Cargo.toml; do
    if [[ -f "$manifest" ]]; then
      printf '%s\n' "$manifest"
    fi
  done
}

assert_allow_dirty_only_release_managed() {
  if [[ -z "$(git status --porcelain=v1 --untracked-files=all)" ]]; then
    return 0
  fi

  local managed_file dirty_file unexpected
  managed_file="$(mktemp)"
  dirty_file="$(mktemp)"
  release_managed_paths | sort -u >"$managed_file"
  git status --porcelain=v1 --untracked-files=all \
    | sed -E 's/^.. //' \
    | sort -u >"$dirty_file"
  unexpected="$(comm -23 "$dirty_file" "$managed_file" || true)"
  rm -f "$managed_file" "$dirty_file"

  if [[ -n "$unexpected" ]]; then
    die "--allow-dirty only permits release-managed paths; commit/stash these first: ${unexpected//$'\n'/, }"
  fi
}

sanitize_rust_build_env() {
  local wrapper="${RUSTC_WRAPPER:-}"
  if [[ -z "$wrapper" ]]; then
    return 0
  fi

  local wrapper_bin="$wrapper"
  if [[ "$wrapper" == */* ]]; then
    if [[ ! -x "$wrapper" ]]; then
      note "RUSTC_WRAPPER=${wrapper} is not executable; disabling it for release commands"
      unset RUSTC_WRAPPER
      return 0
    fi
  else
    if ! wrapper_bin="$(command -v "$wrapper" 2>/dev/null)"; then
      note "RUSTC_WRAPPER=${wrapper} is not available on PATH; disabling it for release commands"
      unset RUSTC_WRAPPER
      return 0
    fi
  fi

  local rustc_bin probe_output="" probe_summary=""
  rustc_bin="$(command -v rustc 2>/dev/null || true)"
  [[ -n "$rustc_bin" ]] || die "rustc is not available on PATH"

  if probe_output="$("$wrapper_bin" "$rustc_bin" -vV 2>&1)"; then
    return 0
  fi

  probe_summary="${probe_output%%$'\n'*}"
  note "RUSTC_WRAPPER=${wrapper} is not compatible with the active rustc; disabling it for release commands"
  if [[ -n "$probe_summary" ]]; then
    note "wrapper probe: ${probe_summary}"
  fi
  unset RUSTC_WRAPPER

  if [[ "$(basename "$wrapper_bin")" == "sccache" ]]; then
    export SCCACHE_DISABLE=1
    note "set SCCACHE_DISABLE=1 after disabling incompatible sccache wrapper"
  fi
}

refresh_third_party_artifacts_if_present() {
  local generator_script="scripts/generate-third-party-artifacts.sh"
  local artifacts=("THIRD_PARTY_LICENSES.md" "THIRD_PARTY_NOTICES.md")
  local tracked_count=0
  local artifact

  for artifact in "${artifacts[@]}"; do
    if git ls-files --error-unmatch "$artifact" >/dev/null 2>&1; then
      tracked_count=$((tracked_count + 1))
    fi
  done

  if [[ "$tracked_count" -eq 0 ]]; then
    return 0
  fi

  [[ -f "$generator_script" ]] \
    || die "tracked third-party artifacts require generator script: ${generator_script}"

  note "regenerating third-party artifacts for release changes"
  bash "$generator_script" --write
}

# === Argument parsing ========================================================

version=""
full_checks=0
skip_checks=0  # backward-compat alias of the default; tracked for usage notes only
skip_readme=0
prepare_only=0
allow_dirty=0

while [[ $# -gt 0 ]]; do
  case "${1:-}" in
    --version)
      if [[ $# -lt 2 ]]; then
        die "--version requires a value"
      fi
      version="${2:-}"
      shift 2
      ;;
    --full-checks)
      full_checks=1
      shift
      ;;
    --skip-checks)
      skip_checks=1
      shift
      ;;
    --skip-readme)
      skip_readme=1
      shift
      ;;
    --prepare-only)
      prepare_only=1
      shift
      ;;
    --allow-dirty)
      allow_dirty=1
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

required_commands=(git python3 cargo)
if [[ "$prepare_only" -eq 0 ]]; then
  required_commands+=(semantic-commit git-scope)
fi
for cmd in "${required_commands[@]}"; do
  if ! command -v "$cmd" >/dev/null 2>&1; then
    die "missing required command: ${cmd}"
  fi
done

repo_root="$(git rev-parse --show-toplevel 2>/dev/null || true)"
if [[ -z "$repo_root" ]]; then
  die "must run inside a git work tree"
fi

cd "$repo_root"

if [[ ! -f Cargo.toml ]]; then
  die "Cargo.toml not found in repo root"
fi

sanitize_rust_build_env

if [[ "$allow_dirty" -eq 0 ]]; then
  if [[ -n "$(git status --porcelain)" ]]; then
    die "working tree is not clean; commit/stash changes or use --allow-dirty"
  fi
else
  assert_allow_dirty_only_release_managed
fi

if [[ "$skip_checks" -eq 1 && "$full_checks" -eq 0 ]]; then
  note "--skip-checks is a deprecated alias of the default (locked cargo check only); ignoring"
fi

python3 - "$version" <<'PY'
from __future__ import annotations

import re
import sys
from pathlib import Path

version = sys.argv[1]

paths = [Path("Cargo.toml")] + sorted(Path("crates").glob("*/Cargo.toml"))
updated: list[str] = []
version_fields_found = 0
dep_fields_seen = 0


def extract_package_name(path: Path) -> str | None:
    section = None
    for line in path.read_text("utf-8").splitlines():
        stripped = line.strip()
        if stripped.startswith("[") and stripped.endswith("]"):
            section = stripped.strip("[]")
            continue
        if section == "package":
            match = re.match(r'\s*name\s*=\s*"([^"]+)"\s*$', line)
            if match:
                return match.group(1)
    return None


workspace_packages = {name for path in paths if (name := extract_package_name(path))}

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
                version_fields_found += 1
                new_line = f"{match.group(1)}\"{version}\"{match.group(2)}"
                if new_line != line:
                    line = new_line
                    changed = True

        dep_match = re.match(r'(\s*([A-Za-z0-9_.-]+)\s*=\s*\{)(.*)(\}\s*(?:#.*)?)$', line)
        if dep_match:
            dep_fields_seen += 1
            dep_key = dep_match.group(2).strip('"')
            body = dep_match.group(3)
            suffix = dep_match.group(4)
            package_match = re.search(r'\bpackage\s*=\s*"([^"]+)"', body)
            package_name = package_match.group(1) if package_match else dep_key

            if package_name in workspace_packages and re.search(r"\bpath\s*=", body):
                if re.search(r"\bversion\s*=", body):
                    new_body = re.sub(
                        r'(\bversion\s*=\s*)"[^"]+"',
                        rf'\1"{version}"',
                        body,
                        count=1,
                    )
                else:
                    path_match = re.search(r"\bpath\s*=", body)
                    if path_match:
                        idx = path_match.start()
                        new_body = body[:idx] + f'version = "{version}", ' + body[idx:]
                    else:
                        new_body = body

                if new_body != body:
                    line = f"{dep_match.group(1)}{new_body}{suffix}"
                    changed = True
        out.append(line)

    if changed:
        new_text = "\n".join(out)
        if text.endswith("\n"):
            new_text += "\n"
        path.write_text(new_text, "utf-8")
        updated.append(path.as_posix())

if not updated:
    if version_fields_found == 0 and dep_fields_seen == 0:
        print("error: no version fields found in Cargo manifests or dependency tables", file=sys.stderr)
        raise SystemExit(2)
    print("info: all manifest versions already set to target; continuing")
else:
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
matched = False

for line in lines:
    if any(pat in line for pat in patterns):
        matched = True
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
elif not matched:
    print("warning: README release tag example not updated (pattern not found)", file=sys.stderr)
PY
  else
    note "README.md not found; skipping README update"
  fi
fi

checks_script="$repo_root/.agents/skills/project-verify-required-checks/scripts/project-verify-required-checks.sh"
refresh_lockfile
# Keep third-party artifacts aligned with the new lockfile; CI re-audits drift on the bump commit.
refresh_third_party_artifacts_if_present

if [[ "$prepare_only" -eq 1 ]]; then
  note "release-managed files prepared for ${version}; stopping before validation and commit"
  exit 0
fi

if [[ "$full_checks" -eq 1 ]]; then
  if [[ ! -f "$checks_script" ]]; then
    die "missing checks script: $checks_script"
  fi
  checks_runner="${NILS_CLI_TEST_RUNNER:-nextest}"
  if [[ -z "${NILS_CLI_TEST_RUNNER:-}" ]]; then
    note "NILS_CLI_TEST_RUNNER not set; defaulting to nextest for --full-checks"
  fi
  NILS_CLI_TEST_RUNNER="$checks_runner" "$checks_script"
else
  verify_workspace_locked
fi

# Re-run artifact generation after checks in case lockfile changed during the check flow.
refresh_third_party_artifacts_if_present

# Stage only the files this skill is expected to produce. Using `git add -A`
# would sweep in unrelated runtime state (e.g. `.claude/` session locks).
stage_paths=()
while IFS= read -r release_path; do
  stage_paths+=("$release_path")
done < <(release_managed_paths)
git add -- "${stage_paths[@]}"

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
if echo "$changed_files" | grep -Eq "^(THIRD_PARTY_LICENSES\.md|THIRD_PARTY_NOTICES\.md)$"; then
  body_lines+=("- Regenerate third-party artifacts for updated lockfile inputs")
fi

{
  printf "chore(release): bump cli versions to %s\n\n" "$version"
  for line in "${body_lines[@]}"; do
    printf "%s\n" "$line"
  done
} | semantic-commit commit

note "release bump for ${version} committed locally; tagging and publishing belong to the release broker"
