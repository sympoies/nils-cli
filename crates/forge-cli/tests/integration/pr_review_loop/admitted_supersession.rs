//! Closed-set regressions for the admitted supersession review findings.
use super::*;
use forge_cli::ops::pr_review_handoff::ReviewHandoff;
use forge_cli::ops::review_state::supersession::{self, SupersessionReason, SupersessionRequest};
use forge_cli::ops::review_state::{
    self, ReviewFindingObservation, ReviewStatePayload, ReviewStateRecord,
};
use pretty_assertions::{assert_eq, assert_ne};

const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn append(records: &mut Vec<ReviewStateRecord>, payload: ReviewStatePayload, owner: Option<u64>) {
    records.push(
        ReviewStateRecord::new(
            "acme/widgets",
            7,
            B,
            records.len() as u64,
            records.last().map(|r| r.record_digest.clone()),
            payload,
        )
        .unwrap()
        .with_assignment_generation(owner)
        .unwrap(),
    );
}
fn request(records: &[ReviewStateRecord]) -> SupersessionRequest {
    SupersessionRequest {
        superseded_record: records[2].record_digest.clone(),
        reason: SupersessionReason::SignedFastForward,
    }
}
fn input(request: &SupersessionRequest, rows: &[ReviewFindingObservation]) -> serde_json::Value {
    serde_json::json!({"schema_version":supersession::INPUT_SCHEMA,"supersession":request,"findings":rows})
}
fn commits(count: usize) -> Vec<String> {
    let mut commits: Vec<_> = (1..count).map(|i| format!("{i:040x}")).collect();
    commits.push(B.into());
    commits
}
fn comparison(commits: &[String]) -> String {
    serde_json::json!({"status":"ahead","ahead_by":commits.len(),"behind_by":0,"total_commits":commits.len(),"base_commit":{"sha":A},"merge_base_commit":{"sha":A},"commits":commits.iter().map(|sha| serde_json::json!({"sha":sha,"commit":{"verification":{"verified":true,"reason":"valid"}}})).collect::<Vec<_>>()}).to_string()
}

#[test]
fn supersession_retry_after_same_head_reassignment_refuses_before_post() {
    let (mut records, rows) = super::supersession::stopped_fixture();
    let request = request(&records);
    let mut owner = ReviewHandoff {
        coordinator_digest: review_state::sha256_digest(b"worker-session"),
        reviewer_digest: review_state::sha256_digest(b"reviewer-session"),
        review_author: "review-app[bot]".into(),
        base_sha: A.into(),
        assigned_head: B.into(),
        returned_reason: None,
        assignment_generation: 1,
        surrendered: false,
        coordinator_transfer: None,
        base_ref: Some("main".into()),
    };
    append(
        &mut records,
        ReviewStatePayload::ReviewHandoff {
            handoff: owner.clone(),
        },
        Some(1),
    );
    let chain = super::supersession::chain(&records);
    let recovered = supersession::apply(
        &chain,
        B,
        chain.tip_digest.as_deref(),
        &request,
        &rows,
        &[B.into()],
    )
    .unwrap();
    append(&mut records, recovered.into_payload(), Some(1));
    let chain = super::supersession::chain(&records);
    assert!(
        !supersession::apply(
            &chain,
            B,
            chain.tip_digest.as_deref(),
            &request,
            &rows,
            &[B.into()]
        )
        .unwrap()
        .changed
    );
    owner.surrendered = true;
    append(
        &mut records,
        ReviewStatePayload::ReviewHandoff {
            handoff: owner.clone(),
        },
        Some(1),
    );
    owner.surrendered = false;
    owner.assignment_generation = 2;
    append(
        &mut records,
        ReviewStatePayload::ReviewHandoff { handoff: owner },
        Some(2),
    );
    let chain = super::supersession::chain(&records);
    let original_markers: Vec<_> = records.iter().map(|r| r.marker().unwrap()).collect();
    let stub = super::super::pr_review_handoff::fixture(B, &records, vec![])
        .env("AGENT_SESSION_ID", "reviewer-session")
        .env("AGENT_REVIEW_ASSIGNMENT_GENERATION", "2");
    let script_path = &stub
        .envs
        .iter()
        .rev()
        .find(|(key, _)| key == "FORGE_CLI_GH_BIN")
        .unwrap()
        .1;
    let script = fs::read_to_string(script_path).unwrap().replace("esac\necho \"unscripted mutation\"", &format!("  \"api repos/acme/widgets/compare/{A}...{B}?per_page=100&page=1\") cat <<'JSON'\n{}\nJSON\n    exit 0 ;;\nesac\necho \"unscripted mutation\"",comparison(&[B.into()])));
    let file = write_findings(&stub, "retry.json", &input(&request, &rows).to_string());
    let stub = stub.gh_stub(&script);
    let out = run_forge_cli(
        &stub,
        &[
            "--provider",
            "github",
            "--repo",
            "acme/widgets",
            "--format",
            "json",
            "pr",
            "review-loop",
            "observe",
            "7",
            "--expected-head",
            B,
            "--expected-state",
            chain.tip_digest.as_deref().unwrap(),
            "--findings-file",
            &file,
        ],
    );
    let log = fs::read_to_string(stub.tempdir.path().join("calls.log")).unwrap();
    for marker in PROVIDER_WRITE_MARKERS {
        assert!(
            !log.contains(marker),
            "provider write preceded refusal: {log}"
        );
    }
    assert_ne!(out.code, 0, "{}", out.stdout);
    assert_eq!(
        parse_envelope(&out.stdout)["error"]["code"],
        "review_supersession_invalid",
        "{}",
        out.stdout
    );
    assert_eq!(
        super::supersession::chain(&records)
            .records
            .iter()
            .map(|r| r.marker().unwrap())
            .collect::<Vec<_>>(),
        original_markers
    );
}

#[test]
fn validate_catalog_and_text_label_match_supersession_emissions() {
    let catalog: serde_yaml_ng::Value =
        serde_yaml_ng::from_str(include_str!("../../../docs/specs/forge-cli-ops-v1.yaml")).unwrap();
    let op = catalog["operations"]
        .as_sequence()
        .unwrap()
        .iter()
        .find(|op| op["id"].as_str() == Some("pr.review-loop.validate"))
        .unwrap();
    let (records, rows) = super::supersession::stopped_fixture();
    let stub = StubEnv::new();
    let body = input(&request(&records), &rows);
    let file = write_findings(&stub, "valid.json", &body.to_string());
    let out = run_forge_cli(
        &stub,
        &[
            "--format",
            "json",
            "pr",
            "review-loop",
            "validate",
            "--findings-file",
            &file,
        ],
    );
    assert_eq!(out.code, 0, "{}", out.stdout);
    let shape = parse_envelope(&out.stdout)["data"]["shape"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(
        op["output"]["data"].as_str().unwrap().contains(&shape),
        "catalog omits emitted {shape}"
    );
    let out = run_forge_cli(
        &stub,
        &["pr", "review-loop", "validate", "--findings-file", &file],
    );
    assert_eq!(out.code, 0, "{}", out.stdout);
    assert!(
        out.stdout.contains("chain-bound supersession envelope"),
        "{}",
        out.stdout
    );
    let mut invalid = body;
    invalid["supersession"]["superseded_record"] = "invalid".into();
    let file = write_findings(&stub, "invalid.json", &invalid.to_string());
    let out = run_forge_cli(
        &stub,
        &[
            "--format",
            "json",
            "pr",
            "review-loop",
            "validate",
            "--findings-file",
            &file,
        ],
    );
    assert_ne!(out.code, 0);
    let code = parse_envelope(&out.stdout)["error"]["code"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(
        op["validations"]
            .as_sequence()
            .unwrap()
            .iter()
            .any(|v| v.as_str() == Some(&code)),
        "catalog omits emitted {code}"
    );
}

#[test]
fn unencodable_commit_proof_is_rejected_before_receipt_admission() {
    let (records, rows) = super::supersession::stopped_fixture();
    let chain = super::supersession::chain(&records);
    let request = request(&records);
    let oversized = commits(1000);
    // The old advertised proof array alone exceeds the entire hex marker cap,
    // independently of state, receipt metadata and the visible comment wrapper.
    assert!(serde_json::to_vec(&oversized).unwrap().len() * 2 > 64 * 1024);
    let result = supersession::apply(
        &chain,
        B,
        chain.tip_digest.as_deref(),
        &request,
        &rows,
        &oversized,
    );
    assert_eq!(result.unwrap_err().kind(), "review_supersession_invalid");
    let result = supersession::apply(
        &chain,
        B,
        chain.tip_digest.as_deref(),
        &request,
        &rows,
        &commits(101),
    );
    assert_eq!(result.unwrap_err().kind(), "review_supersession_invalid");
}

#[test]
fn supported_hundred_commit_boundary_is_encodable_and_replays() {
    let (mut records, rows) = super::supersession::stopped_fixture();
    let chain = super::supersession::chain(&records);
    let transition = supersession::apply(
        &chain,
        B,
        chain.tip_digest.as_deref(),
        &request(&records),
        &rows,
        &commits(100),
    )
    .unwrap();
    let state = transition.state.clone();
    append(&mut records, transition.into_payload(), None);
    let appended = records.last().unwrap();
    assert!(appended.marker().unwrap().len() <= 64 * 1024);
    assert!(
        review_state::render_state_comment_body(appended, None)
            .unwrap()
            .len()
            <= 64 * 1024
    );
    let chain = super::supersession::chain(&records);
    assert_eq!(review_state::latest_review_loop_state(&chain), Some(&state));
    assert_eq!(state.extensions.len(), 1);
    assert_eq!(state.budget.max_auto_reopens_per_fingerprint, 0);
}

#[test]
fn live_hundred_commit_boundary_verifies_the_complete_proof_and_retries_once() {
    super::supersession::live_supersession_with_commits(&commits(100));
}

#[test]
fn provider_over_limit_is_refused_on_the_first_page_before_any_write() {
    let (records, rows) = super::supersession::stopped_fixture();
    let stub = StubEnv::new();
    let file = write_findings(
        &stub,
        "over-limit.json",
        &input(&request(&records), &rows).to_string(),
    );
    let script = super::supersession::compare_stub(&stub, &records)
        .replace("\"ahead_by\":1", "\"ahead_by\":101")
        .replace("\"total_commits\":1", "\"total_commits\":101");
    let stub = stub.gh_stub(&script);
    let out = run_forge_cli(
        &stub,
        &[
            "--provider",
            "github",
            "--repo",
            "acme/widgets",
            "--format",
            "json",
            "--dry-run",
            "pr",
            "review-loop",
            "observe",
            "7",
            "--expected-head",
            B,
            "--expected-state",
            &records[2].record_digest,
            "--findings-file",
            &file,
        ],
    );
    assert_eq!(out.code, 0, "{}", out.stdout);
    let envelope = parse_envelope(&out.stdout);
    assert_eq!(envelope["data"]["preflight_ok"], false);
    assert!(
        out.stdout.contains("between 1 and 100 commits"),
        "{}",
        out.stdout
    );
    let log = gh_args_log(&stub);
    assert_eq!(log.matches("compare/").count(), 1, "{log}");
    for marker in PROVIDER_WRITE_MARKERS {
        assert!(!log.contains(marker), "unexpected mutation: {log}");
    }
}
