#!/usr/bin/env bash
set -euo pipefail

repo_root="$(git rev-parse --show-toplevel 2>/dev/null || true)"
if [[ -z "$repo_root" || ! -d "$repo_root" ]]; then
  echo "error: must run inside the nils-cli git work tree" >&2
  exit 2
fi
cd "$repo_root"

workflow=.github/workflows/dependabot-third-party-apply.yml
# shellcheck disable=SC2016 # Match the literal workflow source, without expansion.
expected_guard='if [[ "${author}" != "dependabot[bot]" && "${author}" != "app/dependabot" ]]; then'
guard_count="$(rg -F -c -- "$expected_guard" "$workflow" || true)"
guard_count="${guard_count:-0}"

if [[ "$guard_count" != "2" ]]; then
  echo "FAIL: expected both privileged Dependabot paths to accept bot and app author identities" >&2
  echo "  found $guard_count matching guards in $workflow; expected 2" >&2
  exit 1
fi

# Run the actual privileged merge step. Both command boundaries are hermetic:
# gh supplies provider projections and forge supplies canonical gate receipts.
# No credentials, network, queue, or real provider mutation are available.
python3 - "$workflow" <<'PY'
import copy
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

workflow = Path(sys.argv[1]).read_text()
merge_job = workflow.split("\n  merge:\n", 1)[1]
assert workflow.count("github.event.workflow_run.head_repository.full_name == github.repository") == 2
assert "startsWith(github.event.workflow_run.head_branch, 'dependabot/cargo/')" in workflow
assert "startsWith(github.event.workflow_run.head_branch, 'dependabot/cargo/cargo-minor-patch-')" in merge_job
assert "github.event.workflow_run.event == 'pull_request'" in merge_job
assert "github.event.workflow_run.conclusion == 'success'" in merge_job
assert "ref: ${{ github.workflow_sha }}" in merge_job
assert "repository: ${{ github.repository }}" in merge_job
assert "persist-credentials: false" in merge_job
assert "ref: ${{ github.event.workflow_run.head_sha }}" not in workflow
assert "cargo build --locked -p nils-forge-cli" in merge_job
assert "BOT_APP_SLUG: ${{ steps.bot_token.outputs.app-slug }}" in merge_job
assert "force: false" in workflow
step = workflow.split("      - name: Squash merge the bump\n", 1)[1]
script = step.split("        run: |\n", 1)[1]
script = "\n".join(line[10:] for line in script.splitlines()) + "\n"
head = "a" * 40
old_head = "b" * 40
repo = "example/tools"
branch = "dependabot/cargo/cargo-minor-patch-example"
approved = {
    "ok": True,
    "data": {
        "head_sha": head,
        "status": "reviewed",
        "approval": {"state": "finally-approved", "reviewed_head": head},
    },
}
normal = {
    "pr": {"number": 7, "author": {"login": "dependabot[bot]"},
           "headRefOid": head, "isCrossRepository": False},
    "commit": {"sha": head, "author": {"login": "dependabot[bot]", "type": "Bot"},
               "committer": {"login": "web-flow", "type": "User"},
               "commit": {"verification": {"verified": True}}},
    "admission": approved,
    "check_exit": 0,
    "merge_exit": 0,
    # Deliberately irrelevant to exact-head admission, including in denials.
    "reviewDecision": "APPROVED",
}

stub = r'''#!/usr/bin/env python3
import json
import os
from pathlib import Path
import sys

args = sys.argv[1:]
tool = Path(sys.argv[0]).name
fixture = json.loads(Path(os.environ["FIXTURE"]).read_text())
with Path(os.environ["CALLS"]).open("a") as log:
    log.write(json.dumps([tool] + args) + "\n")
repo, head = os.environ["REPO"], os.environ["HEAD_SHA"]
if tool == "gh" and args[:2] == ["pr", "list"]:
    assert args == ["pr", "list", "--repo", repo, "--head", os.environ["HEAD_BRANCH"],
                    "--state", "open", "--json", "number,author,headRefOid,isCrossRepository",
                    "--jq", '[.[] | select(.isCrossRepository == false)][0] // empty']
    pr = fixture["pr"]
    if pr and not pr["isCrossRepository"]:
        print(json.dumps(pr))
elif tool == "gh" and args == ["api", f"repos/{repo}/commits/{head}"]:
    print(json.dumps(fixture["commit"]))
elif tool == "gh" and args[:2] == ["pr", "merge"]:
    # Record the old unsafe admission without invoking GitHub.
    Path(os.environ["QUEUED"]).write_text("unsafe gh admission\n")
elif tool == "forge-cli" and args == ["--provider", "github", "--repo", repo,
        "--format", "json", "pr", "review-handoff", "check", "7",
        "--expected-head", head, "--approval-policy", "final-approval"]:
    print(json.dumps(fixture["admission"]))
    sys.exit(fixture["check_exit"])
elif tool == "forge-cli" and args == ["--provider", "github", "--repo", repo,
        "pr", "merge", "7", "--method", "squash", "--expected-head", head,
        "--review-convergence"]:
    if fixture["merge_exit"]:
        print("canonical merge gate refused changed evidence", file=sys.stderr)
        sys.exit(fixture["merge_exit"])
    Path(os.environ["QUEUED"]).write_text("canonical forge admission\n")
else:
    print(f"unexpected offline transport call: {tool} {args!r}", file=sys.stderr)
    sys.exit(97)
'''

cases = []


def case(name, allow=False, **changes):
    fixture = copy.deepcopy(normal)
    fixture.update(changes)
    cases.append((name, fixture, allow))


# These errors are canonical owner receipts, not a second review-state parser.
# The contract verifies delegation and refusal propagation. Semantic ledger and
# native-publication parsing remain covered by forge-cli's owning tests.
for name, code in [
    ("old-head approval only", "designated_final_approval_required"),
    ("absent current-head approval", "awaiting_designated_review"),
    ("current blocking native review", "awaiting_designated_review"),
    ("governed COMMENTED blocking validation disposition", "awaiting_designated_review"),
    ("open blocking ledger finding", "awaiting_designated_review"),
    ("unknown incomplete review evidence", "review_snapshot_incomplete"),
]:
    case(name, admission={"ok": False, "error": {"code": code}}, check_exit=65)
case("exact-head approved Dependabot bot", allow=True)
app_pr = copy.deepcopy(normal["pr"])
app_pr["author"]["login"] = "app/dependabot"
app_commit = copy.deepcopy(normal["commit"])
app_commit["author"]["login"] = "artifact-automation[bot]"
case("exact-head approved artifact app and app/dependabot author", allow=True,
     pr=app_pr, commit=app_commit)
human = copy.deepcopy(normal["commit"])
human["author"] = {"login": "integration-maintainer", "type": "User"}
case("human integration head", commit=human)
unknown = copy.deepcopy(normal["commit"])
unknown["author"] = None
case("unknown commit attribution", commit=unknown)
unknown_committer = copy.deepcopy(normal["commit"])
unknown_committer["committer"] = None
case("unknown commit signer attribution", commit=unknown_committer)
other_bot = copy.deepcopy(normal["commit"])
other_bot["author"]["login"] = "unrelated-automation[bot]"
case("untrusted automation app", commit=other_bot)
unsigned = copy.deepcopy(normal["commit"])
unsigned["commit"]["verification"]["verified"] = False
case("unverified bot attribution", commit=unsigned)
wrong_commit = copy.deepcopy(normal["commit"])
wrong_commit["sha"] = old_head
case("commit evidence is for another head", commit=wrong_commit)
moved = copy.deepcopy(normal["pr"])
moved["headRefOid"] = old_head
case("head moved after CI", pr=moved)
case("head or review changed during canonical final gates", merge_exit=65)
for name, patch in [("fork branch", {"isCrossRepository": True}),
                    ("non-Dependabot PR author", {"author": {"login": "contributor"}})]:
    pr = copy.deepcopy(normal["pr"])
    pr.update(patch)
    case(name, pr=pr)
case("no matching open PR", pr=None)

with tempfile.TemporaryDirectory(prefix="dependabot-admission-") as directory:
    root = Path(directory)
    for tool in ("gh", "forge-cli"):
        executable = root / tool
        executable.write_text(stub)
        executable.chmod(0o755)
    failures = []
    for index, (name, fixture, allow) in enumerate(cases):
        fixture_file, calls, queued = (root / f"{index}.{suffix}"
                                       for suffix in ("json", "calls", "queued"))
        fixture_file.write_text(json.dumps(fixture))
        env = {"PATH": f"{root}:{os.defpath}", "REPO": repo,
               "HEAD_BRANCH": branch, "HEAD_SHA": head,
               "BOT_APP_SLUG": "artifact-automation", "FORGE_BIN": str(root / "forge-cli"),
               "FIXTURE": str(fixture_file), "CALLS": str(calls), "QUEUED": str(queued)}
        result = subprocess.run(["bash", "-c", script], env=env, text=True,
                                stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        transport = calls.read_text() if calls.exists() else ""
        canonical = queued.exists() and queued.read_text() == "canonical forge admission\n"
        if (queued.exists() != allow or (allow and (result.returncode != 0 or not canonical))
                or (not allow and result.returncode not in (0, 65))):
            failures.append(name)
            print(f"FAIL: {name}: queued={queued.exists()}, expected={allow}, exit={result.returncode}")
            print(result.stdout + transport)
        else:
            print(f"ok: {name}")
    if failures:
        sys.exit(1)
PY

echo "PASS: dependabot-third-party-workflow-contract.test.sh"
