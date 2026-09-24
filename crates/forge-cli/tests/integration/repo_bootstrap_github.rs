//! GitHub repository bootstrap contract coverage.

use std::fs;
use std::path::PathBuf;

use pretty_assertions::assert_eq;

use super::support::{StubEnv, parse_envelope, run_forge_cli};

const SHA: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

struct Fixture {
    stub: StubEnv,
    readme: PathBuf,
    reason: PathBuf,
    exists: PathBuf,
    remote_sha: PathBuf,
    gh_log: PathBuf,
    git_log: PathBuf,
}

impl Fixture {
    fn new(existing: bool) -> Self {
        let stub = StubEnv::new();
        let readme = stub.tempdir.path().join("README.md");
        let reason = stub.tempdir.path().join("authorization.txt");
        let exists = stub.tempdir.path().join("remote-exists");
        let remote_sha = stub.tempdir.path().join("remote-sha");
        let gh_log = stub.tempdir.path().join("gh.log");
        let git_log = stub.tempdir.path().join("git.log");
        let state_home = stub.tempdir.path().join("state");
        let default_branch_file = stub.tempdir.path().join("default-branch");
        fs::write(&readme, "# Widgets\n").expect("README fixture");
        fs::write(
            &reason,
            "Operator authorized this exact repository bootstrap.\n",
        )
        .expect("authorization fixture");
        if existing {
            fs::write(&exists, "yes\n").expect("existing repository fixture");
        }
        let gh = stub.write_stub("gh-bootstrap", r#"#!/bin/sh
if [ "$1" = auth ]; then
  printf '%s\n' 'fixture-token-value'
  exit 0
fi
printf 'GH_HOST=%s %s\n' "$GH_HOST" "$*" >> "$GH_TEST_LOG"
endpoint=
method=GET
include=no
for arg in "$@"; do
  case "$arg" in
    --include) include=yes ;;
    user|user/repos|orgs/*|repos/*) endpoint=$arg ;;
    POST|PATCH) method=$arg ;;
  esac
done
response_header() {
  if [ "$include" = yes ]; then
    printf 'HTTP/2.0 %s\r\nContent-Type: application/json\r\n\r\n' "$1"
  fi
}
not_found() { response_header '404 Not Found'; printf '%s\n' '{"message":"Not Found"}'; exit 1; }
repo_json() {
  branch=${GH_TEST_DEFAULT_BRANCH:-main}
  if [ -f "$GH_TEST_BRANCH_FILE" ]; then branch=$(cat "$GH_TEST_BRANCH_FILE"); fi
  visibility=${GH_TEST_VISIBILITY:-public}
  if [ -f "$GH_TEST_REMOTE_SHA" ] && [ -n "${GH_TEST_VISIBILITY_AFTER_PUSH:-}" ]; then visibility=$GH_TEST_VISIBILITY_AFTER_PUSH; fi
  response_header '200 OK'
  printf '{"owner":{"login":"sympoies","type":"%s"},"name":"widgets","private":%s,"visibility":"%s","clone_url":"https://github.com/sympoies/widgets.git","default_branch":"%s"}\n' "${GH_TEST_OWNER_TYPE:-Organization}" "${GH_TEST_PRIVATE:-false}" "$visibility" "$branch"
}
case "$endpoint" in
  user)
    if [ "${GH_TEST_TRANSPORT_FAILURE:-}" = yes ]; then
      printf '%s\n' 'gh: simulated connection failure' >&2
      exit 1
    fi
    response_header '200 OK'; printf '%s\n' '{"login":"operator"}' ;;
  user/repos|orgs/sympoies/repos)
    [ "$method" = POST ] || exit 2
    : > "$GH_TEST_EXISTS"
    repo_json
    ;;
  repos/sympoies/widgets)
    [ -f "$GH_TEST_EXISTS" ] || not_found
    if [ "$method" = PATCH ]; then
      printf '%s\n' 'main' > "$GH_TEST_BRANCH_FILE"
    fi
    repo_json
    ;;
  repos/sympoies/widgets/git/refs)
    [ -f "$GH_TEST_REMOTE_SHA" ] || {
      response_header '409 Conflict'
      printf '%s\n' '{"message":"Git Repository is empty."}'
      exit 1
    }
    response_header '200 OK'
    if [ "${GH_TEST_EXTRA_REF:-}" = yes ]; then
      printf '[{"ref":"refs/heads/main","object":{"sha":"%s"}},{"ref":"refs/tags/other","object":{"sha":"%s"}}]\n' "$(cat "$GH_TEST_REMOTE_SHA")" "$GH_TEST_SHA"
    else
      printf '[{"ref":"refs/heads/main","object":{"sha":"%s"}}]\n' "$(cat "$GH_TEST_REMOTE_SHA")"
    fi
    ;;
  repos/sympoies/widgets/git/ref/heads/main)
    [ -f "$GH_TEST_REMOTE_SHA" ] || not_found
    response_header '200 OK'
    printf '{"ref":"refs/heads/main","object":{"sha":"%s"}}\n' "$(cat "$GH_TEST_REMOTE_SHA")"
    ;;
  repos/sympoies/widgets/git/commits/*)
    response_header '200 OK'
    printf '{"sha":"%s","verification":{"verified":%s}}\n' "$GH_TEST_SHA" "${GH_TEST_VERIFIED:-true}"
    ;;
  *) printf 'unexpected gh endpoint: %s\n' "$endpoint" >&2; exit 2 ;;
esac
"#);
        let git = stub.write_stub(
            "git-bootstrap",
            r#"#!/bin/sh
printf '%s\n' "$*" >> "$GH_TEST_GIT_LOG"
case "$*" in
  *"rev-parse"*"HEAD^{commit}"*) printf '%s\n' "$GH_TEST_SHA" ;;
  *"rev-list --parents -n 1"*) printf '%s\n' "$GH_TEST_SHA" ;;
  *"log -1 --format=%G?"*) printf '%s\n' "${GH_TEST_LOCAL_SIGNATURE:-G}" ;;
  *"push "*)
    [ "$FORGE_CLI_BOOTSTRAP_USERNAME" = x-access-token ] || exit 9
    [ "$FORGE_CLI_BOOTSTRAP_GITHUB_TOKEN" = fixture-token-value ] || exit 9
    printf '%s\n' "$GH_TEST_SHA" > "$GH_TEST_REMOTE_SHA"
    if [ "${GH_TEST_PUSH_MODE:-success}" = ambiguous ]; then
      printf '%s\n' 'simulated lost push response' >&2
      exit 1
    fi
    ;;
esac
exit 0
"#,
        );
        let semantic = stub.write_stub(
            "semantic-bootstrap",
            r#"#!/bin/sh
printf '{"ok":true,"commit":{"sha":"%s"}}\n' "$GH_TEST_SHA"
"#,
        );
        let stub = stub
            .env("FORGE_CLI_GH_BIN", gh.to_string_lossy())
            .env("FORGE_CLI_GIT_BIN", git.to_string_lossy())
            .env("FORGE_CLI_SEMANTIC_COMMIT_BIN", semantic.to_string_lossy())
            .env("XDG_STATE_HOME", state_home.to_string_lossy())
            .env("GH_TEST_EXISTS", exists.to_string_lossy())
            .env("GH_TEST_REMOTE_SHA", remote_sha.to_string_lossy())
            .env("GH_TEST_BRANCH_FILE", default_branch_file.to_string_lossy())
            .env("GH_TEST_LOG", gh_log.to_string_lossy())
            .env("GH_TEST_GIT_LOG", git_log.to_string_lossy())
            .env("GH_TEST_SHA", SHA);
        Self {
            stub,
            readme,
            reason,
            exists,
            remote_sha,
            gh_log,
            git_log,
        }
    }

    fn run(&self, existing: bool, resume: bool) -> super::support::CmdOutput {
        self.run_with_visibility(existing, resume, "public")
    }

    fn run_with_visibility(
        &self,
        existing: bool,
        resume: bool,
        visibility: &str,
    ) -> super::support::CmdOutput {
        let mut args = vec![
            "--provider",
            "github",
            "--repo",
            "sympoies/widgets",
            "--format",
            "json",
            "repo",
            "bootstrap",
            "--owner-kind",
            "org",
            "--visibility",
            visibility,
            "--default-branch",
            "main",
            "--file",
            self.readme.to_str().expect("UTF-8 path"),
            "--message",
            "chore: initialize repository",
            "--reason-file",
            self.reason.to_str().expect("UTF-8 path"),
        ];
        if existing {
            args.push("--existing-empty");
        }
        if resume {
            args.push("--resume");
        }
        run_forge_cli(&self.stub, &args)
    }
}

#[test]
fn github_public_existing_empty_bootstrap_has_a_bounded_dry_run() {
    let stub = StubEnv::new();
    let readme = stub.tempdir.path().join("README.md");
    let reason = stub.tempdir.path().join("authorization.txt");
    fs::write(&readme, "# Widgets\n").expect("README fixture");
    fs::write(
        &reason,
        "Operator authorized adoption of this empty public repository.\n",
    )
    .expect("authorization fixture");
    let output = run_forge_cli(
        &stub,
        &[
            "--provider",
            "github",
            "--repo",
            "sympoies/widgets",
            "--format",
            "json",
            "--dry-run",
            "repo",
            "bootstrap",
            "--owner-kind",
            "org",
            "--visibility",
            "public",
            "--existing-empty",
            "--default-branch",
            "main",
            "--file",
            readme.to_str().expect("UTF-8 path"),
            "--message",
            "chore: initialize repository",
            "--reason-file",
            reason.to_str().expect("UTF-8 path"),
        ],
    );
    assert_eq!(output.code, 0, "stderr={}", output.stderr);
    let envelope = parse_envelope(&output.stdout);
    assert_eq!(envelope["data"]["provider"], "github");
    assert_eq!(envelope["data"]["private"], false);
    assert_eq!(envelope["data"]["existing_empty"], true);
    assert_eq!(envelope["data"]["repository"], "sympoies/widgets");
}

#[test]
fn github_adopts_exact_empty_public_repo_with_signed_root_and_idempotent_resume() {
    let fixture = Fixture::new(true);
    let first = fixture.run(true, false);
    assert_eq!(
        first.code, 0,
        "stdout={} stderr={}",
        first.stdout, first.stderr
    );
    let first = parse_envelope(&first.stdout);
    assert_eq!(first["data"]["private"], false);
    assert_eq!(first["data"]["remote_created"], false);
    assert_eq!(first["data"]["root_commit_sha"], SHA);
    assert_eq!(first["data"]["signature_verified"], true);
    assert_eq!(
        fs::read_to_string(&fixture.remote_sha)
            .expect("remote SHA")
            .trim(),
        SHA
    );
    let second = fixture.run(true, true);
    assert_eq!(
        second.code, 0,
        "stdout={} stderr={}",
        second.stdout, second.stderr
    );
    assert_eq!(parse_envelope(&second.stdout)["data"]["idempotent"], true);
    assert!(
        fs::read_to_string(&fixture.gh_log)
            .expect("gh log")
            .contains("api --include"),
        "GitHub API calls must request response headers"
    );
    let git_log = fs::read_to_string(&fixture.git_log).expect("git log");
    assert_eq!(git_log.matches("push --porcelain").count(), 1);
    assert!(!first.to_string().contains("fixture-token-value"));
}

#[test]
fn github_preserves_non_http_gh_failure_diagnostic() {
    let mut fixture = Fixture::new(true);
    fixture
        .stub
        .envs
        .push(("GH_TEST_TRANSPORT_FAILURE".into(), "yes".into()));
    let result = fixture.run(true, false);
    assert_ne!(result.code, 0);
    let error = &parse_envelope(&result.stdout)["error"];
    assert_eq!(error["code"], "bootstrap_github_api_failed");
    assert!(error.to_string().contains("simulated connection failure"));
    assert!(!fixture.remote_sha.exists());
}

#[test]
fn github_does_not_repeat_an_indeterminate_first_push_on_resume() {
    let fixture = Fixture::new(true);
    let first = fixture.run(true, false);
    assert_eq!(first.code, 0, "stdout={}", first.stdout);
    let first_data = parse_envelope(&first.stdout);
    let receipt_path = first_data["data"]["receipt"]
        .as_str()
        .expect("receipt path");
    let mut receipt: serde_json::Value =
        serde_json::from_slice(&fs::read(receipt_path).expect("receipt")).expect("receipt JSON");
    receipt["complete"] = false.into();
    fs::write(receipt_path, serde_json::to_vec_pretty(&receipt).unwrap()).expect("write receipt");
    fs::remove_file(&fixture.remote_sha).expect("simulate absent remote ref");

    let resumed = fixture.run(true, true);
    assert_ne!(resumed.code, 0);
    assert_eq!(
        parse_envelope(&resumed.stdout)["error"]["code"],
        "bootstrap_push_indeterminate"
    );
    let git_log = fs::read_to_string(&fixture.git_log).expect("git log");
    assert_eq!(git_log.matches("push --porcelain").count(), 1);
}

#[test]
fn github_creates_an_empty_public_org_repo_before_first_push() {
    let fixture = Fixture::new(false);
    let result = fixture.run(false, false);
    assert_eq!(
        result.code, 0,
        "stdout={} stderr={}",
        result.stdout, result.stderr
    );
    assert_eq!(
        parse_envelope(&result.stdout)["data"]["remote_created"],
        true
    );
    assert!(fixture.exists.exists());
    let log = fs::read_to_string(&fixture.gh_log).expect("gh log");
    assert_eq!(log.matches("orgs/sympoies/repos").count(), 1);
    assert!(log.contains("private=false"));
    assert!(log.contains("auto_init=false"));
}

#[test]
fn github_refuses_adoption_when_other_refs_exist() {
    let fixture = Fixture::new(true);
    fs::write(&fixture.remote_sha, SHA).expect("existing ref fixture");
    let result = fixture.run(true, false);
    assert_ne!(result.code, 0);
    assert_eq!(
        parse_envelope(&result.stdout)["error"]["code"],
        "repository_not_empty"
    );
    assert!(
        !fs::read_to_string(&fixture.git_log)
            .unwrap_or_default()
            .contains("push ")
    );
}

#[test]
fn github_refuses_visibility_mismatch_before_push() {
    let mut fixture = Fixture::new(true);
    fixture
        .stub
        .envs
        .push(("GH_TEST_PRIVATE".to_string(), "true".to_string()));
    let result = fixture.run(true, false);
    assert_ne!(result.code, 0);
    assert_eq!(
        parse_envelope(&result.stdout)["error"]["code"],
        "remote_drift"
    );
    assert!(!fixture.remote_sha.exists());
}

#[test]
fn github_private_bootstrap_rejects_enterprise_internal_visibility() {
    let mut fixture = Fixture::new(true);
    fixture
        .stub
        .envs
        .push(("GH_TEST_PRIVATE".into(), "true".into()));
    fixture
        .stub
        .envs
        .push(("GH_TEST_VISIBILITY".into(), "internal".into()));
    let result = fixture.run_with_visibility(true, false, "private");
    assert_ne!(result.code, 0);
    assert_eq!(
        parse_envelope(&result.stdout)["error"]["code"],
        "remote_drift"
    );
    assert!(
        !fs::read_to_string(&fixture.git_log)
            .unwrap_or_default()
            .contains("push ")
    );
}

#[test]
fn github_rechecks_visibility_after_the_first_push() {
    let mut fixture = Fixture::new(true);
    fixture
        .stub
        .envs
        .push(("GH_TEST_PRIVATE".into(), "true".into()));
    fixture
        .stub
        .envs
        .push(("GH_TEST_VISIBILITY".into(), "private".into()));
    fixture
        .stub
        .envs
        .push(("GH_TEST_VISIBILITY_AFTER_PUSH".into(), "public".into()));
    let result = fixture.run_with_visibility(true, false, "private");
    assert_ne!(result.code, 0);
    assert_eq!(
        parse_envelope(&result.stdout)["error"]["code"],
        "remote_drift"
    );
    assert!(
        fixture.remote_sha.exists(),
        "the drift is detected after push"
    );
}

#[test]
fn github_reconciles_an_ambiguous_first_push_without_retrying_it() {
    let mut fixture = Fixture::new(true);
    fixture
        .stub
        .envs
        .push(("GH_TEST_PUSH_MODE".to_string(), "ambiguous".to_string()));
    let result = fixture.run(true, false);
    assert_eq!(
        result.code, 0,
        "stdout={} stderr={}",
        result.stdout, result.stderr
    );
    assert_eq!(parse_envelope(&result.stdout)["data"]["reconciled"], true);
    assert_eq!(
        fs::read_to_string(&fixture.git_log)
            .expect("git log")
            .matches("push --porcelain")
            .count(),
        1
    );
}

#[test]
fn github_refuses_unverified_provider_signature() {
    let mut fixture = Fixture::new(true);
    fixture
        .stub
        .envs
        .push(("GH_TEST_VERIFIED".to_string(), "false".to_string()));
    let result = fixture.run(true, false);
    assert_ne!(result.code, 0);
    assert_eq!(
        parse_envelope(&result.stdout)["error"]["code"],
        "provider_signature_unverified"
    );
}

#[test]
fn github_binds_api_to_the_selected_host_despite_ambient_gh_host() {
    let mut fixture = Fixture::new(true);
    fixture
        .stub
        .envs
        .push(("GH_HOST".to_string(), "wrong.example".to_string()));
    let result = fixture.run(true, false);
    assert_eq!(
        result.code, 0,
        "stdout={} stderr={}",
        result.stdout, result.stderr
    );
    let log = fs::read_to_string(&fixture.gh_log).expect("gh log");
    assert!(
        log.lines()
            .all(|line| line.starts_with("GH_HOST=github.com "))
    );
}

#[test]
fn github_receipt_is_isolated_by_host() {
    let fixture = Fixture::new(true);
    let result = fixture.run(true, false);
    assert_eq!(
        result.code, 0,
        "stdout={} stderr={}",
        result.stdout, result.stderr
    );
    let old_receipt = parse_envelope(&result.stdout)["data"]["receipt"]
        .as_str()
        .expect("receipt path")
        .to_string();
    let resumed = run_forge_cli(
        &fixture.stub,
        &[
            "--provider",
            "github",
            "--host",
            "enterprise.example",
            "--repo",
            "sympoies/widgets",
            "--format",
            "json",
            "repo",
            "bootstrap",
            "--owner-kind",
            "org",
            "--visibility",
            "public",
            "--existing-empty",
            "--default-branch",
            "main",
            "--file",
            fixture.readme.to_str().expect("UTF-8 path"),
            "--message",
            "chore: initialize repository",
            "--reason-file",
            fixture.reason.to_str().expect("UTF-8 path"),
            "--resume",
        ],
    );
    assert_ne!(resumed.code, 0);
    assert_eq!(
        parse_envelope(&resumed.stdout)["error"]["code"],
        "bootstrap_receipt_missing"
    );
    assert!(!resumed.stdout.contains(&old_receipt));
    assert_eq!(
        fs::read_to_string(&fixture.git_log)
            .expect("git log")
            .matches("push --porcelain")
            .count(),
        1
    );
}

#[test]
fn github_refuses_to_adopt_a_different_owner_kind() {
    let mut fixture = Fixture::new(true);
    fixture
        .stub
        .envs
        .push(("GH_TEST_OWNER_TYPE".to_string(), "User".to_string()));
    let result = fixture.run(true, false);
    assert_ne!(result.code, 0);
    assert_eq!(
        parse_envelope(&result.stdout)["error"]["code"],
        "remote_drift"
    );
    assert!(!fixture.remote_sha.exists());
}

#[test]
fn github_refuses_completion_if_another_ref_appears_during_push() {
    let mut fixture = Fixture::new(true);
    fixture
        .stub
        .envs
        .push(("GH_TEST_EXTRA_REF".to_string(), "yes".to_string()));
    let result = fixture.run(true, false);
    assert_ne!(result.code, 0);
    assert_eq!(
        parse_envelope(&result.stdout)["error"]["code"],
        "remote_drift"
    );
    assert!(fixture.remote_sha.exists());
}

#[test]
fn github_reports_provider_neutral_repo_validation() {
    let stub = StubEnv::new();
    let readme = stub.tempdir.path().join("README.md");
    let reason = stub.tempdir.path().join("reason.txt");
    fs::write(&readme, "# Widgets\n").expect("README fixture");
    fs::write(&reason, "Explicit bootstrap authorization.\n").expect("reason fixture");
    let result = run_forge_cli(
        &stub,
        &[
            "--provider",
            "github",
            "--format",
            "json",
            "--dry-run",
            "repo",
            "bootstrap",
            "--owner-kind",
            "org",
            "--default-branch",
            "main",
            "--file",
            readme.to_str().expect("UTF-8 path"),
            "--message",
            "chore: initialize repository",
            "--reason-file",
            reason.to_str().expect("UTF-8 path"),
        ],
    );
    assert_ne!(result.code, 0);
    let envelope = parse_envelope(&result.stdout);
    assert_eq!(envelope["error"]["code"], "repo_invalid");
    assert!(
        !envelope["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("Forgejo")
    );
}
