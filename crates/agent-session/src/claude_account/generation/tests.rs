use super::*;
use pretty_assertions::assert_eq;

pub(crate) fn envelope(n: u64) -> Value {
    let version = json!({"authority_id":"10000000-0000-4000-8000-000000000001", "authority_epoch":"20000000-0000-4000-8000-000000000001", "generation":n.to_string()});
    let snapshot = format!("30000000-0000-4000-8000-{n:012}");
    let time = "2030-01-01T00:00:00.000Z";
    let stores: Vec<Value> = ["account_credentials", "account_config", "keychain", "directory_preparation", "active_credentials", "proxy_projection"].iter().map(|kind| {
        let required = matches!(*kind, "account_credentials" | "account_config" | "directory_preparation");
        json!({"kind":kind,"required":required,"status":if required {"verified"} else {"not_required"},"verified_at":if required {Some(time)} else {None}})
    }).collect();
    json!({"schema_version":"agent-session.credential-state.v1", "ok":true,"provider":"claude","account":"alpha","host_id":"40000000-0000-4000-8000-000000000001","read_at":time,"error":null,
        "state":{"authority":{"version":version,"snapshot_id":snapshot,"auth":{"state":"unknown"}},
        "publication":{"status":"published","policy_revision":"1","target":version,"snapshot_id":snapshot,"attempt_id":"50000000-0000-4000-8000-000000000001", "started_at":time,"updated_at":time,"published_at":time,"stores":stores,"failure":null,
        "last_published":{"version":version,"snapshot_id":snapshot,"policy_revision":"1","published_at":time}},"refresh":null,"authority_contact":{"state":"unknown"}}})
}

#[test]
fn generation_order_is_same_account_host_authority_epoch_and_strictly_newer() {
    let old = decode(&envelope(9), "alpha").unwrap().unwrap();
    let new = decode(&envelope(10), "alpha").unwrap().unwrap();
    assert!(new.newer_than(&old));
    assert!(!old.newer_than(&new));
    assert!(!new.newer_than(&new));
    for change in ["account", "host", "authority", "epoch", "snapshot"] {
        let mut other = new.clone();
        match change {
            "account" => other.account = "beta".into(),
            "host" => other.host_id = "40000000-0000-4000-8000-000000000002".into(),
            "authority" => {
                other.version.authority_id = "10000000-0000-4000-8000-000000000002".into()
            }
            "epoch" => {
                other.version.authority_epoch = "20000000-0000-4000-8000-000000000002".into()
            }
            _ => other.snapshot_id = old.snapshot_id.clone(),
        }
        assert!(!other.newer_than(&old), "{change}");
    }
}

#[test]
fn generation_publication_requires_complete_matching_receipts() {
    for change in [
        "account",
        "provider",
        "target",
        "snapshot",
        "policy",
        "unverified",
        "missing_store",
        "duplicate_store",
        "not_required",
        "invalid_uuid",
        "invalid_variant",
        "invalid_counter",
        "revoked",
        "timestamp",
    ] {
        let mut value = envelope(2);
        let p = &mut value["state"]["publication"];
        match change {
            "account" => value["account"] = json!("beta"),
            "provider" => value["provider"] = json!("codex"),
            "target" => p["target"]["generation"] = json!("1"),
            "snapshot" => p["snapshot_id"] = json!("30000000-0000-4000-8000-000000000009"),
            "policy" => p["policy_revision"] = json!("2"),
            "unverified" => p["stores"][0]["status"] = json!("pending"),
            "missing_store" => {
                p["stores"].as_array_mut().unwrap().pop();
            }
            "duplicate_store" => p["stores"][1] = p["stores"][0].clone(),
            "not_required" => {
                p["stores"][0]["required"] = json!(false);
                p["stores"][0]["status"] = json!("not_required");
                p["stores"][0]["verified_at"] = Value::Null;
            }
            "invalid_uuid" => value["host_id"] = json!("sensitive-canary"),
            "invalid_variant" => value["host_id"] = json!("40000000-0000-4000-0000-000000000001"),
            "invalid_counter" => p["last_published"]["version"]["generation"] = json!("02"),
            "revoked" => value["state"]["authority"]["auth"]["state"] = json!("revoked"),
            _ => p["last_published"]["published_at"] = json!("invalid"),
        }
        assert!(decode(&value, "alpha").is_err(), "{change}");
    }
    for status in ["missing", "publishing", "partial", "blocked"] {
        let mut value = envelope(2);
        value["state"]["publication"]["status"] = json!(status);
        assert_eq!(decode(&value, "alpha").unwrap(), None);
    }
}

#[test]
fn generation_projection_excludes_untrusted_broker_metadata() {
    let mut value = envelope(2);
    value["accessToken"] = json!("sensitive-canary");
    value["state"]["authority"]["private_path"] = json!("sensitive-canary");
    let result = serde_json::to_string(&decode(&value, "alpha").unwrap()).unwrap();
    assert!(!result.contains("sensitive-canary"));
}
