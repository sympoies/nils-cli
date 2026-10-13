//! Isolated compatibility hold and ordinary state exclusivity fixtures.

use std::{fs, process::Command};

use forge_cli::ops::label::{LabelTarget, validate_label_inputs};
use pretty_assertions::assert_eq;
use serde_json::{Value, json};
use tempfile::TempDir;

use super::support::{StubEnv, parse_envelope, run_forge_cli_in};

const COMPATIBILITY_HOLD: &str = "state::do-not-merge";
const TARGETS: [LabelTarget; 3] = [LabelTarget::Issue, LabelTarget::Pr, LabelTarget::Mr];

fn catalog() -> Value {
    let labels = [
        COMPATIBILITY_HOLD,
        "state::ready",
        "state::review",
        "state::do-not-merge-later",
        "type::bug",
        "type::feature",
        "do-not-merge",
        "control::hold",
    ]
    .map(|name| {
        json!({
            "name": name, "color": "123456", "applies_to": ["issue", "pr", "mr"]
        })
    });
    json!({
        "groups": [
            {"name": "state", "prefix": "state::", "exclusive": true},
            {"name": "type", "prefix": "type::", "exclusive": true},
            {"name": "control", "prefix": "control::", "exclusive": false}
        ],
        "labels": labels
    })
}

fn validate(
    catalog: &Value,
    labels: &[&str],
    target: LabelTarget,
) -> Result<(), forge_cli::error::ForgeError> {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("labels.json");
    fs::write(&path, serde_json::to_vec(catalog).unwrap()).unwrap();
    validate_label_inputs(
        &labels
            .iter()
            .map(|name| (*name).to_string())
            .collect::<Vec<_>>(),
        Some(path.to_str().unwrap()),
        true,
        target,
    )
}

#[test]
fn compatibility_hold_and_one_state_are_valid_in_both_orders_for_every_target() {
    for target in TARGETS {
        for group in ["state", "control"] {
            let mut catalog = catalog();
            catalog["labels"][0]["group"] = json!(group);
            for labels in [
                [COMPATIBILITY_HOLD, "state::ready"],
                ["state::ready", COMPATIBILITY_HOLD],
            ] {
                validate(&catalog, &labels, target).unwrap_or_else(|error| {
                    panic!("{target:?} {labels:?} (declared group {group}) rejected: {error}")
                });
            }
        }
    }
}

#[test]
fn hold_alone_and_workflow_state_alone_are_valid() {
    for target in TARGETS {
        for label in [COMPATIBILITY_HOLD, "state::ready"] {
            validate(&catalog(), &[label], target).unwrap();
        }
    }
}

#[test]
fn two_ordinary_states_conflict_with_or_without_compatibility_hold() {
    for target in TARGETS {
        for labels in [
            vec!["state::ready", "state::review"],
            vec![COMPATIBILITY_HOLD, "state::ready", "state::review"],
            vec!["state::ready", COMPATIBILITY_HOLD, "state::review"],
            vec!["state::ready", "state::review", COMPATIBILITY_HOLD],
            vec![COMPATIBILITY_HOLD, "state::ready", "state::ready"],
        ] {
            assert_eq!(
                validate(&catalog(), &labels, target).unwrap_err().kind(),
                "label_group_conflict"
            );
        }
    }
}

#[test]
fn lookalike_holds_and_other_exclusive_groups_are_not_exempt() {
    for target in TARGETS {
        for labels in [
            vec!["state::ready", "state::do-not-merge-later"],
            vec!["type::bug", "type::feature"],
            vec![COMPATIBILITY_HOLD, "type::bug", "type::feature"],
        ] {
            assert_eq!(
                validate(&catalog(), &labels, target).unwrap_err().kind(),
                "label_group_conflict"
            );
        }
    }
}

#[test]
fn compatibility_hold_still_requires_catalog_membership_and_applicability() {
    let mut missing = catalog();
    missing["labels"].as_array_mut().unwrap().remove(0);
    for target in TARGETS {
        assert_eq!(
            validate(&missing, &[COMPATIBILITY_HOLD, "state::ready"], target)
                .unwrap_err()
                .kind(),
            "label_unknown"
        );
    }
    let mut pr_only = catalog();
    pr_only["labels"][0]["applies_to"] = json!(["pr"]);
    for target in [LabelTarget::Issue, LabelTarget::Mr] {
        assert_eq!(
            validate(&pr_only, &["state::ready", COMPATIBILITY_HOLD], target)
                .unwrap_err()
                .kind(),
            "label_not_applicable"
        );
    }
}

#[test]
fn independent_hold_labels_coexist_with_one_workflow_state() {
    for target in TARGETS {
        validate(
            &catalog(),
            &["do-not-merge", "control::hold", "state::ready"],
            target,
        )
        .unwrap();
        validate(
            &catalog(),
            &[
                COMPATIBILITY_HOLD,
                "do-not-merge",
                "control::hold",
                "state::ready",
            ],
            target,
        )
        .unwrap();
    }
}

#[test]
fn held_pr_with_workflow_state_still_refuses_merge() {
    for hold in [COMPATIBILITY_HOLD, "do-not-merge", "control::hold"] {
        let repo = TempDir::new().unwrap();
        assert!(
            Command::new("git")
                .args(["init", "--quiet"])
                .current_dir(repo.path())
                .status()
                .unwrap()
                .success()
        );
        let stub = StubEnv::new();
        let merged = stub.tempdir.path().join("merged");
        let labels = json!([[{"name": hold}, {"name": "state::ready"}]]);
        let body = format!(
            r#"#!/bin/sh
case "$1 $2" in
  "pr view") printf '%s\n' '{{"number":7,"url":"https://github.com/example/project/pull/7","state":"OPEN","isDraft":false,"title":"Fixture","headRefName":"feature","baseRefName":"main","headRefOid":"head123"}}' ;;
  "api graphql") printf '%s\n' '{{"data":{{"repository":{{"object":null}}}}}}' ;;
  "api --paginate") printf '%s\n' '{labels}' ;;
  "pr merge") touch '{merged}'; exit 99 ;;
  *) echo "unexpected fixture call: $*" >&2; exit 99 ;;
esac
"#,
            merged = merged.display()
        );
        let stub = stub.gh_stub(&body);
        let out = run_forge_cli_in(
            &stub,
            &[
                "--provider",
                "github",
                "--repo",
                "example/project",
                "--format",
                "json",
                "pr",
                "merge",
                "7",
            ],
            Some(repo.path()),
        );
        assert_eq!(out.code, 65, "{} {}", out.stdout, out.stderr);
        assert_eq!(
            parse_envelope(&out.stdout)["error"]["code"],
            "pr_hold_active"
        );
        assert!(!merged.exists(), "workflow state must not release a hold");
    }
}
