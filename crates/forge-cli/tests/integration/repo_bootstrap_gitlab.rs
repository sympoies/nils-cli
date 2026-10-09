//! Empty GitLab project bootstrap contracts, without live provider calls.
use std::fs;
use std::path::PathBuf;

use pretty_assertions::{assert_eq, assert_ne};

use super::support::{CmdOutput, StubEnv, parse_envelope, run_forge_cli};

const SHA: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

struct Fixture {
    stub: StubEnv,
    readme: PathBuf,
    reason: PathBuf,
    log: PathBuf,
    git_log: PathBuf,
    remote_sha: PathBuf,
    user_namespace: bool,
}

impl Fixture {
    fn new(existing: bool) -> Self {
        let stub = StubEnv::new();
        let readme = stub.tempdir.path().join("README.md");
        let reason = stub.tempdir.path().join("authorization.txt");
        let exists = stub.tempdir.path().join("exists");
        let remote_sha = stub.tempdir.path().join("remote-sha");
        let branch = stub.tempdir.path().join("branch");
        let log = stub.tempdir.path().join("glab.log");
        let git_log = stub.tempdir.path().join("git.log");
        fs::write(&readme, "# Widgets\n").unwrap();
        fs::write(
            &reason,
            "Operator authorized this exact project bootstrap.\n",
        )
        .unwrap();
        if existing {
            fs::write(&exists, "yes").unwrap();
        }
        let glab = stub.write_stub("glab", r#"#!/bin/sh
if [ "$1" = config ]; then
  [ "${GL_TEST_FAIL_TOKEN:-}" != yes ] || exit 1
  [ "$*" = 'config get token --host gitlab.example.com' ] || exit 9
  printf '%s\n' 'fixture-token-value'; exit 0
fi
printf 'GITLAB_HOST=%s %s\n' "$GITLAB_HOST" "$*" >> "$GL_TEST_LOG"
endpoint= method=GET
for arg in "$@"; do
  case "$arg" in user|namespaces/*|projects*) endpoint=$arg ;; POST|PUT) method=$arg ;; esac
done
case "$endpoint" in
  projects/operator%2Fwidgets*) endpoint="projects/team%2Fsub%2Fwidgets${endpoint#projects/operator%2Fwidgets}" ;;
esac
header() { printf 'HTTP/2 %s\r\nContent-Type: application/json\r\n\r\n' "$1"; }
missing() { header '404 Not Found'; printf '%s\n' '{"message":"404 Project Not Found"}'; exit 1; }
project() {
  default=null; [ ! -f "$GL_TEST_BRANCH" ] || default="\"$(cat "$GL_TEST_BRANCH")\""
  empty=true; [ ! -f "$GL_TEST_REMOTE_SHA" ] || empty=false
  owner=team/sub; kind=group; visibility=public
  if [ "${GL_TEST_USER_NAMESPACE:-}" = yes ]; then owner=operator; kind=user; visibility=private; fi
  clone_url=${GL_TEST_CLONE_URL:-https://gitlab.example.com/$owner/widgets.git}
  header '200 OK'
  printf '{"path":"widgets","path_with_namespace":"%s/widgets","namespace":{"id":42,"full_path":"%s","kind":"%s"},"visibility":"%s","http_url_to_repo":"%s","default_branch":%s,"empty_repo":%s}\n' "$owner" "$owner" "$kind" "$visibility" "$clone_url" "$default" "$empty"
}
case "$endpoint" in
  user) header '200 OK'; printf '%s\n' '{"username":"operator"}' ;;
  namespaces/team%2Fsub) header '200 OK'; printf '%s\n' '{"id":42,"full_path":"team/sub","kind":"group"}' ;;
  namespaces/operator) header '200 OK'; printf '%s\n' '{"id":42,"full_path":"operator","kind":"user"}' ;;
  projects)
    [ "$method" = POST ] || exit 9
    : > "$GL_TEST_EXISTS"; project ;;
  projects/team%2Fsub%2Fwidgets)
    [ -f "$GL_TEST_EXISTS" ] || missing
    if [ "$method" = PUT ]; then
      [ "${GL_TEST_FAIL_DEFAULT:-}" != yes ] || { header '503 Unavailable'; exit 1; }
      printf '%s\n' 'trunk/topic' > "$GL_TEST_BRANCH"
    fi
    project ;;
  projects/team%2Fsub%2Fwidgets/repository/branches/trunk%2Ftopic)
    [ -f "$GL_TEST_REMOTE_SHA" ] || missing
    header '200 OK'; printf '{"name":"trunk/topic","commit":{"id":"%s"}}\n' "$(cat "$GL_TEST_REMOTE_SHA")" ;;
  projects/team%2Fsub%2Fwidgets/repository/branches*)
    [ -f "$GL_TEST_REMOTE_SHA" ] || [ "${GL_TEST_TAGS:-[]}" != '[]' ] || missing
    header '200 OK'
    if [ -f "$GL_TEST_REMOTE_SHA" ]; then
      printf '[{"name":"trunk/topic","commit":{"id":"%s"}}]\n' "$(cat "$GL_TEST_REMOTE_SHA")"
    else printf '%s\n' '[]'; fi ;;
  projects/team%2Fsub%2Fwidgets/repository/tags*)
    [ -f "$GL_TEST_REMOTE_SHA" ] || [ "${GL_TEST_TAGS:-[]}" != '[]' ] || missing
    header '200 OK'
    if [ -f "$GL_TEST_REMOTE_SHA" ] && [ "${GL_TEST_TAG_AFTER_PUSH:-}" = yes ]; then
      printf '%s\n' '[{"name":"other"}]'
    else printf '%s\n' "${GL_TEST_TAGS:-[]}"; fi ;;
  projects/team%2Fsub%2Fwidgets/repository/commits/*/signature)
    header '200 OK'; printf '{"verification_status":"%s"}\n' "${GL_TEST_VERIFICATION:-verified}" ;;
  projects/team%2Fsub%2Fwidgets/repository/commits/*)
    header '200 OK'; printf '{"id":"%s","parent_ids":%s}\n' "${GL_TEST_COMMIT_ID:-$GL_TEST_SHA}" "${GL_TEST_PARENTS:-[]}" ;;
  *) printf 'unexpected endpoint: %s\n' "$endpoint" >&2; exit 9 ;;
esac
"#);
        let git = stub.write_stub("git-bootstrap", r#"#!/bin/sh
printf '%s\n' "$*" >> "$GL_TEST_GIT_LOG"
case "$*" in
  *"init --initial-branch="*) command git -C "$2" init --initial-branch="${4#--initial-branch=}" >/dev/null ;;
  *" add "*|*" config "*) command git "$@" ;;
  *"rev-parse"*"HEAD^{commit}"*|*"rev-list --parents -n 1"*) printf '%s\n' "$GL_TEST_SHA" ;;
  *"log -1 --format=%G?"*) printf '%s\n' G ;;
  *"push "*)
    [ "$FORGE_CLI_BOOTSTRAP_USERNAME" = oauth2 ] || exit 9
    [ "$FORGE_CLI_BOOTSTRAP_GITLAB_TOKEN" = fixture-token-value ] || exit 9
    [ "${GL_TEST_PUSH_ABSENT:-}" != yes ] || exit 1
    printf '%s\n' "$GL_TEST_SHA" > "$GL_TEST_REMOTE_SHA" ;;
esac
"#);
        let semantic = stub.write_stub(
            "semantic-bootstrap",
            r#"#!/bin/sh
printf '{"ok":true,"commit":{"sha":"%s"}}\n' "$GL_TEST_SHA"
"#,
        );
        let state_home = stub.tempdir.path().join("state");
        let stub = stub
            .env("FORGE_CLI_GLAB_BIN", glab.to_string_lossy())
            .env("FORGE_CLI_GIT_BIN", git.to_string_lossy())
            .env("FORGE_CLI_SEMANTIC_COMMIT_BIN", semantic.to_string_lossy())
            .env("XDG_STATE_HOME", state_home.to_string_lossy())
            .env("GL_TEST_EXISTS", exists.to_string_lossy())
            .env("GL_TEST_REMOTE_SHA", remote_sha.to_string_lossy())
            .env("GL_TEST_BRANCH", branch.to_string_lossy())
            .env("GL_TEST_LOG", log.to_string_lossy())
            .env("GL_TEST_GIT_LOG", git_log.to_string_lossy())
            .env("GL_TEST_SHA", SHA);
        Self {
            stub,
            readme,
            reason,
            log,
            git_log,
            remote_sha,
            user_namespace: false,
        }
    }

    fn run(&self, existing: bool, resume: bool, dry_run: bool) -> CmdOutput {
        let mut args = vec![
            "--provider",
            "gitlab",
            "--host",
            "gitlab.example.com",
            "--repo",
            if self.user_namespace {
                "operator/widgets"
            } else {
                "team/sub/widgets"
            },
            "--format",
            "json",
            "repo",
            "bootstrap",
            "--owner-kind",
            if self.user_namespace { "user" } else { "org" },
            "--default-branch",
            "trunk/topic",
            "--file",
            self.readme.to_str().unwrap(),
            "--message",
            "chore: initialize repository",
            "--reason-file",
            self.reason.to_str().unwrap(),
        ];
        if !self.user_namespace {
            args.extend(["--visibility", "public"]);
        }
        if existing {
            args.push("--existing-empty");
        }
        if resume {
            args.push("--resume");
        }
        if dry_run {
            args.push("--dry-run");
        }
        run_forge_cli(&self.stub, &args)
    }
}

fn success(output: CmdOutput) -> serde_json::Value {
    assert_eq!(
        output.code, 0,
        "stdout={} stderr={}",
        output.stdout, output.stderr
    );
    parse_envelope(&output.stdout)
}

#[test]
fn gitlab_bootstrap_creates_empty_project_and_sets_requested_default() {
    let fixture = Fixture::new(false);
    let result = success(fixture.run(false, false, false));
    assert_eq!(result["data"]["provider"], "gitlab");
    assert_eq!(result["data"]["host"], "gitlab.example.com");
    assert_eq!(result["data"]["root_commit_sha"], SHA);
    assert_eq!(result["data"]["remote_created"], true);
    assert_eq!(result["data"]["signature_verified"], true);
    let log = fs::read_to_string(&fixture.log).unwrap();
    assert!(
        log.contains("namespace_id=42") && log.contains("initialize_with_readme=false"),
        "{log}"
    );
    assert!(
        log.contains("PUT") && log.contains("default_branch=trunk/topic"),
        "{log}"
    );
    assert!(
        log.lines()
            .all(|line| line.starts_with("GITLAB_HOST=gitlab.example.com "))
    );
    let git = fs::read_to_string(&fixture.git_log).unwrap();
    assert!(
        git.contains(&format!("{SHA}:refs/heads/trunk/topic")),
        "{git}"
    );
    assert!(git.contains("rev-list --parents -n 1"));
    assert!(!git.contains("--force"));
}

#[test]
fn gitlab_bootstrap_adopts_existing_empty_project_without_creation() {
    let fixture = Fixture::new(true);
    let result = success(fixture.run(true, false, false));
    assert_eq!(result["data"]["remote_created"], false);
    assert!(!fs::read_to_string(&fixture.log).unwrap().contains("POST"));
}

#[test]
fn gitlab_bootstrap_creates_authenticated_user_project_with_default_private_visibility() {
    let mut fixture = Fixture::new(false);
    fixture.user_namespace = true;
    fixture.stub = fixture.stub.env("GL_TEST_USER_NAMESPACE", "yes");
    let result = success(fixture.run(false, false, false));
    assert_eq!(result["data"]["root_commit_sha"], SHA);
    let log = fs::read_to_string(&fixture.log).unwrap();
    assert!(log.contains("namespaces/operator"), "{log}");
    let create = log.lines().find(|line| line.contains("POST")).unwrap();
    assert!(create.contains("visibility=private"), "{create}");
    assert!(create.contains("namespace_id=42"), "{create}");
    assert!(
        fs::read_to_string(&fixture.git_log)
            .unwrap()
            .contains("https://gitlab.example.com/operator/widgets.git"),
        "{log}"
    );
}

#[test]
fn gitlab_bootstrap_refuses_clone_url_host_or_path_drift_before_push() {
    for clone_url in [
        "https://other.gitlab.example.com/team/sub/widgets.git",
        "https://gitlab.example.com/team/sub/other.git",
    ] {
        let mut fixture = Fixture::new(true);
        fixture.stub = fixture.stub.env("GL_TEST_CLONE_URL", clone_url);
        let output = fixture.run(true, false, false);
        assert_ne!(output.code, 0);
        assert_eq!(
            parse_envelope(&output.stdout)["error"]["code"],
            "remote_drift"
        );
        let git = fs::read_to_string(&fixture.git_log).unwrap_or_default();
        assert!(!git.contains("push "), "{clone_url}: {git}");
        assert!(!fixture.remote_sha.exists());
    }
}

#[test]
fn gitlab_bootstrap_refuses_delivered_commit_with_parents() {
    let mut fixture = Fixture::new(true);
    fixture.stub = fixture.stub.env(
        "GL_TEST_PARENTS",
        "[\"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\"]",
    );
    let output = fixture.run(true, false, false);
    assert_ne!(output.code, 0);
    assert_eq!(
        parse_envelope(&output.stdout)["error"]["code"],
        "bootstrap_commit_not_root"
    );
    assert_eq!(fs::read_to_string(&fixture.remote_sha).unwrap().trim(), SHA);
}

#[test]
fn gitlab_bootstrap_refuses_mismatched_delivered_commit_id() {
    let mut fixture = Fixture::new(true);
    fixture.stub = fixture.stub.env(
        "GL_TEST_COMMIT_ID",
        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
    );
    let output = fixture.run(true, false, false);
    assert_ne!(output.code, 0);
    assert_eq!(
        parse_envelope(&output.stdout)["error"]["code"],
        "remote_drift"
    );
    assert_eq!(fs::read_to_string(&fixture.remote_sha).unwrap().trim(), SHA);
}

#[test]
fn gitlab_bootstrap_refuses_non_empty_project_before_local_commit_or_push() {
    let fixture = Fixture::new(true);
    fs::write(&fixture.remote_sha, SHA).unwrap();
    let output = fixture.run(true, false, false);
    assert_eq!(
        parse_envelope(&output.stdout)["error"]["code"],
        "repository_not_empty"
    );
    assert!(!fixture.git_log.exists());
}

#[test]
fn gitlab_bootstrap_resumes_after_default_update_failure_without_repush() {
    let mut fixture = Fixture::new(true);
    fixture
        .stub
        .envs
        .push(("GL_TEST_FAIL_DEFAULT".into(), "yes".into()));
    let output = fixture.run(true, false, false);
    assert_eq!(
        parse_envelope(&output.stdout)["error"]["code"],
        "bootstrap_default_branch_failed"
    );
    fixture
        .stub
        .envs
        .retain(|(key, _)| key != "GL_TEST_FAIL_DEFAULT");
    let result = success(fixture.run(true, true, false));
    assert_eq!(result["data"]["reconciled"], true);
    success(fixture.run(true, true, false));
    let git = fs::read_to_string(&fixture.git_log).unwrap();
    assert_eq!(
        git.lines()
            .filter(|line| line.contains("push --porcelain"))
            .count(),
        1
    );
}

#[test]
fn gitlab_bootstrap_dry_run_has_no_backend_or_state_mutations() {
    let fixture = Fixture::new(false);
    let result = success(fixture.run(false, false, true));
    assert_eq!(result["data"]["auto_init"], false);
    assert_eq!(result["data"]["provider"], "gitlab");
    assert!(!fixture.log.exists());
    assert!(!fixture.git_log.exists());
    assert!(!fixture.stub.tempdir.path().join("state").exists());
}

#[test]
fn gitlab_bootstrap_refuses_unverified_provider_signature() {
    let mut fixture = Fixture::new(true);
    fixture
        .stub
        .envs
        .push(("GL_TEST_VERIFICATION".into(), "unverified".into()));
    let output = fixture.run(true, false, false);
    assert_eq!(
        parse_envelope(&output.stdout)["error"]["code"],
        "provider_signature_unverified"
    );
}

#[test]
fn gitlab_bootstrap_refuses_tag_only_project() {
    let mut fixture = Fixture::new(true);
    fixture
        .stub
        .envs
        .push(("GL_TEST_TAGS".into(), "[{\"name\":\"other\"}]".into()));
    let output = fixture.run(true, false, false);
    assert_eq!(
        parse_envelope(&output.stdout)["error"]["code"],
        "repository_not_empty"
    );
    assert!(!fixture.git_log.exists());
}

#[test]
fn gitlab_bootstrap_refuses_extra_refs_after_push() {
    let mut fixture = Fixture::new(true);
    fixture
        .stub
        .envs
        .push(("GL_TEST_TAG_AFTER_PUSH".into(), "yes".into()));
    let output = fixture.run(true, false, false);
    assert_eq!(
        parse_envelope(&output.stdout)["error"]["code"],
        "remote_drift"
    );
}

#[test]
fn gitlab_bootstrap_never_repeats_an_indeterminate_first_push_on_resume() {
    let mut fixture = Fixture::new(true);
    fixture
        .stub
        .envs
        .push(("GL_TEST_PUSH_ABSENT".into(), "yes".into()));
    let output = fixture.run(true, false, false);
    assert_eq!(
        parse_envelope(&output.stdout)["error"]["code"],
        "bootstrap_push_failed"
    );
    let output = fixture.run(true, true, false);
    assert_eq!(
        parse_envelope(&output.stdout)["error"]["code"],
        "bootstrap_push_indeterminate"
    );
    let git = fs::read_to_string(&fixture.git_log).unwrap();
    assert_eq!(
        git.lines()
            .filter(|line| line.contains("push --porcelain"))
            .count(),
        1
    );
}

#[test]
fn gitlab_bootstrap_resumes_after_credential_lookup_failure_before_push() {
    let mut fixture = Fixture::new(false);
    fixture.stub = fixture.stub.env("GL_TEST_FAIL_TOKEN", "yes");
    assert_ne!(fixture.run(false, false, false).code, 0);
    assert!(
        !fs::read_to_string(&fixture.git_log)
            .unwrap()
            .contains("push ")
    );
    fixture.stub = fixture.stub.env("GL_TEST_FAIL_TOKEN", "no");
    success(fixture.run(false, true, false));
    assert_eq!(
        fs::read_to_string(&fixture.git_log)
            .unwrap()
            .lines()
            .filter(|line| line.contains("push "))
            .count(),
        1
    );
}
