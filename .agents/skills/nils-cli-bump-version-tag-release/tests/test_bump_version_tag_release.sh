#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
skill_root="$(cd "${script_dir}/.." && pwd)"
entrypoint="${skill_root}/scripts/nils-cli-bump-version-tag-release.sh"

fail() {
  echo "error: $*" >&2
  exit 1
}

assert_contains() {
  local file="$1"
  local pattern="$2"
  if ! rg -q -- "$pattern" "$file"; then
    echo "error: expected pattern '$pattern' in $file" >&2
    sed -n '1,220p' "$file" >&2 || true
    exit 1
  fi
}

assert_not_contains() {
  local file="$1"
  local pattern="$2"
  if rg -q -- "$pattern" "$file"; then
    echo "error: unexpected pattern '$pattern' in $file" >&2
    sed -n '1,220p' "$file" >&2 || true
    exit 1
  fi
}

create_temp_repo() {
  local repo_dir="$1"
  local readme_tag="$2"

  git init --initial-branch=main "$repo_dir" >/dev/null
  git -C "$repo_dir" config user.email "test@example.com"
  git -C "$repo_dir" config user.name "Test User"

  mkdir -p \
    "${repo_dir}/crates/codex-cli" \
    "${repo_dir}/scripts" \
    "${repo_dir}/.agents/skills/nils-cli-verify-required-checks/scripts"

  cat > "${repo_dir}/Cargo.toml" <<'EOF'
[workspace]
members = ["crates/codex-cli"]
resolver = "2"

[workspace.package]
version = "0.6.4"
EOF

  cat > "${repo_dir}/crates/codex-cli/Cargo.toml" <<'EOF'
[package]
name = "nils-codex-cli"
version = "0.6.4"
edition = "2021"
EOF

  cat > "${repo_dir}/README.md" <<EOF
To trigger a release build, push a tag like \`${readme_tag}\`:

- \`git tag -a ${readme_tag} -m "${readme_tag}"\`
- \`git push origin ${readme_tag}\`
EOF

  cat > "${repo_dir}/THIRD_PARTY_LICENSES.md" <<'EOF'
licenses-old
EOF

  cat > "${repo_dir}/THIRD_PARTY_NOTICES.md" <<'EOF'
notices-old
EOF

  cat > "${repo_dir}/scripts/generate-third-party-artifacts.sh" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

[[ "${1:-}" == "--write" ]] || exit 1
echo "licenses-generated" > THIRD_PARTY_LICENSES.md
echo "notices-generated" > THIRD_PARTY_NOTICES.md
echo "PASS: regenerated THIRD_PARTY_LICENSES.md THIRD_PARTY_NOTICES.md"
EOF
  chmod +x "${repo_dir}/scripts/generate-third-party-artifacts.sh"

  cat > "${repo_dir}/.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

log_file="${MOCK_LOG:?}"
echo "checks:start" >> "$log_file"
[[ -f Cargo.lock ]] || {
  echo "missing Cargo.lock before checks" >&2
  exit 1
}
[[ -z "${RUSTC_WRAPPER:-}" ]] || {
  echo "RUSTC_WRAPPER should be unset for checks" >&2
  exit 1
}
echo "checks:ok" >> "$log_file"
EOF
  chmod +x "${repo_dir}/.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh"

  git -C "$repo_dir" add .
  git -C "$repo_dir" commit -m "init" >/dev/null
}

create_mock_cargo() {
  local bin_dir="$1"
  cat > "${bin_dir}/cargo" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

log_file="${MOCK_LOG:?}"
echo "cargo:$*" >> "$log_file"
echo "cargo:RUSTC_WRAPPER=${RUSTC_WRAPPER-}" >> "$log_file"

case "${1:-}" in
  generate-lockfile)
    [[ -z "${RUSTC_WRAPPER:-}" ]] || {
      echo "RUSTC_WRAPPER should be unset before cargo generate-lockfile" >&2
      exit 1
    }
    echo "# mock lockfile" > Cargo.lock
    ;;
  check)
    [[ -f Cargo.lock ]] || {
      echo "missing Cargo.lock before cargo check" >&2
      exit 1
    }
    ;;
  *)
    echo "unexpected cargo command: $*" >&2
    exit 1
    ;;
esac
EOF
  chmod +x "${bin_dir}/cargo"
}

create_mock_semantic_commit() {
  local bin_dir="$1"
  cat > "${bin_dir}/semantic-commit" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

if [[ "${1:-}" != "commit" ]]; then
  echo "unexpected semantic-commit command: $*" >&2
  exit 1
fi

msg_file="$(mktemp)"
cat > "$msg_file"
git commit -F "$msg_file" >/dev/null
rm -f "$msg_file"
EOF
  chmod +x "${bin_dir}/semantic-commit"
}

create_mock_git_scope() {
  local bin_dir="$1"
  cat > "${bin_dir}/git-scope" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
exit 0
EOF
  chmod +x "${bin_dir}/git-scope"
}

create_mock_bad_wrapper() {
  local bin_dir="$1"
  cat > "${bin_dir}/bad-wrapper" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
echo "Compiler not supported: mock wrapper" >&2
exit 1
EOF
  chmod +x "${bin_dir}/bad-wrapper"
}

test_full_checks_refresh_lockfile_and_disable_bad_wrapper() {
  local tmp repo bin_dir log_file stderr_file
  tmp="$(mktemp -d)"
  repo="${tmp}/repo"
  bin_dir="${tmp}/bin"
  log_file="${tmp}/mock.log"
  stderr_file="${tmp}/stderr.log"

  mkdir -p "$repo" "$bin_dir"
  create_temp_repo "$repo" "v0.6.4"
  create_mock_cargo "$bin_dir"
  create_mock_semantic_commit "$bin_dir"
  create_mock_git_scope "$bin_dir"
  create_mock_bad_wrapper "$bin_dir"

  (
    cd "$repo"
    PATH="${bin_dir}:$PATH" \
      MOCK_LOG="$log_file" \
      RUSTC_WRAPPER="bad-wrapper" \
      "$entrypoint" --version v0.6.5 --skip-push
  ) >"${tmp}/stdout.log" 2>"${stderr_file}"

  local order_file="${tmp}/order.log"
  rg -n 'cargo:generate-lockfile|checks:start' "$log_file" >"$order_file"
  assert_contains "$order_file" '1:cargo:generate-lockfile'
  assert_contains "$order_file" '3:checks:start'
  assert_not_contains "$log_file" 'cargo:RUSTC_WRAPPER=bad-wrapper'
  assert_contains "$stderr_file" 'disabling it for release commands'
  assert_contains "${repo}/README.md" 'v0.6.5'

  git -C "$repo" rev-parse -q --verify "refs/tags/v0.6.5" >/dev/null \
    || fail "expected tag v0.6.5 to exist"
}

test_readme_already_at_target_is_not_warned() {
  local tmp repo bin_dir log_file stderr_file
  tmp="$(mktemp -d)"
  repo="${tmp}/repo"
  bin_dir="${tmp}/bin"
  log_file="${tmp}/mock.log"
  stderr_file="${tmp}/stderr.log"

  mkdir -p "$repo" "$bin_dir"
  create_temp_repo "$repo" "v0.6.5"
  create_mock_cargo "$bin_dir"
  create_mock_semantic_commit "$bin_dir"
  create_mock_git_scope "$bin_dir"

  (
    cd "$repo"
    env -u RUSTC_WRAPPER \
      PATH="${bin_dir}:$PATH" \
      MOCK_LOG="$log_file" \
      "$entrypoint" --version v0.6.5 --skip-checks --skip-push
  ) >"${tmp}/stdout.log" 2>"${stderr_file}"

  assert_not_contains "$stderr_file" 'warning: README release tag example not updated'
  assert_contains "${repo}/README.md" 'v0.6.5'
  assert_contains "$log_file" 'cargo:generate-lockfile'
  assert_contains "$log_file" 'cargo:check --workspace --locked'
}

if [[ ! -f "${skill_root}/SKILL.md" ]]; then
  fail "missing SKILL.md"
fi
if [[ ! -f "$entrypoint" ]]; then
  fail "missing entrypoint script"
fi

test_full_checks_refresh_lockfile_and_disable_bad_wrapper
test_readme_already_at_target_is_not_warned

echo "ok: project skill smoke checks passed"
