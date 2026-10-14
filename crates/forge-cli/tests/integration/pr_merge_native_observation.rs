mod native_observation_tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn late_user_thread(allow_unresolved: bool) {
        let config = r#"[review_convergence]
require = true
quiet_period = "0s"
timeout = "20m"
[[review_convergence.bots]]
login = "example-review-bot"
mode = "observed"
"#;
        let tempdir = make_github_repo(Some(config));
        let repo_path = tempdir.path().join("repo");
        let stub = StubEnv::new();
        let review = serde_json::json!({
            "id":"PRR_user", "databaseId":1,
            "url":"https://github.com/acme/widgets/pull/7#pullrequestreview-1",
            "author":{"login":"example-review-bot", "__typename":"User", "id":"U_reviewer"},
            "state":"APPROVED", "commit":{"oid":"head123"},
            "submittedAt":"2026-10-11T00:00:00Z", "body":"native approval"
        });
        let thread = serde_json::json!({
            "id":"PRRT_late", "isResolved":false, "isOutdated":false,
            "path":"src/lib.rs", "diffSide":"RIGHT", "line":10, "originalLine":10,
            "originalStartLine":null, "startDiffSide":null, "startLine":null, "subjectType":"LINE",
            "comments":{
                "totalCount":1,
                "nodes":[{
                    "id":"PRRC_late", "author":{"login":"example-review-bot"},
                    "body":"late native user finding", "createdAt":"2026-10-11T00:01:00Z",
                    "url":"https://github.com/acme/widgets/pull/7#discussion_r1"
                }],
                "pageInfo":{"hasNextPage":false,"endCursor":null}
            }
        });
        let response = serde_json::json!({
            "data":{"repository":{"pullRequest":{
                "headRefOid":"head123",
                "reviewThreads":{"totalCount":1,"nodes":[thread],
                    "pageInfo":{"hasNextPage":false,"endCursor":null}}
            }}}
        });
        let response_path = stub.tempdir.path().join("late-thread.json");
        fs::write(&response_path, response.to_string()).expect("late thread fixture");
        let calls_path = stub.tempdir.path().join("thread-calls");
        let extra_cases = format!(
            r#"
      *"reviewThreads(first: 100"*)
        count=0
        if [ -f {calls} ]; then count=$(cat {calls}); fi
        count=$((count + 1))
        printf '%s\n' "$count" > {calls}
        if [ "$count" -ge 3 ]; then
          cat {response}
        else
          printf '%s\n' '{{"data":{{"repository":{{"pullRequest":{{"headRefOid":"head123","reviewThreads":{{"totalCount":0,"nodes":[],"pageInfo":{{"hasNextPage":false,"endCursor":null}}}}}}}}}}}}'
        fi
        ;;
"#,
            calls = calls_path.display(),
            response = response_path.display()
        );
        let body = github_merge_stub_full(
            &stub,
            &review.to_string(),
            "",
            false,
            Some(review_loop_marker(false)),
            ONE_REQUIRED_PASS_CHECKS,
            NO_MERGE_POLICY,
            &extra_cases,
        );
        let stub = stub.gh_gate_stub(&body);
        let mut args = vec![
            "--provider",
            "github",
            "--repo",
            "acme/widgets",
            "--format",
            "json",
            "pr",
            "merge",
            "7",
        ];
        if allow_unresolved {
            args.extend([
                "--allow-unresolved-threads",
                "--allow-unresolved-threads-reason",
                "synthetic override control",
            ]);
        }
        let out = run_forge_cli_in(&stub, &args, Some(&repo_path));
        let envelope = parse_envelope(&out.stdout);
        assert_eq!(
            out.code,
            if allow_unresolved { 0 } else { 65 },
            "stdout={}\nstderr={}",
            out.stdout,
            out.stderr
        );
        assert_eq!(
            stub.tempdir.path().join("github-merged").exists(),
            allow_unresolved
        );
        assert_eq!(fs::read_to_string(calls_path).unwrap().trim(), "3");
        if allow_unresolved {
            assert_eq!(
                envelope["data"]["review_convergence"]["unresolved_threads"],
                1
            );
            assert!(
                envelope["data"]["review_convergence"]["semantic_activity"]
                    .as_array()
                    .unwrap()
                    .is_empty()
            );
        } else {
            assert_eq!(envelope["error"]["code"], "unresolved_review_threads");
        }
    }

    #[test]
    fn pr_merge_refuses_late_native_user_threads_before_mutation() {
        late_user_thread(false);
    }

    #[test]
    fn pr_merge_explicit_thread_override_allows_a_late_native_user_thread() {
        late_user_thread(true);
    }
}
