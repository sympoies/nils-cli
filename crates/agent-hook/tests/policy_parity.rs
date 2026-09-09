use agent_hook::policy_parity::{DSH_CAPABILITY_GROUP_SCHEMA_VERSION, DshCapabilityGroup, DshTier};
use pretty_assertions::assert_eq;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Fixture {
    schema_version: String,
    capabilities: Vec<CapabilityFixture>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CapabilityFixture {
    id: DshCapabilityGroup,
    migration_task: String,
    tier: DshTier,
}

#[test]
fn dsh_capability_group_schema_matches_the_frozen_migration_fixture() {
    let fixture: Fixture = serde_json::from_str(include_str!(
        "fixtures/dsh-policy-capability-groups.v1.json"
    ))
    .expect("valid DSH capability group fixture");

    assert_eq!(fixture.schema_version, DSH_CAPABILITY_GROUP_SCHEMA_VERSION);
    assert_eq!(fixture.capabilities.len(), 23);
    assert_eq!(
        fixture
            .capabilities
            .iter()
            .map(|entry| entry.id)
            .collect::<Vec<_>>(),
        DshCapabilityGroup::ALL,
    );
    assert!(
        fixture
            .capabilities
            .iter()
            .all(|entry| matches!(entry.migration_task.as_str(), "2.3" | "3.2" | "3.3" | "3.4"))
    );
    for entry in &fixture.capabilities {
        assert_eq!(
            entry.tier,
            entry.id.tier(),
            "{} tier drifted from the frozen table",
            entry.id.as_str()
        );
    }
}

#[test]
fn every_dsh_capability_group_has_exactly_one_tier_with_the_accepted_defaults() {
    let integrity = [
        DshCapabilityGroup::OwnerUnclaimed,
        DshCapabilityGroup::SemanticConflict,
        DshCapabilityGroup::OperationLifecycle,
        DshCapabilityGroup::AgentScopeLockGuard,
        DshCapabilityGroup::CheckoutLeaseGuard,
        DshCapabilityGroup::McpSecretScan,
        DshCapabilityGroup::FinishLineRecord,
    ];
    let governed = [
        DshCapabilityGroup::BlockDirectGitCommit,
        DshCapabilityGroup::BlockDirectGitWorktree,
        DshCapabilityGroup::BlockDirectPrCreate,
        DshCapabilityGroup::BlockUnsafeDefaultDelivery,
        DshCapabilityGroup::SemanticCommitBodyGate,
        DshCapabilityGroup::BlockProjectMemoryWrite,
        DshCapabilityGroup::PortablePathsScan,
        DshCapabilityGroup::PreEditIntentGate,
    ];
    let reminders = [
        DshCapabilityGroup::ForgeLabelReminder,
        DshCapabilityGroup::MemoryWritePrincipleReminder,
        DshCapabilityGroup::SkillUsageReminder,
        DshCapabilityGroup::StopPrePrReminder,
        DshCapabilityGroup::UserPromptAgentMemory,
        DshCapabilityGroup::SessionStartHealthcheck,
        DshCapabilityGroup::AgentActivity,
        DshCapabilityGroup::BlockDirectPython,
    ];
    assert_eq!(
        integrity.len() + governed.len() + reminders.len(),
        DshCapabilityGroup::ALL.len()
    );
    for group in DshCapabilityGroup::ALL {
        let expected = if integrity.contains(&group) {
            DshTier::Integrity
        } else if governed.contains(&group) {
            DshTier::GovernedSeam
        } else {
            assert!(reminders.contains(&group), "{} is untiered", group.as_str());
            DshTier::Reminder
        };
        assert_eq!(group.tier(), expected, "{}", group.as_str());
        assert_eq!(
            group.remediation().is_some(),
            group.tier() == DshTier::GovernedSeam,
            "{}: exactly the Tier B seams carry remediation",
            group.as_str()
        );
        assert!(
            group.shell_subjects().is_empty() || group.tier() == DshTier::GovernedSeam,
            "{}: only Tier B groups narrow the shell classification by subject",
            group.as_str()
        );
    }
    assert_eq!(DshTier::Integrity.enforcement_default(), "block");
    assert_eq!(DshTier::GovernedSeam.enforcement_default(), "block");
    assert_eq!(DshTier::Reminder.enforcement_default(), "context");
}

#[test]
fn dsh_capability_group_schema_rejects_unknown_ids() {
    assert!(serde_json::from_str::<DshCapabilityGroup>("\"unknown-python-handler\"").is_err());
}

#[test]
fn canonical_spec_names_the_strict_dsh_ingress_and_policy_contracts() {
    let specification = include_str!("../docs/specs/agent-hook-v1.md");
    for contract in [
        "`agent-hook.dsh-ingress.v1`",
        "`agent-hook.dsh-ingress.v2`",
        "`agent-hook.dsh-ingress.v3`",
        "`agent-hook.dsh-ingress.v4`",
        "`agent-hook.dsh-ingress.v5`",
        "`dsh.policy.v1`",
    ] {
        assert!(
            specification.contains(contract),
            "canonical specification is missing {contract}"
        );
    }
    assert!(specification.contains("v1 explicitly forbids `subject`"));
    assert!(specification.contains("v2 requires this complete subject"));
    assert!(specification.contains("native allow/block admission"));
    assert!(specification.contains("bounded model"));
}

/// Marker that introduces the documented `runtime-kit.handler.v1` allowlist in
/// `agent-hook-v1.md`.
const ALLOWLIST_MARKER: &str = "The v1 allowlist is:";

/// Collect the handler IDs the spec documents, reading the backticked entries
/// of the single paragraph that follows the allowlist marker.
///
/// The parse keys on the marker line and the paragraph break, so the prose
/// around the list can be reworded freely while a missing or extra ID still
/// fails. Changing the marker itself, or splitting the list across a blank
/// line, fails loudly here rather than silently passing.
fn documented_runtime_handler_ids(spec: &str) -> Vec<String> {
    let after_marker = spec
        .split_once(ALLOWLIST_MARKER)
        .expect("agent-hook-v1.md documents the v1 allowlist")
        .1;

    let mut paragraph = String::new();
    for line in after_marker.lines() {
        if line.trim().is_empty() {
            if paragraph.is_empty() {
                continue;
            }
            break;
        }
        paragraph.push_str(line);
        paragraph.push(' ');
    }
    assert!(
        !paragraph.is_empty(),
        "the allowlist marker is followed by a documented paragraph"
    );

    let mut ids = Vec::new();
    let mut rest = paragraph.as_str();
    while let Some((_, tail)) = rest.split_once('`') {
        let (id, tail) = tail
            .split_once('`')
            .expect("each documented allowlist entry closes its backticks");
        ids.push(id.to_string());
        rest = tail;
    }
    ids
}

#[test]
fn documented_runtime_handler_allowlist_matches_the_compiled_map() {
    let documented = documented_runtime_handler_ids(include_str!("../docs/specs/agent-hook-v1.md"));
    let compiled = agent_hook::policy_parity::runtime_handler_ids();

    let mut documented_sorted = documented.clone();
    documented_sorted.sort();
    let mut compiled_sorted = compiled
        .iter()
        .map(|id| (*id).to_string())
        .collect::<Vec<_>>();
    compiled_sorted.sort();

    assert_eq!(
        documented_sorted, compiled_sorted,
        "the documented v1 allowlist and the compiled handler map disagree; \
         update both `agent-hook-v1.md` and `RUNTIME_HANDLERS` together"
    );
}

#[test]
fn documented_runtime_handler_allowlist_has_no_duplicate_entries() {
    let documented = documented_runtime_handler_ids(include_str!("../docs/specs/agent-hook-v1.md"));
    let mut unique = documented.clone();
    unique.sort();
    unique.dedup();

    assert_eq!(
        unique.len(),
        documented.len(),
        "the documented v1 allowlist lists each handler ID once"
    );
}

#[test]
fn compiled_runtime_handler_allowlist_is_sorted_and_unique() {
    let compiled = agent_hook::policy_parity::runtime_handler_ids();
    let mut expected = compiled.clone();
    expected.sort();
    expected.dedup();

    assert_eq!(
        compiled, expected,
        "`RUNTIME_HANDLERS` stays sorted with one entry per handler ID so the \
         table can be diffed against the spec"
    );
}
