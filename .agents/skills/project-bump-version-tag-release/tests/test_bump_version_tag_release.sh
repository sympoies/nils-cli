#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
skill_root="$(cd "${script_dir}/.." && pwd)"
entrypoint="${skill_root}/scripts/project-bump-version-tag-release.sh"

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
    "${repo_dir}/.agents/skills/project-verify-required-checks/scripts"

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

  cat > "${repo_dir}/.agents/skills/project-verify-required-checks/scripts/project-verify-required-checks.sh" <<'EOF'
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
  chmod +x "${repo_dir}/.agents/skills/project-verify-required-checks/scripts/project-verify-required-checks.sh"

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
  update)
    # Release bumps re-pin workspace crate versions via `cargo update
    # --workspace`, which also writes Cargo.lock if absent.
    [[ -z "${RUSTC_WRAPPER:-}" ]] || {
      echo "RUSTC_WRAPPER should be unset before cargo update" >&2
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

assert_no_tag_and_one_release_commit() {
  local repo="$1"
  [[ -z "$(git -C "$repo" tag --list)" ]] || fail "release preparation must not create a tag"
  git -C "$repo" log -1 --format=%s | rg -q '^chore\(release\): bump cli versions to 0\.6\.5$' \
    || fail "expected the release bump commit at HEAD"
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
      "$entrypoint" --version v0.6.5 --full-checks
  ) >"${tmp}/stdout.log" 2>"${stderr_file}"

  local order_file="${tmp}/order.log"
  rg -n 'cargo:update --workspace|checks:start' "$log_file" >"$order_file"
  assert_contains "$order_file" '1:cargo:update --workspace'
  assert_contains "$order_file" '3:checks:start'
  assert_not_contains "$log_file" 'cargo:RUSTC_WRAPPER=bad-wrapper'
  assert_contains "$stderr_file" 'disabling it for release commands'
  assert_contains "${repo}/README.md" 'v0.6.5'

  assert_no_tag_and_one_release_commit "$repo"
}

test_default_path_skips_full_audit_and_runs_locked_check() {
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

  (
    cd "$repo"
    env -u RUSTC_WRAPPER \
      PATH="${bin_dir}:$PATH" \
      MOCK_LOG="$log_file" \
      "$entrypoint" --version v0.6.5
  ) >"${tmp}/stdout.log" 2>"${stderr_file}"

  assert_contains "$log_file" 'cargo:update --workspace'
  assert_contains "$log_file" 'cargo:check --workspace --locked'
  # Full audit stack must NOT run in the new default path.
  if rg -q 'checks:start' "$log_file"; then
    fail "default path unexpectedly ran the full audit stack"
  fi

  assert_no_tag_and_one_release_commit "$repo"
}

test_skip_checks_is_deprecated_alias_of_default() {
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

  (
    cd "$repo"
    env -u RUSTC_WRAPPER \
      PATH="${bin_dir}:$PATH" \
      MOCK_LOG="$log_file" \
      "$entrypoint" --version v0.6.5 --skip-checks
  ) >"${tmp}/stdout.log" 2>"${stderr_file}"

  assert_contains "$stderr_file" '--skip-checks is a deprecated alias'
  assert_contains "$log_file" 'cargo:check --workspace --locked'
  if rg -q 'checks:start' "$log_file"; then
    fail "--skip-checks unexpectedly ran the full audit stack"
  fi
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
      "$entrypoint" --version v0.6.5 --skip-checks
  ) >"${tmp}/stdout.log" 2>"${stderr_file}"

  assert_not_contains "$stderr_file" 'warning: README release tag example not updated'
  assert_contains "${repo}/README.md" 'v0.6.5'
  assert_contains "$log_file" 'cargo:update --workspace'
  assert_contains "$log_file" 'cargo:check --workspace --locked'
}

test_allow_dirty_rejects_non_release_managed_paths() {
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

  echo "temporary docs fix" >"${repo}/crates/codex-cli/README.md"

  set +e
  (
    cd "$repo"
    env -u RUSTC_WRAPPER \
      PATH="${bin_dir}:$PATH" \
      MOCK_LOG="$log_file" \
      "$entrypoint" --version v0.6.5 --skip-checks --allow-dirty \
      >"${tmp}/stdout.log" 2>"${stderr_file}"
  )
  local rc=$?
  set -e

  if [[ "$rc" -eq 0 ]]; then
    fail "expected --allow-dirty with non-release-managed paths to exit non-zero"
  fi
  assert_contains "$stderr_file" '--allow-dirty only permits release-managed paths'
  assert_contains "$stderr_file" 'crates/codex-cli/README.md'
  if [[ -f "$log_file" ]]; then
    assert_not_contains "$log_file" 'cargo:'
  fi
}

test_release_flags_that_publish_are_rejected() {
  local tmp repo bin_dir flag rc
  tmp="$(mktemp -d)"
  repo="${tmp}/repo"
  bin_dir="${tmp}/bin"

  mkdir -p "$repo" "$bin_dir"
  create_temp_repo "$repo" "v0.6.4"
  create_mock_cargo "$bin_dir"
  create_mock_semantic_commit "$bin_dir"
  create_mock_git_scope "$bin_dir"

  # `--skip-push` and the other `--skip-*` flags used to gate publication; with
  # no publication path left, none of them is a supported flag.
  for flag in \
    --skip-push --direct-push --force-tag --from-tap --skip-tap --skip-tap-wait \
    --skip-tap-tag --skip-ci-wait --skip-dev-clean --skip-local-brew-upgrade \
    --ci-gate-main "--release-branch chore/x" "--tap-repo o/r" "--tap-dir /x" \
    "--tap-formula f"; do
    set +e
    (
      cd "$repo"
      env -u RUSTC_WRAPPER PATH="${bin_dir}:$PATH" MOCK_LOG="${tmp}/mock.log" \
        "$entrypoint" --version v0.6.5 $flag
    ) >"${tmp}/stdout.log" 2>"${tmp}/stderr.log"
    rc=$?
    set -e
    [[ "$rc" -ne 0 ]] || fail "expected ${flag} to be rejected"
    assert_contains "${tmp}/stderr.log" 'unknown argument'
  done
  assert_contains "${repo}/README.md" 'v0.6.4'
  [[ -z "$(git -C "$repo" tag --list)" ]] || fail "a rejected flag must not create a tag"
}

test_usage_documents_preparation_only() {
  local tmp
  tmp="$(mktemp -d)"
  "$entrypoint" --help >"${tmp}/help.log"
  assert_not_contains "${tmp}/help.log" 'tap|--direct-push|--skip-push|--force-tag|brew'
  assert_contains "${tmp}/help.log" '--prepare-only'
}

run_all() {
  if [[ ! -f "${skill_root}/SKILL.md" ]]; then
    fail "missing SKILL.md"
  fi
  if [[ ! -f "$entrypoint" ]]; then
    fail "missing entrypoint script"
  fi

  local tests=(
    test_full_checks_refresh_lockfile_and_disable_bad_wrapper
    test_default_path_skips_full_audit_and_runs_locked_check
    test_skip_checks_is_deprecated_alias_of_default
    test_readme_already_at_target_is_not_warned
    test_allow_dirty_rejects_non_release_managed_paths
    test_release_flags_that_publish_are_rejected
    test_usage_documents_preparation_only
  )

  # `test_full_checks...` drives the real toolchain: it probes the active
  # `rustc` (RUSTC_WRAPPER compatibility check). Skip it (rather than fail) when
  # the Rust toolchain is not installed, so the mock-driven suite still runs on
  # toolchain-less hosts.
  local toolchain_ready=1
  if ! command -v rustc >/dev/null 2>&1 || ! command -v cargo >/dev/null 2>&1; then
    toolchain_ready=0
  fi

  local failed=0 t
  for t in "${tests[@]}"; do
    case "$t" in
      test_full_checks_refresh_lockfile_and_disable_bad_wrapper)
        if [[ "$toolchain_ready" -eq 0 ]]; then
          echo "SKIP ${t} (requires rustc+cargo on PATH)"
          continue
        fi
        ;;
    esac
    if ( set -e; "$t" ); then
      echo "PASS ${t}"
    else
      echo "FAIL ${t}" >&2
      failed=1
    fi
  done

  if [[ "$failed" -ne 0 ]]; then
    exit 1
  fi
  echo "ok: project skill smoke checks passed"
}

if [[ "${BASH_SOURCE[0]}" == "${0}" ]]; then
  run_all
fi
