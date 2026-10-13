use super::support::StubEnv;

pub(super) fn gitlab_merge_api_stub_full(
    stub: &StubEnv,
    discussions: &str,
    description: &str,
) -> String {
    let sentinel = stub.tempdir.path().join("merged");
    let args_log = stub.tempdir.path().join("merge-args");
    format!(
        r#"#!/bin/sh
set -e
case "$1 $2" in
  "repo view")
    cat <<'EOF'
{{
  "namespace": {{ "full_path": "group" }},
  "path": "project",
  "web_url": "https://gitlab.example.com/group/project",
  "default_branch": "main",
  "merge_method": "merge",
  "squash_option": "default_on"
}}
EOF
    ;;
  "mr view")
    if [ -e {sentinel} ]; then
      cat <<'EOF'
{{
  "iid": 7,
  "web_url": "https://gitlab.example.com/group/project/-/merge_requests/7",
  "description": {description},
  "state": "merged",
  "draft": false,
  "title": "feat: sample",
  "source_branch": "feat/sample",
  "source_project_id": 1,
  "target_branch": "main",
  "merge_status": "can_be_merged",
  "sha": "abc123",
  "merge_commit_sha": "def456",
  "labels": [],
  "head_pipeline": {{ "id": 99, "status": "success", "web_url": "https://gitlab.example.com/group/project/-/pipelines/99" }}
}}
EOF
    else
      cat <<'EOF'
{{
  "iid": 7,
  "web_url": "https://gitlab.example.com/group/project/-/merge_requests/7",
  "description": {description},
  "state": "opened",
  "draft": false,
  "title": "feat: sample",
  "source_branch": "feat/sample",
  "source_project_id": 1,
  "target_branch": "main",
  "merge_status": "can_be_merged",
  "sha": "abc123",
  "merge_commit_sha": null,
  "labels": [],
  "head_pipeline": {{ "id": 99, "status": "success", "web_url": "https://gitlab.example.com/group/project/-/pipelines/99" }}
}}
EOF
    fi
    ;;
  "api graphql") printf '%s\n' '{{"data":{{"project":{{"repository":{{"blobs":{{"nodes":[],"pageInfo":{{"hasNextPage":false}}}}}}}}}}}}' ;;
  "api --hostname")
    case "$*" in
      *"merge_requests?state=opened"*) printf '%s\n' '[]' ;;
      *"projects/group%2Fproject/pipelines/99/jobs?per_page=100"*)
        cat <<'EOF'
[
  {{
    "name": "build",
    "stage": "test",
    "status": "success",
    "allow_failure": false,
    "web_url": "https://gitlab.example.com/group/project/-/jobs/1"
  }}
]
EOF
        ;;
      *"projects/group%2Fproject/merge_requests/7") printf '%s\n' '{{"labels":[]}}' ;;
      *)
        echo "stub: unexpected api args: $*" >&2
        exit 99
        ;;
    esac
    ;;
  "api --paginate")
    case "$*" in
      *"projects/group%2Fproject/merge_requests/7/discussions?per_page=100"*)
        # Merge lock-down rule 13 — review-thread sweep.
        cat <<'EOF'
{discussions}
EOF
        ;;
      *)
        echo "stub: unexpected paginate api args: $*" >&2
        exit 99
        ;;
    esac
    ;;
  "api --method")
    printf '%s\n' "$*" > {args_log}
    case "$*" in
      *"PUT"*"--hostname gitlab.example.com"*"projects/group%2Fproject/merge_requests/7/merge"*"squash=true"*"should_remove_source_branch=true"*"sha=abc123"*)
        touch {sentinel}
        cat <<'EOF'
{{ "state": "merged", "merge_commit_sha": "def456" }}
EOF
        ;;
      *)
        echo "stub: unexpected merge api args: $*" >&2
        exit 99
        ;;
    esac
    ;;
  *)
    echo "stub: unexpected glab args: $*" >&2
    exit 99
    ;;
esac
"#,
        sentinel = sentinel.display(),
        args_log = args_log.display(),
        discussions = discussions,
        description = description,
    )
}
