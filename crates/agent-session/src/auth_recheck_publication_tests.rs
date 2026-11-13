//! Publication-domain refusal and exhausted-recheck guidance regressions.
use super::*;
use pretty_assertions::assert_eq;

#[test]
fn publication_identity_shapes_remain_quarantined() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::tempdir().unwrap();
    let _broker = broker(tmp.path(), &lock);
    publish(tmp.path(), 2);
    let context = context(tmp.path());
    let tmux = fake_tmux(tmp.path());
    let record = quarantined(&context, tmp.path(), &tmux);
    let path = crate::session_dir(&context, ID).join("auth-incidents.json");
    let before = fs::read(&path).unwrap();
    for shape in [
        "host",
        "authority",
        "epoch",
        "same_generation_different_snapshot",
        "account",
        "partial",
        "failed",
        "malformed",
        "unpublished",
        "lower",
        "equal",
    ] {
        let mut publication = crate::claude_account::generation::tests::envelope(2);
        match shape {
            "host" => publication["host_id"] = json!("40000000-0000-4000-8000-000000000002"),
            "authority" | "epoch" => {
                let (field, value) = if shape == "authority" {
                    ("authority_id", "10000000-0000-4000-8000-000000000002")
                } else {
                    ("authority_epoch", "20000000-0000-4000-8000-000000000002")
                };
                for pointer in [
                    "/state/authority/version",
                    "/state/publication/target",
                    "/state/publication/last_published/version",
                ] {
                    publication.pointer_mut(pointer).unwrap()[field] = json!(value);
                }
            }
            "same_generation_different_snapshot" => {
                for pointer in [
                    "/state/authority",
                    "/state/publication",
                    "/state/publication/last_published",
                ] {
                    publication.pointer_mut(pointer).unwrap()["snapshot_id"] =
                        json!("30000000-0000-4000-8000-000000000099");
                }
            }
            "account" => publication["account"] = json!("beta"),
            "partial" => publication["state"]["publication"]["status"] = json!("partial"),
            "failed" => {
                publication["state"]["publication"]["failure"] = json!({"code":"synthetic_failure"})
            }
            "malformed" => publication["state"]["publication"]["published_at"] = json!("invalid"),
            "unpublished" => publication["state"]["publication"]["status"] = json!("missing"),
            "lower" => publication = crate::claude_account::generation::tests::envelope(1),
            "equal" => {}
            _ => unreachable!(),
        }
        fs::write(
            tmp.path().join("credential-state.json"),
            publication.to_string(),
        )
        .unwrap();
        let error = recheck(&context, &record, &tmux).unwrap_err();
        assert_eq!(error.code(), "claude-account-recheck-refused", "{shape}");
        let expected = if shape == "equal" {
            "provider_verification_required"
        } else {
            "credentials_not_republished"
        };
        assert_eq!(error.details().unwrap()["reason"], expected, "{shape}");
        assert_eq!(fs::read(&path).unwrap(), before, "{shape}");
        assert_eq!(
            crate::auth_incident::recovery::ensure_input_allowed(&context, &record)
                .unwrap_err()
                .code(),
            "auth-quarantined",
            "{shape}"
        );
    }
}

#[test]
fn revoked_exhaustion_keeps_credential_recheck_guidance() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::tempdir().unwrap();
    let _broker = broker(tmp.path(), &lock);
    let context = context(tmp.path());
    let tmux = fake_tmux(tmp.path());
    let record = quarantined(&context, tmp.path(), &tmux);
    let mut publication = crate::claude_account::generation::tests::envelope(1);
    publication["state"]["authority"]["auth"]["state"] = json!("revoked");
    fs::write(
        tmp.path().join("credential-state.json"),
        publication.to_string(),
    )
    .unwrap();
    let start = incident_store(&context)["incidents"][0]["observed_at"]
        .as_str()
        .unwrap()
        .parse::<jiff::Timestamp>()
        .unwrap()
        .as_second();
    for offset in [30, 120, 600] {
        crate::auth_incident::auto_settle::tick(&context, &record, start + offset).unwrap();
    }
    let store = incident_store(&context);
    assert_eq!(
        store["incidents"][0]["recovery_result"],
        "recheck_exhausted"
    );
    assert_eq!(
        store["incidents"][0]["auto_settle"]["attempts"][2]["result"],
        "credentials_not_republished"
    );
    assert!(crate::auth_incident::recovery::ensure_input_allowed(&context, &record).is_err());
    crate::auth_incident::tick(&context, &record, &tmux, "fixture", Some(false)).unwrap();
    let alerts = crate::auth_incident::alert_delivery::list(&context, 100).unwrap();
    let exhausted = alerts["alerts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|alert| alert["result"] == "recheck_exhausted")
        .unwrap();
    assert_eq!(
        exhausted["recovery_action"],
        "same_account_credential_recheck"
    );
}
