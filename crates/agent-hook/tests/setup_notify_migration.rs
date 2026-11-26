mod support;

use std::fs;
use std::path::PathBuf;

use pretty_assertions::assert_eq;
use support::Fixture;

const POLICY: &str = r#"schema_version = "agent-hook.policy.v1"
bundle_id = "runtime-kit"
version = "2026.07.20.1"

[[rules]]
id = "runtime.pre-edit"
products = ["codex"]
events = ["PreToolUse"]
priority = 10
mode = "enforce"
failure_posture = "closed"
override_class = "locked"
capability = { id = "decision.allow.v1", reason_code = "pre-edit" }
"#;

// The generated 1.41.3 notify assignment with the #4099 suffix and no final LF.
const PRIOR_CONFIG: &str = include_str!("fixtures/codex-notify-1.41.3.toml");
const USER_TOML: &str = "\n# unrelated comment\nweb_search = \"cached\"";
const MANAGED_BLOCK: &str = "\n# >>> agent-hook:provider-ingress:v1 >>>\n";

fn write_config(fixture: &Fixture, contents: &str) -> PathBuf {
    let codex = fixture.home.join(".codex");
    fs::create_dir_all(&codex).expect("Codex directory");
    let config = codex.join("config.toml");
    fs::write(&config, contents).expect("Codex config");
    Fixture::set_private(&config);
    config
}

fn notify_argv(raw: &str) -> Vec<String> {
    let document = raw.parse::<toml_edit::DocumentMut>().expect("Codex TOML");
    document["notify"]
        .as_array()
        .expect("notify array")
        .iter()
        .map(|arg| arg.as_str().expect("notify argument").to_string())
        .collect()
}

#[test]
fn exact_prior_codex_notify_requires_reviewed_migration() {
    let fixture = Fixture::new(POLICY);
    let config = write_config(&fixture, PRIOR_CONFIG);
    assert!(PRIOR_CONFIG.ends_with(USER_TOML));

    let doctor = fixture.run(&["doctor", "--product", "codex", "--format", "json"], None);
    assert_eq!(doctor.code, 0, "doctor output: {}", doctor.stdout_json());
    assert_eq!(
        doctor.stdout_json()["data"][0]["status"],
        "migration-needed"
    );
    assert_eq!(fs::read_to_string(&config).unwrap(), PRIOR_CONFIG);
    assert_eq!(
        agent_hook::setup::codex_notification_ownership(&config, &notify_argv(PRIOR_CONFIG)),
        None
    );

    let preview = fixture.run(
        &[
            "setup",
            "--product",
            "codex",
            "--dry-run",
            "--format",
            "json",
        ],
        None,
    );
    assert_eq!(preview.code, 0, "setup preview: {}", preview.stdout_json());
    let preview_json = preview.stdout_json();
    let plan = &preview_json["data"];
    assert_eq!(plan["status"], "migration-needed");
    assert_eq!(plan["configured"], false);
    assert_eq!(plan["would_configure"], true);
    assert_eq!(plan["would_change"], true);
    assert_eq!(plan["apply_allowed"], false);
    let digest = plan["plan_digest"].as_str().expect("plan digest");
    assert_eq!(fs::read_to_string(&config).unwrap(), PRIOR_CONFIG);

    let apply = ["setup", "--product", "codex", "--apply", "--format", "json"];
    let unreviewed = fixture.run(&apply, None);
    assert_eq!(unreviewed.code, 65);
    assert_eq!(
        unreviewed.stdout_json()["error"]["code"],
        "setup-plan-digest-required"
    );
    assert_eq!(fs::read_to_string(&config).unwrap(), PRIOR_CONFIG);

    let mut reviewed_args = apply.to_vec();
    reviewed_args.extend(["--expected-plan-digest", digest]);
    let reviewed = fixture.run(&reviewed_args, None);
    assert_eq!(
        reviewed.code,
        0,
        "reviewed apply: {}",
        reviewed.stdout_json()
    );
    assert_eq!(reviewed.stdout_json()["data"]["configured"], true);
    let migrated = fs::read_to_string(&config).expect("migrated config");
    assert_eq!(notify_argv(&migrated), support::codex_notify_argv());
    let (user_config, _) = migrated.split_once(MANAGED_BLOCK).expect("managed block");
    assert_eq!(
        &user_config[user_config.find(USER_TOML).unwrap()..],
        USER_TOML
    );

    let doctor = fixture.run(&["doctor", "--product", "codex", "--format", "json"], None);
    assert_eq!(doctor.code, 0);
    assert_eq!(doctor.stdout_json()["data"][0]["status"], "converged");
    let second = fixture.run(&apply, None);
    assert_eq!(second.code, 0);
    assert_eq!(second.stdout_json()["data"]["changed"], false);
    assert_eq!(fs::read_to_string(&config).unwrap(), migrated);
}

#[test]
fn prior_selector_lookalikes_keep_the_foreign_callback_removal_guard() {
    let fixture = Fixture::new(POLICY);
    for foreign in [
        PRIOR_CONFIG.replace("exit 127\"\"\"", "exit 126\"\"\""),
        PRIOR_CONFIG.replace(
            "\"--agent\", \"codex\"]",
            "\"--agent\", \"codex\", \"--user-flag\"]",
        ),
    ] {
        assert_ne!(foreign, PRIOR_CONFIG);
        let config = write_config(&fixture, &foreign);
        for args in [
            &["doctor", "--product", "codex", "--format", "json"][..],
            &[
                "setup",
                "--product",
                "codex",
                "--dry-run",
                "--format",
                "json",
            ],
        ] {
            let refused = fixture.run(args, None);
            assert_eq!(refused.code, 65);
            assert_eq!(
                refused.stdout_json()["error"]["code"],
                "provider-notification-config-nonreversible"
            );
            assert_eq!(fs::read_to_string(&config).unwrap(), foreign);
        }
    }
}
