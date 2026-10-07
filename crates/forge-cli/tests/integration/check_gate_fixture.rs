//! Translate the existing rollup scenarios into explicit head/configuration
//! fixtures for gate tests. New registration tests use native REST fixtures.
pub fn adapt(body: &str) -> String {
    let shim = r#"
if [ "$1" = "--rollup-gate-fixture" ]; then
  shift
else
  case "$1 $2" in
    "pr view")
      case " $* " in
        *" headRefOid,baseRefName,url "*)
          fixture_repo=example/project
          previous=
          for arg in "$@"; do
            if [ "$previous" = "--repo" ] || [ "$previous" = "-R" ]; then fixture_repo="$arg"; fi
            previous="$arg"
          done
          printf '{"headRefOid":"fixture-head","baseRefName":"main","url":"https://github.com/%s/pull/42"}\n' "$fixture_repo"
          exit 0;;
      esac;;
    "api graphql")
      case " $* " in
        *"ForgeCheckRequirements"*)
          python3 -c 'import json,sys; rows=json.load(open(sys.argv[1])); print(json.dumps({"data":{"repository":{"ref":{"branchProtectionRule":{"requiredStatusChecks":[{"context":r["name"],"app":None} for r in rows]}}}}}))' "$0.required"
          exit 0;;
      esac;;
    "api repos/"*)
      case "$2" in
        */rules/branches/*) echo '[]'; exit 0;;
        */commits/*/check-runs\?*)
          "$0" --rollup-gate-fixture pr checks 1 --json name,state,bucket,workflow,link,startedAt,completedAt,description > "$0.all" || test -s "$0.all" || printf '[]' > "$0.all"
          "$0" --rollup-gate-fixture pr checks 1 --required --json name,state,bucket,workflow,link,startedAt,completedAt,description > "$0.required" || test -s "$0.required" || printf '[]' > "$0.required"
          python3 -c 'import json,sys; rows=json.load(open(sys.argv[1])); req=json.load(open(sys.argv[2])); names={r["name"] for r in rows}; rows += [r for r in req if r["name"] not in names]; conv={"pass":"success","fail":"failure","cancel":"cancelled","skipping":"skipped"}; runs=[{"name":r["name"],"status":"in_progress" if r.get("bucket")=="pending" else "completed","conclusion":None if r.get("bucket")=="pending" else conv.get(r.get("bucket"), r.get("conclusion", "success")),"details_url":r.get("link"),"workflow":r.get("workflow")} for r in rows]; print(json.dumps({"total_count":len(runs),"check_runs":runs}))' "$0.all" "$0.required"
          exit 0;;
        */commits/*/status\?*) echo '{"total_count":0,"statuses":[]}'; exit 0;;
      esac;;
  esac
fi
"#;
    let (first, rest) = body.split_once('\n').expect("shell fixture shebang");
    format!("{first}\n{shim}\n{rest}")
}
