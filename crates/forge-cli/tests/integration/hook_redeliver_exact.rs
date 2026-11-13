//! Exact selection reuses the existing portable paginated provider fixture.
use super::*;
use pretty_assertions::{assert_eq, assert_ne};

fn exact(stub: &StubEnv, family: &str, dry: bool, format: &str, ids: &[&str]) -> CmdOutput {
    let (flag, target) = if family == "repo" {
        ("--repo", "acme/widget")
    } else {
        ("--org", "acme")
    };
    let mut args = vec!["redeliver", flag, target, "--hook", "7"];
    for id in ids {
        args.extend(["--delivery-id", id]);
    }
    run(stub, family, dry, format, &args)
}

#[test]
fn hook_redeliver_exact_one_and_several_ids_across_pages_read_back_once() {
    for family in ["repo", "org"] {
        for ids in [vec!["12"], vec!["11", "12", "11"]] {
            let stub = stub(vec![
                delivery(11, 500, SINCE),
                delivery(12, 200, UNTIL),
                delivery(13, 503, SINCE),
            ]);
            let out = exact(&stub, family, false, "json", &ids);
            assert_eq!(out.code, 0, "{out:?}");
            no_leaks(&out);
            let env = parse_envelope(&out.stdout);
            let rows = env["data"]["rows"].as_array().unwrap();
            let expected = if ids.len() == 1 {
                vec![12]
            } else {
                vec![11, 12]
            };
            assert_eq!(
                rows.iter()
                    .map(|r| r["delivery"].as_u64().unwrap())
                    .collect::<Vec<_>>(),
                expected
            );
            for row in rows {
                assert_eq!(
                    row["observed_delivery"].as_u64(),
                    Some(row["delivery"].as_u64().unwrap() + 1000)
                );
                assert_eq!(row["status_code"], 201);
            }
            assert_eq!(
                calls(&stub)
                    .iter()
                    .filter(|c| c.contains("--method POST"))
                    .count(),
                expected.len()
            );
            assert!(!calls(&stub).iter().any(|c| c.contains("/13/attempts")));
        }
    }
}

#[test]
fn hook_redeliver_exact_dry_run_previews_only_requested_ids_and_current_status() {
    for family in ["repo", "org"] {
        for format in ["json", "text"] {
            let stub = stub(vec![
                delivery(11, 500, SINCE),
                delivery(12, 200, UNTIL),
                delivery(13, 503, SINCE),
            ]);
            let out = exact(&stub, family, true, format, &["11", "12"]);
            assert_eq!(out.code, 0, "{out:?}");
            no_leaks(&out);
            assert_eq!(
                calls(&stub).len(),
                1,
                "dry-run only lists all delivery pages"
            );
            assert!(calls(&stub)[0].ends_with("--paginate --slurp"));
            if format == "json" {
                let env = parse_envelope(&out.stdout);
                assert_eq!(env["data"]["total"], 2);
                assert_eq!(env["data"]["rows"][0]["delivery"], 11);
                assert_eq!(env["data"]["rows"][0]["delivery_status"], "failed");
                assert_eq!(env["data"]["rows"][1]["delivery"], 12);
                assert_eq!(env["data"]["rows"][1]["delivery_status"], "success");
                assert_eq!(env["data"]["rows"][1]["status_code"], 200);
            } else {
                assert_eq!(out.stdout.lines().count(), 2);
                assert!(
                    out.stdout
                        .contains("delivery 11: would_redeliver failed 500")
                );
                assert!(
                    out.stdout
                        .contains("delivery 12: would_redeliver success 200")
                );
            }
        }
    }
}

#[test]
fn hook_redeliver_exact_unknown_or_mixed_ids_refuse_before_any_post() {
    for family in ["repo", "org"] {
        for dry in [false, true] {
            for ids in [vec!["99"], vec!["11", "99"]] {
                let stub = stub(vec![delivery(11, 500, SINCE)]);
                let out = exact(&stub, family, dry, "json", &ids);
                assert_eq!(out.code, 1, "{out:?}");
                let env = parse_envelope(&out.stdout);
                assert_eq!(
                    env["data"]["rows"][0]["error"]["kind"],
                    "hook_delivery_not_found"
                );
                assert!(
                    env["data"]["rows"][0]["error"]["message"]
                        .as_str()
                        .unwrap()
                        .contains("99")
                );
                assert_eq!(
                    calls(&stub).len(),
                    1,
                    "all ids must be resolved before any POST"
                );
                no_leaks(&out);
            }
        }
    }
}

#[test]
fn hook_redeliver_exact_batch_selects_ids_and_preflights_unknown_ids() {
    for family in ["repo", "org"] {
        for dry in [false, true] {
            for ids in [json!([11, 12]), json!([11, 99])] {
                let stub = stub(vec![
                    delivery(11, 500, SINCE),
                    delivery(12, 200, UNTIL),
                    delivery(13, 500, SINCE),
                ]);
                let file = stub.tempdir.path().join("exact.json");
                let target = if family == "repo" {
                    "acme/widget"
                } else {
                    "acme"
                };
                fs::write(
                    &file,
                    json!([{"action":"redeliver","target":target,"hook":7,"delivery_ids":ids}])
                        .to_string(),
                )
                .unwrap();
                let out = run(
                    &stub,
                    family,
                    dry,
                    "json",
                    &["batch", "--file", file.to_str().unwrap()],
                );
                let unknown = ids[1] == 99;
                assert_eq!(out.code, if unknown { 1 } else { 0 }, "{out:?}");
                let env = parse_envelope(&out.stdout);
                if unknown {
                    assert_eq!(
                        env["data"]["rows"][0]["error"]["kind"],
                        "hook_delivery_not_found"
                    );
                    assert!(
                        env["data"]["rows"][0]["error"]["message"]
                            .as_str()
                            .unwrap()
                            .contains("99")
                    );
                } else {
                    assert_eq!(env["data"]["total"], 2);
                    assert_eq!(env["data"]["rows"][0]["delivery"], 11);
                    assert_eq!(env["data"]["rows"][1]["delivery"], 12);
                }
                assert_eq!(
                    calls(&stub)
                        .iter()
                        .filter(|c| c.contains("--method POST"))
                        .count(),
                    if dry || unknown { 0 } else { 2 }
                );
                no_leaks(&out);
            }
        }
    }
}

#[test]
fn hook_redeliver_exact_refuses_filters_and_zero_ids_without_provider_calls() {
    let stub = stub(vec![]);
    for extra in [
        vec!["--status", "failed"],
        vec!["--since", SINCE],
        vec!["--until", UNTIL],
        vec!["--delivery-id", "0"],
    ] {
        let mut args = vec![
            "redeliver",
            "--repo",
            "acme/widget",
            "--hook",
            "7",
            "--delivery-id",
            "11",
        ];
        args.extend(extra);
        let out = run(&stub, "repo", false, "json", &args);
        assert_ne!(out.code, 0, "{out:?}");
    }
    for fields in [
        json!({"delivery_ids":[11],"status":"failed"}),
        json!({"delivery_ids":[11],"since":SINCE,"until":UNTIL}),
        json!({"delivery_ids":[0]}),
        json!({"delivery_ids":[]}),
    ] {
        let mut row = json!({"action":"redeliver","target":"acme/widget","hook":7});
        row.as_object_mut()
            .unwrap()
            .extend(fields.as_object().unwrap().clone());
        let file = stub.tempdir.path().join("invalid-exact.json");
        fs::write(&file, json!([row]).to_string()).unwrap();
        let out = run(
            &stub,
            "repo",
            false,
            "json",
            &["batch", "--file", file.to_str().unwrap()],
        );
        assert_eq!(out.code, 65, "{out:?}");
    }
    assert!(calls(&stub).is_empty());
}
