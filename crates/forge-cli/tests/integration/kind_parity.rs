//! Kind-set parity check (R1 repair for nils-cli#2062).
//!
//! The authoritative forge-cli operation catalog (`forge-cli-ops-v1.yaml`) and
//! the Markdown command surfaces (`forge-cli-spec-v1.md`) must agree with the
//! single source of truth in [`PrKind`]: the full seven-kind set
//! (`feature`, `bug`, `chore`, `docs`, `ci`, `refactor`, `test`) and its
//! branch-prefix mapping. Before this check the catalog and the Markdown
//! listed only `feature|bug` for `pr create` / `pr deliver` and only `feat|fix`
//! for the branch rules, so a consumer following the contract would reject the
//! `test` kind that the code already accepts and emits.

use pretty_assertions::assert_eq;

use forge_cli::validations::PrKind;

/// The canonical kind set in declaration order, from `PrKind::all()`.
fn canonical_kinds() -> Vec<String> {
    PrKind::all()
        .iter()
        .map(|kind| kind.as_str().to_string())
        .collect()
}

/// The canonical branch prefixes, one per kind, from `PrKind::branch_prefix()`.
fn canonical_prefixes() -> Vec<String> {
    PrKind::all()
        .iter()
        .map(|kind| kind.branch_prefix().to_string())
        .collect()
}

/// Parse an `enum<...>` literal (e.g. `enum<feature|bug|...>`) into its members.
fn enum_members(literal: &str) -> Vec<String> {
    literal
        .strip_prefix("enum<")
        .and_then(|inner| inner.strip_suffix('>'))
        .map(|inner| inner.split('|').map(str::to_string).collect())
        .unwrap_or_default()
}

/// Extract the first `(a|b|c)` alternation from a regex string.
fn first_alternation(regex: &str) -> Vec<String> {
    let start = regex
        .find('(')
        .expect("branch rule must open a prefix group");
    let close = regex[start + 1..].find(')').map(|i| start + 1 + i);
    let inner = close.map(|end| &regex[start + 1..end]);
    inner
        .map(|inner| inner.split('|').map(str::to_string).collect())
        .unwrap_or_default()
}

#[test]
fn catalog_kind_enums_match_prkind_seven_kind_set() {
    let catalog: serde_yaml_ng::Value =
        serde_yaml_ng::from_str(include_str!("../../docs/specs/forge-cli-ops-v1.yaml")).unwrap();
    let kinds = canonical_kinds();

    // 1. `pr create` (under `operations`) — `--kind` input and output `kind`.
    let pr_create = catalog["operations"]
        .as_sequence()
        .unwrap()
        .iter()
        .find(|op| op["id"].as_str() == Some("pr.create"))
        .expect("pr.create operation in catalog");
    let kind_input = pr_create["inputs"]
        .as_sequence()
        .unwrap()
        .iter()
        .find(|inp| inp["name"].as_str() == Some("--kind"))
        .expect("pr.create --kind input");
    assert_eq!(
        enum_members(kind_input["type"].as_str().unwrap()),
        kinds,
        "pr.create --kind input enum must equal the PrKind seven-kind set"
    );
    assert_eq!(
        enum_members(pr_create["output"]["data"]["kind"].as_str().unwrap()),
        kinds,
        "pr.create output.kind must equal the PrKind seven-kind set"
    );

    // 2. `pr deliver` (under `macros`) — `--kind` input and output `kind`.
    let pr_deliver = catalog["macros"]
        .as_sequence()
        .unwrap()
        .iter()
        .find(|op| op["id"].as_str() == Some("pr.deliver"))
        .expect("pr.deliver macro in catalog");
    let kind_input = pr_deliver["inputs"]
        .as_sequence()
        .unwrap()
        .iter()
        .find(|inp| inp["name"].as_str() == Some("--kind"))
        .expect("pr.deliver --kind input");
    assert_eq!(
        enum_members(kind_input["type"].as_str().unwrap()),
        kinds,
        "pr.deliver --kind input enum must equal the PrKind seven-kind set"
    );
    assert_eq!(
        enum_members(pr_deliver["output"]["data"]["kind"].as_str().unwrap()),
        kinds,
        "pr.deliver output.kind must equal the PrKind seven-kind set"
    );

    // 3. `branch_name` rule must enumerate all seven branch prefixes.
    let branch_name_rule = catalog["validations_catalog"]["branch_name"]["rule"]
        .as_str()
        .unwrap();
    assert_eq!(
        first_alternation(branch_name_rule),
        canonical_prefixes(),
        "branch_name rule must enumerate all seven branch prefixes"
    );

    // 4. `branch_kind_matches` rule must enumerate every kind -> prefix mapping.
    let branch_kind_matches = catalog["validations_catalog"]["branch_kind_matches"]["rule"]
        .as_str()
        .unwrap();
    for (kind, prefix) in kinds.iter().zip(canonical_prefixes().iter()) {
        assert!(
            branch_kind_matches.contains(&format!("{kind} -> {prefix}/*")),
            "branch_kind_matches rule must enumerate the {kind} -> {prefix}/* mapping"
        );
    }
}

#[test]
fn markdown_kind_flag_surfaces_list_the_seven_kind_set() {
    let spec = include_str!("../../docs/specs/forge-cli-spec-v1.md");
    let expected = format!("--kind {}", canonical_kinds().join("|"));
    assert!(
        spec.contains(&expected),
        "forge-cli-spec-v1.md `--kind` surfaces must list the full seven-kind set: {expected}"
    );
}
