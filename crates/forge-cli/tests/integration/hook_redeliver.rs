//! Redelivery acceptance through a portable synthetic GitHub CLI.
use std::fs;

use pretty_assertions::assert_eq;
use serde_json::{Value, json};

use super::repo_hook::calls;
use super::support::{CmdOutput, StubEnv, parse_envelope, run_forge_cli};

const SINCE: &str = "2026-01-02T00:00:00Z";
const UNTIL: &str = "2026-01-03T00:00:00Z";
const CANARIES: [&str; 4] = [
    "REQUEST-PAYLOAD-CANARY",
    "RESPONSE-PAYLOAD-CANARY",
    "HEADER-CANARY",
    "SECRET-CANARY",
];

fn delivery(id: u64, code: u16, at: &str) -> Value {
    json!({"id": id, "guid": format!("event-{id}"), "redelivery": false,
        "status_code": code, "status": "SECRET-CANARY", "delivered_at": at,
        "request": {"payload": CANARIES[0], "headers": {"Authorization": CANARIES[2]}},
        "response": {"payload": CANARIES[1]}, "secret": CANARIES[3]})
}

fn stub(rows: Vec<Value>) -> StubEnv {
    let stub = StubEnv::new();
    let dir = stub.tempdir.path().to_path_buf();
    // Two pages, including when the first is short: gh owns cursor traversal.
    let split = rows.len().min(1);
    fs::write(
        dir.join("before.json"),
        json!([&rows[..split], &rows[split..]]).to_string(),
    )
    .unwrap();
    let mut after = rows.clone();
    for row in rows {
        let id = row["id"].as_u64().unwrap();
        let mut new = row;
        new["id"] = json!(new["id"].as_u64().unwrap() + 1000);
        new["redelivery"] = json!(true);
        new["status_code"] = json!(201);
        new["delivered_at"] = json!("2026-01-04T00:00:00Z");
        after.push(new);
        fs::write(
            dir.join(format!("after-{id}.json")),
            json!([&after]).to_string(),
        )
        .unwrap();
    }
    stub.gh_stub(&format!(r#"#!/bin/sh
set -eu
dir='{}'
printf '%s\n' "$*" >> "$dir/calls"
case "$2" in
  */deliveries\?per_page=100)
    [ "$3" = --paginate ] && [ "$4" = --slurp ] || exit 98
    if [ -e "$dir/fail-list" ]; then echo 'SECRET-CANARY HEADER-CANARY' >&2; exit 1; fi
    if [ -e "$dir/attempted" ] && [ ! -e "$dir/pending" ]; then cat "$dir/after-$(cat "$dir/attempted").json"; else cat "$dir/before.json"; fi ;;
  */attempts)
    [ "$3" = --method ] && [ "$4" = POST ] && [ "$#" -eq 4 ] || exit 97
    case "$2" in */11/attempts) if [ -e "$dir/fail-post" ]; then
      printf '%s\n' 'SECRET-CANARY REQUEST-PAYLOAD-CANARY RESPONSE-PAYLOAD-CANARY HEADER-CANARY' >&2; exit 1
    fi;; esac
    id=${{2%/attempts}}; id=${{id##*/}}
    printf '%s' "$id" > "$dir/attempted" ;;
  */deliveries/*)
    if [ -e "$dir/fail-read" ]; then echo 'SECRET-CANARY HEADER-CANARY' >&2; exit 1; fi
    id=${{2##*/}}
    if [ -e "$dir/bad-read" ]; then echo '{{"request":"SECRET-CANARY"}}'; exit 0; fi
    code=201; if [ -e "$dir/receiver-failed" ]; then code=503; fi
    printf '{{"id":%s,"status_code":%s,"status":"SECRET-CANARY","request":{{"payload":"REQUEST-PAYLOAD-CANARY","headers":{{"x-secret":"HEADER-CANARY"}}}},"response":{{"payload":"RESPONSE-PAYLOAD-CANARY"}}}}' "$id" "$code" ;;
  *) echo 'unexpected synthetic provider call' >&2; exit 99 ;;
esac
"#, dir.display()))
}

fn run(stub: &StubEnv, family: &str, dry: bool, format: &str, tail: &[&str]) -> CmdOutput {
    let mut args = vec!["--provider", "github", "--format", format];
    if dry {
        args.push("--dry-run");
    }
    args.extend([family, "hook"]);
    args.extend_from_slice(tail);
    run_forge_cli(stub, &args)
}

fn single(stub: &StubEnv, family: &str, dry: bool, format: &str, status: &str) -> CmdOutput {
    let (flag, target) = if family == "repo" {
        ("--repo", "acme/widget")
    } else {
        ("--org", "acme")
    };
    run(
        stub,
        family,
        dry,
        format,
        &[
            "redeliver",
            flag,
            target,
            "--hook",
            "7",
            "--status",
            status,
            "--since",
            SINCE,
            "--until",
            UNTIL,
        ],
    )
}

fn no_leaks(out: &CmdOutput) {
    for canary in CANARIES {
        assert!(
            !out.stdout.contains(canary) && !out.stderr.contains(canary),
            "{out:?}"
        );
    }
}

#[test]
fn hook_redeliver_filters_status_and_window_inclusive_start_exclusive_end() {
    for family in ["repo", "org"] {
        let stub = stub(vec![
            delivery(11, 500, SINCE),
            delivery(12, 302, SINCE),
            delivery(13, 503, "2026-01-01T23:59:59Z"),
            delivery(14, 500, UNTIL),
            delivery(15, 0, "2026-01-02T12:00:00+00:00"),
        ]);
        let out = single(&stub, family, false, "json", "failed");
        assert_eq!(out.code, 0, "{out:?}");
        no_leaks(&out);
        let env = parse_envelope(&out.stdout);
        assert_eq!(
            env["schema_version"],
            format!("cli.forge-cli.{family}.hook.redeliver.v1")
        );
        let rows = env["data"]["rows"].as_array().unwrap();
        assert_eq!(
            rows.iter()
                .map(|r| r["delivery"].as_u64().unwrap())
                .collect::<Vec<_>>(),
            vec![11, 15]
        );
        assert_eq!(rows[0]["status"], "redelivered");
        assert_eq!(rows[0]["observed_delivery"], 1011);
        assert_eq!(rows[0]["delivery_status"], "success");
        assert_eq!(rows[0]["status_code"], 201);
        let posts: Vec<_> = calls(&stub)
            .into_iter()
            .filter(|c| c.contains("--method POST"))
            .collect();
        let prefix = if family == "repo" {
            "repos/acme/widget"
        } else {
            "orgs/acme"
        };
        assert_eq!(
            posts,
            vec![
                format!("api {prefix}/hooks/7/deliveries/11/attempts --method POST"),
                format!("api {prefix}/hooks/7/deliveries/15/attempts --method POST")
            ]
        );
        assert!(
            calls(&stub)
                .iter()
                .any(|c| c == &format!("api {prefix}/hooks/7/deliveries/1011"))
        );
    }
}

#[test]
fn hook_redeliver_dry_run_paginates_lists_and_never_posts_or_reads_payloads() {
    for family in ["repo", "org"] {
        for format in ["json", "text"] {
            let stub = stub(vec![delivery(11, 500, SINCE), delivery(12, 200, SINCE)]);
            let out = single(&stub, family, true, format, "all");
            assert_eq!(out.code, 0, "{out:?}");
            no_leaks(&out);
            assert_eq!(calls(&stub).len(), 1);
            assert!(calls(&stub)[0].ends_with("--paginate --slurp"));
            if format == "json" {
                let env = parse_envelope(&out.stdout);
                assert_eq!(env["data"]["total"], 2);
                assert_eq!(env["data"]["rows"][1]["status"], "would_redeliver");
            } else {
                assert_eq!(out.stdout.lines().count(), 2);
                assert!(out.stdout.contains("hook 7 delivery 12"));
            }
        }
    }
}

#[test]
fn hook_redeliver_batch_continues_after_failed_post_and_sanitizes_errors() {
    for family in ["repo", "org"] {
        for format in ["json", "text"] {
            let stub = stub(vec![delivery(11, 500, SINCE), delivery(12, 500, SINCE)]);
            fs::write(stub.tempdir.path().join("fail-post"), "").unwrap();
            let file = stub.tempdir.path().join("rows.json");
            let target = if family == "repo" {
                "acme/widget"
            } else {
                "acme"
            };
            fs::write(&file, json!([{"target":target,"hook":7,"action":"redeliver","status":"failed","since":SINCE,"until":UNTIL}]).to_string()).unwrap();
            let out = run(
                &stub,
                family,
                false,
                format,
                &["batch", "--file", file.to_str().unwrap()],
            );
            assert_eq!(out.code, 1, "{out:?}");
            no_leaks(&out);
            assert_eq!(
                calls(&stub)
                    .iter()
                    .filter(|c| c.contains("--method POST"))
                    .count(),
                2
            );
            if format == "json" {
                let env = parse_envelope(&out.stdout);
                assert_eq!(env["data"]["failed"], 1);
                assert_eq!(
                    env["data"]["rows"][0]["error"]["kind"],
                    "hook_redelivery_failed"
                );
                assert_eq!(env["data"]["rows"][1]["status_code"], 201);
            } else {
                assert_eq!(out.stdout.lines().count(), 2);
            }
        }
    }
}

#[test]
fn hook_redeliver_reports_pending_without_relabeling_the_original_delivery() {
    let stub = stub(vec![delivery(11, 500, SINCE)]);
    fs::write(stub.tempdir.path().join("pending"), "").unwrap();
    let out = single(&stub, "repo", false, "json", "failed");
    assert_eq!(out.code, 0, "{out:?}");
    let env = parse_envelope(&out.stdout);
    assert_eq!(env["data"]["rows"][0]["delivery_status"], "pending");
    assert_eq!(env["data"]["rows"][0]["observed_delivery"], Value::Null);
}

#[test]
fn hook_redeliver_failed_or_malformed_readback_is_per_delivery_and_private() {
    for flag in ["fail-read", "bad-read"] {
        let stub = stub(vec![delivery(11, 500, SINCE), delivery(12, 500, SINCE)]);
        fs::write(stub.tempdir.path().join(flag), "").unwrap();
        let out = single(&stub, "repo", false, "json", "failed");
        assert_eq!(out.code, 1, "{out:?}");
        no_leaks(&out);
        assert_eq!(parse_envelope(&out.stdout)["data"]["failed"], 2);
        assert_eq!(
            calls(&stub)
                .iter()
                .filter(|c| c.contains("--method POST"))
                .count(),
            2
        );
    }
}

#[test]
fn hook_redeliver_reports_a_new_attempt_that_still_fails_at_the_receiver() {
    let stub = stub(vec![delivery(11, 500, SINCE)]);
    fs::write(stub.tempdir.path().join("receiver-failed"), "").unwrap();
    let out = single(&stub, "repo", false, "json", "failed");
    assert_eq!(out.code, 1, "{out:?}");
    let env = parse_envelope(&out.stdout);
    assert_eq!(env["data"]["rows"][0]["observed_delivery"], 1011);
    assert_eq!(env["data"]["rows"][0]["delivery_status"], "failed");
    assert_eq!(env["data"]["rows"][0]["status_code"], 503);
    no_leaks(&out);
}

#[test]
fn hook_redeliver_validates_window_and_entire_batch_before_provider_calls() {
    let stub = stub(vec![]);
    let out = run(
        &stub,
        "repo",
        false,
        "json",
        &[
            "redeliver",
            "--repo",
            "acme/widget",
            "--hook",
            "7",
            "--since",
            UNTIL,
            "--until",
            SINCE,
        ],
    );
    assert_eq!(out.code, 65, "{out:?}");
    let file = stub.tempdir.path().join("invalid.json");
    fs::write(&file, json!([
        {"target":"acme/widget","hook":7,"action":"redeliver","status":"failed","since":SINCE,"until":UNTIL},
        {"target":"acme/widget","hook":8,"action":"redeliver","status":"failed","since":"SECRET-CANARY","until":UNTIL}
    ]).to_string()).unwrap();
    let out = run(
        &stub,
        "repo",
        false,
        "json",
        &["batch", "--file", file.to_str().unwrap()],
    );
    assert_eq!(out.code, 65, "{out:?}");
    no_leaks(&out);
    assert!(calls(&stub).is_empty());
}

#[test]
fn hook_redeliver_success_filter_and_empty_selection_do_not_replay_failures() {
    let stub = stub(vec![delivery(11, 500, SINCE), delivery(12, 302, SINCE)]);
    let out = single(&stub, "repo", true, "json", "success");
    assert_eq!(out.code, 0, "{out:?}");
    let env = parse_envelope(&out.stdout);
    assert_eq!(env["data"]["total"], 1);
    assert_eq!(env["data"]["rows"][0]["delivery"], 12);
    let stub = self::stub(vec![]);
    let out = single(&stub, "org", false, "json", "failed");
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(parse_envelope(&out.stdout)["data"]["total"], 0);
    assert_eq!(calls(&stub).len(), 1);
}

#[test]
fn hook_redeliver_list_errors_are_private_and_duplicate_deliveries_are_attempted_once() {
    let stub = stub(vec![delivery(11, 500, SINCE), delivery(11, 500, SINCE)]);
    let out = single(&stub, "repo", false, "json", "failed");
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(
        calls(&stub)
            .iter()
            .filter(|c| c.contains("--method POST"))
            .count(),
        1
    );
    fs::write(stub.tempdir.path().join("fail-list"), "").unwrap();
    let out = single(&stub, "repo", false, "json", "failed");
    assert_eq!(out.code, 1, "{out:?}");
    no_leaks(&out);
}

#[test]
fn hook_redeliver_does_not_reuse_an_earlier_attempt_for_the_same_event() {
    let first = delivery(11, 500, SINCE);
    let mut second = delivery(12, 500, SINCE);
    second["guid"] = first["guid"].clone();
    let stub = stub(vec![first.clone(), second.clone()]);
    let mut new = first.clone();
    new["id"] = json!(1011);
    new["redelivery"] = json!(true);
    for id in [11, 12] {
        fs::write(
            stub.tempdir.path().join(format!("after-{id}.json")),
            json!([[first, second, new]]).to_string(),
        )
        .unwrap();
    }
    let out = single(&stub, "repo", false, "json", "failed");
    assert_eq!(out.code, 0, "{out:?}");
    let env = parse_envelope(&out.stdout);
    assert_eq!(env["data"]["rows"][0]["observed_delivery"], 1011);
    assert_eq!(env["data"]["rows"][1]["observed_delivery"], Value::Null);
    assert_eq!(env["data"]["rows"][1]["delivery_status"], "pending");
}

#[test]
fn hook_redeliver_does_not_attribute_a_delayed_attempt_to_a_later_post() {
    let first = delivery(11, 500, SINCE);
    let mut second = delivery(12, 500, SINCE);
    second["guid"] = first["guid"].clone();
    let stub = stub(vec![first.clone(), second.clone()]);
    fs::write(
        stub.tempdir.path().join("after-11.json"),
        json!([[first.clone(), second.clone()]]).to_string(),
    )
    .unwrap();
    let mut delayed = first.clone();
    delayed["id"] = json!(1011);
    delayed["redelivery"] = json!(true);
    fs::write(
        stub.tempdir.path().join("after-12.json"),
        json!([[first, second, delayed]]).to_string(),
    )
    .unwrap();
    let out = single(&stub, "repo", false, "json", "failed");
    assert_eq!(out.code, 0, "{out:?}");
    let env = parse_envelope(&out.stdout);
    let rows = env["data"]["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    for row in rows {
        assert_eq!(row["observed_delivery"], Value::Null);
        assert_eq!(row["delivery_status"], "pending");
    }
    assert_eq!(
        calls(&stub)
            .iter()
            .filter(|c| c.contains("--method POST"))
            .count(),
        2
    );
    assert!(!calls(&stub).iter().any(|c| c.ends_with("/deliveries/1011")));
}

#[test]
fn hook_redeliver_does_not_choose_between_multiple_new_attempts() {
    let original = delivery(11, 500, SINCE);
    let stub = stub(vec![original.clone()]);
    let mut first = original.clone();
    first["id"] = json!(1011);
    first["redelivery"] = json!(true);
    let mut second = first.clone();
    second["id"] = json!(1012);
    fs::write(
        stub.tempdir.path().join("after-11.json"),
        json!([[original, first, second]]).to_string(),
    )
    .unwrap();
    let out = single(&stub, "repo", false, "json", "failed");
    assert_eq!(out.code, 0, "{out:?}");
    let env = parse_envelope(&out.stdout);
    assert_eq!(env["data"]["rows"][0]["observed_delivery"], Value::Null);
    assert_eq!(env["data"]["rows"][0]["delivery_status"], "pending");
    assert_eq!(
        calls(&stub).len(),
        3,
        "list, POST, list; no ambiguous detail GET"
    );
    no_leaks(&out);
}

#[cfg(unix)]
#[test]
fn hook_redeliver_batch_reuses_actor_and_authorizes_each_target() {
    use super::repo_hook::identity::{CREDENTIAL, Fixture};
    for family in ["repo", "org"] {
        let f = Fixture::new();
        fs::write(f.home.path().join("gh"), format!(r#"#!/bin/sh
set -eu
dir='{}'
case "$1:$2" in
  api:user) printf 'probe\n' >> "$dir/probes"; printf '{{"login":"account-a"}}'; exit 0;;
esac
[ "$GH_TOKEN" = "$FIXTURE_ACCOUNT_A_CREDENTIAL" ] && [ -z "${{GITHUB_TOKEN:-}}" ] || exit 9
printf '%s\n' "$*" >> "$dir/calls"
printf '[[{{"id":11,"guid":"event-11","redelivery":false,"status_code":500,"delivered_at":"2026-01-02T00:00:00Z"}}]]'
"#, f.home.path().display())).unwrap();
        let targets = if family == "repo" {
            ["sandbox/widget", "sandbox/gadget"]
        } else {
            ["sandbox", "sandbox"]
        };
        let file = f.home.path().join("rows.json");
        let rows: Vec<_> = targets.iter().enumerate().map(|(i, target)| json!({"target":target,"hook":i+7,"action":"redeliver","status":"failed","since":SINCE,"until":UNTIL})).collect();
        fs::write(&file, json!(rows).to_string()).unwrap();
        let out = f
            .command("contributor")
            .args(["--dry-run", family, "hook", "batch", "--file"])
            .arg(&file)
            .output()
            .unwrap();
        assert_eq!(
            out.status.code(),
            Some(0),
            "{}",
            String::from_utf8_lossy(&out.stdout)
        );
        assert_eq!(f.count("calls"), 2);
        assert_eq!(f.count("probes"), 1);
        assert!(!String::from_utf8_lossy(&out.stdout).contains(CREDENTIAL));
        if family == "org" {
            let out = f
                .command("coordinator")
                .args(["--dry-run", "org", "hook", "batch", "--file"])
                .arg(&file)
                .output()
                .unwrap();
            assert_eq!(out.status.code(), Some(1));
            assert_eq!(
                f.count("calls"),
                2,
                "repository-only rule cannot authorize organization redelivery"
            );
        }
    }
}
