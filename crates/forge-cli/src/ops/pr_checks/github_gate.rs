//! GitHub gate snapshots use exact-head REST rows and the configured required
//! set, rather than inferring completeness from already registered rollups.
use super::*;
#[derive(PartialEq, Eq)]
struct Head {
    sha: String,
    base: String,
    repo: String,
}
#[derive(Clone, PartialEq, Eq)]
struct Requirement {
    name: String,
    app: Option<i64>,
}

fn read_head<R: BackendRunner>(
    runner: &R,
    ctx: &ProviderContext,
    id: &str,
) -> Result<Head, ForgeError> {
    let mut argv = vec![
        "pr".into(),
        "view".into(),
        id.into(),
        "--json".into(),
        "headRefOid,baseRefName,url".into(),
    ];
    ctx.push_repo_override(&mut argv);
    let value = runner.run(&BackendCall::new(BackendProgram::Gh, argv))?;
    let value: serde_json::Value =
        serde_json::from_str(&value.stdout).map_err(|_| invalid("PR head metadata"))?;
    let sha = json_string(&value, &["headRefOid"]).ok_or_else(|| invalid("PR head SHA"))?;
    let base = json_string(&value, &["baseRefName"]).ok_or_else(|| invalid("PR base branch"))?;
    let pr_url = json_string(&value, &["url"]).ok_or_else(|| invalid("PR repository URL"))?;
    let url = url::Url::parse(&pr_url).map_err(|_| invalid("PR repository URL"))?;
    let host = url
        .host_str()
        .ok_or_else(|| invalid("PR repository authority"))?;
    let authority = url
        .port()
        .map_or_else(|| host.to_string(), |port| format!("{host}:{port}"));
    if !matches!(url.scheme(), "https" | "http")
        || !crate::provider::authorities_equal(Provider::GitHub, &authority, &ctx.host)
    {
        return Err(invalid("PR repository authority"));
    }
    let parts: Vec<_> = url
        .path_segments()
        .ok_or_else(|| invalid("PR repository URL"))?
        .collect();
    if parts.len() != 4 || parts[2] != "pull" || parts[0].is_empty() || parts[1].is_empty() {
        return Err(invalid("PR head metadata"));
    }
    let repo = format!("{}/{}", parts[0], parts[1]);
    if ctx
        .repo
        .as_deref()
        .is_some_and(|expected| !expected.eq_ignore_ascii_case(&repo))
    {
        return Err(invalid("PR repository binding"));
    }
    Ok(Head { sha, base, repo })
}
fn invalid(surface: &str) -> ForgeError {
    ForgeError::validation(
        schema_err(),
        "checks_snapshot_incomplete",
        format!("cannot establish complete {surface} for the current head"),
        None,
    )
}
fn api<R: BackendRunner>(
    runner: &R,
    ctx: &ProviderContext,
    endpoint: String,
) -> Result<serde_json::Value, ForgeError> {
    let mut argv = vec!["api".into(), endpoint.into()];
    ctx.push_github_api_hostname(&mut argv);
    let output = runner.run(&BackendCall::new(BackendProgram::Gh, argv))?;
    serde_json::from_str(&output.stdout).map_err(|_| invalid("GitHub checks response"))
}
fn requirement(value: &serde_json::Value, app_key: &str) -> Result<Requirement, ForgeError> {
    let name = value
        .get("context")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| invalid("required check configuration"))?;
    let app = match value.get(app_key) {
        None | Some(serde_json::Value::Null) => None,
        Some(v) => Some(
            v.as_i64()
                .ok_or_else(|| invalid("required check application"))?,
        ),
    }
    .filter(|id| *id > 0);
    Ok(Requirement {
        name: name.into(),
        app,
    })
}

fn requirements<R: BackendRunner>(
    runner: &R,
    ctx: &ProviderContext,
    head: &Head,
) -> Result<Vec<Requirement>, ForgeError> {
    let (owner, name) = head
        .repo
        .split_once('/')
        .ok_or_else(|| invalid("repository"))?;
    // This metadata read works without the administration permission required
    // by the REST branch-protection endpoint.
    let query = "query ForgeCheckRequirements($owner:String!,$name:String!,$ref:String!){repository(owner:$owner,name:$name){ref(qualifiedName:$ref){branchProtectionRule{requiredStatusChecks{context app{databaseId}}}}}}";
    let mut argv: Vec<OsString> = vec![
        "api".into(),
        "graphql".into(),
        "-f".into(),
        format!("query={query}").into(),
    ];
    for (key, value) in [
        ("owner", owner),
        ("name", name),
        ("ref", &format!("refs/heads/{}", head.base)),
    ] {
        argv.extend(["-f".into(), format!("{key}={value}").into()]);
    }
    ctx.push_github_api_hostname(&mut argv);
    let output = runner.run(&BackendCall::new(BackendProgram::Gh, argv))?;
    let value: serde_json::Value =
        serde_json::from_str(&output.stdout).map_err(|_| invalid("branch protection"))?;
    if value
        .get("errors")
        .is_some_and(|v| !v.as_array().is_some_and(|a| a.is_empty()))
    {
        return Err(invalid("branch protection"));
    }
    let branch = value
        .pointer("/data/repository/ref")
        .filter(|v| v.is_object())
        .ok_or_else(|| invalid("base branch protection"))?;
    let protection = branch
        .get("branchProtectionRule")
        .ok_or_else(|| invalid("base branch protection"))?;
    let mut required = Vec::new();
    if !protection.is_null() {
        let rows = protection
            .get("requiredStatusChecks")
            .and_then(|v| v.as_array())
            .ok_or_else(|| invalid("required check configuration"))?;
        for row in rows {
            let mut item = requirement(row, "app_id")?;
            item.app = row
                .pointer("/app/databaseId")
                .filter(|v| !v.is_null())
                .map(|v| {
                    v.as_i64()
                        .ok_or_else(|| invalid("required check application"))
                })
                .transpose()?;
            required.push(item);
        }
    }
    let base = url::form_urlencoded::byte_serialize(head.base.as_bytes()).collect::<String>();
    let rules = match api(
        runner,
        ctx,
        format!("repos/{}/rules/branches/{base}?per_page=100", head.repo),
    ) {
        Ok(rules) => rules,
        Err(error) if is_free_plan_rules_limitation(&error) => {
            eprintln!(
                "note: GitHub repository rules are unavailable for this repository plan; using GraphQL branch protection requirements"
            );
            return Ok(required);
        }
        Err(error) => return Err(error),
    };
    let rules = rules
        .as_array()
        .ok_or_else(|| invalid("base branch rules"))?;
    // A full page cannot establish that there are no additional requirements.
    if rules.len() >= 100 {
        return Err(invalid("base branch rules pagination"));
    }
    for rule in rules {
        if rule.get("type").and_then(|v| v.as_str()) == Some("workflows") {
            // A workflow path is not a check context. Treating its absence as
            // an empty required set would admit an unrelated optional check.
            return Err(invalid("ruleset required workflows"));
        }
        if rule.get("type").and_then(|v| v.as_str()) == Some("required_status_checks") {
            let rows = rule
                .pointer("/parameters/required_status_checks")
                .and_then(|v| v.as_array())
                .ok_or_else(|| invalid("ruleset required checks"))?;
            for row in rows {
                let item = requirement(row, "integration_id")?;
                if !required.contains(&item) {
                    required.push(item);
                }
            }
        }
    }
    Ok(required)
}

fn is_free_plan_rules_limitation(error: &ForgeError) -> bool {
    error.kind() == "backend_error"
        && error.detail().is_some_and(|detail| {
            detail.contains("(HTTP 403)")
                && detail.contains(
                    "Upgrade to GitHub Pro or make this repository public to enable this feature.",
                )
        })
}

pub(super) fn snapshot<R: BackendRunner>(
    runner: &R,
    ctx: &ProviderContext,
    args: &PrChecksArgs,
) -> Result<PrChecksPayload, ForgeError> {
    let head = read_head(runner, ctx, &args.id)?;
    let runs = api(
        runner,
        ctx,
        format!(
            "repos/{}/commits/{}/check-runs?per_page=100",
            head.repo, head.sha
        ),
    )?;
    let statuses = api(
        runner,
        ctx,
        format!(
            "repos/{}/commits/{}/status?per_page=100",
            head.repo, head.sha
        ),
    )?;
    let runs_rows = runs
        .get("check_runs")
        .and_then(|v| v.as_array())
        .ok_or_else(|| invalid("head check runs"))?;
    let status_rows = statuses
        .get("statuses")
        .and_then(|v| v.as_array())
        .ok_or_else(|| invalid("head statuses"))?;
    let mut rows = Vec::new();
    for row in runs_rows {
        let check = parse_github_rest_check_run(row, false)?;
        let app = row.pointer("/app/id").and_then(|v| v.as_i64());
        rows.push((check, app, false));
    }
    for row in status_rows {
        rows.push((parse_github_rest_status(row, false)?, None, true));
    }
    let required = requirements(runner, ctx, &head)?;
    let mut missing_checks = Vec::new();
    for requirement in required {
        let mut found = false;
        for (check, app, is_status) in &mut rows {
            if check.name == requirement.name {
                let matches_app = requirement.app.is_none_or(|id| *app == Some(id));
                // A same-name commit status must pass too, but cannot replace
                // the check run from the configured App.
                check.required |= matches_app || *is_status;
                found |= matches_app;
            }
        }
        if !found {
            missing_checks.push(pending(&requirement.name));
        }
    }
    let mut checks: Vec<_> = rows.into_iter().map(|(check, _, _)| check).collect();
    checks.extend(missing_checks);
    if json_total_count_exceeds_len(&runs, runs_rows.len())
        || json_total_count_exceeds_len(&statuses, status_rows.len())
    {
        checks.push(pending("github-checks-pagination-truncated"));
    }
    // Never return green for a snapshot collected while the PR changed head
    // or target branch. The waiter retries; the merge gate refuses it.
    if read_head(runner, ctx, &args.id)? != head {
        checks.clear();
        checks.push(pending("github-pr-head-changed"));
    }
    Ok(aggregate(ctx, checks, args.required_only, None))
}
fn pending(name: &str) -> CheckItem {
    CheckItem {
        name: name.into(),
        state: "pending",
        url: None,
        conclusion: None,
        workflow: None,
        required: true,
        started_at: None,
        completed_at: None,
    }
}
