"use strict";
const { execFileSync } = require("node:child_process");
const { checkoutGit } = require("./devlog-fold-pr.cjs");
const { oid } = require("./devlog-fold-release.cjs");

const READ = `query FoldAdmission($owner: String!, $repo: String!, $number: Int!) {
  repository(owner: $owner, name: $repo) {
    main: ref(qualifiedName: "refs/heads/main") { target { oid } }
    freezes: issues(first: 1, states: OPEN, labels: ["merge-freeze"]) { totalCount }
    pullRequest(number: $number) {
      id state url isDraft isCrossRepository isInMergeQueue reviewDecision
      author { __typename login } baseRefName baseRefOid headRefName headRefOid
      labels(first: 100) { pageInfo { hasNextPage } nodes { name } }
      mergeCommit { oid }
      mergeQueueEntry { id state pullRequest { id headRefOid } }
      lastEnqueue: timelineItems(last: 1, itemTypes: [ADDED_TO_MERGE_QUEUE_EVENT]) {
        nodes { ... on AddedToMergeQueueEvent { id actor { login } } }
      }
      queueHistory: timelineItems(last: 1, itemTypes: [ADDED_TO_MERGE_QUEUE_EVENT, REMOVED_FROM_MERGE_QUEUE_EVENT]) {
        nodes {
          __typename
          ... on AddedToMergeQueueEvent { id createdAt actor { login } }
          ... on RemovedFromMergeQueueEvent { id reason }
        }
      }
    }
  }
}`;

const candidateFields = new Set(READ.match(/[A-Za-z_]\w*/g));

function candidateReadError(error) {
  const denied = Array.isArray(error?.errors) && error.errors.find(row => row?.type === "FORBIDDEN");
  if (!denied) return new Error("GitHub GraphQL FoldAdmission failed");
  // Only render identifiers from the fixed query. The Octokit error can carry
  // partial results and request headers, so do not attach it as a cause.
  const { path } = denied;
  const known = Array.isArray(path) && path[0] === "repository" && path.length <= 8 &&
    path.every(part => candidateFields.has(part) || (Number.isSafeInteger(part) && part >= 0));
  const suffix = known ? " at " + path.slice(1).reduce((field, part) =>
    field + (typeof part === "number" ? "[]" : "." + part), "repository") : " (field path unavailable)";
  return new Error("GitHub GraphQL FoldAdmission: FORBIDDEN" + suffix);
}

const ENQUEUE = `mutation FoldEnqueue($id: ID!, $head: GitObjectID!) {
  enqueuePullRequest(input: {pullRequestId: $id, expectedHeadOid: $head}) {
    mergeQueueEntry { id state pullRequest { id headRefOid } }
  }
}`;

const requiredMinimum = ["test", "test_containment", "cargo-deny (advisories + bans)"];

function readHoldLabels(git, base) {
  const config = git("show", `${base}:.forge-cli.toml`);
  // Resolve the same repository policy as forge-cli, from the admitted base.
  const labels = JSON.parse(execFileSync("python3", ["-c", `
import json, sys, tomllib
print(json.dumps(tomllib.loads(sys.stdin.read()).get("merge", {}).get("hold_labels", [])))
`], { input: config, encoding: "utf8" }));
  if (!Array.isArray(labels) || labels.some(label => typeof label !== "string" ||
      !label.trim() || Buffer.byteLength(label.trim()) > 128 || /[\p{Cc}]/u.test(label)))
    throw new Error("Invalid base hold-label policy");
  return new Set(["state::do-not-merge", "do-not-merge", "control::hold", ...labels.map(label => label.trim())]);
}

async function readCandidate({ github, repo, number, id, receipt, app, holdLabels, beforeEnqueue }) {
  const { repository } = await github.graphql(READ, { ...repo, number })
    .catch(error => { throw candidateReadError(error); });
  const pr = repository?.pullRequest;
  if (pr?.id !== id || pr.isCrossRepository !== false ||
      pr.author?.__typename !== "Bot" || pr.author.login !== app ||
      pr.headRefName !== receipt.branch || pr.headRefOid !== receipt.head ||
      pr.baseRefName !== "main" || pr.baseRefOid !== receipt.base)
    throw new Error("PR identity, recorded base or exact head changed");
  if (pr.state === "MERGED") return pr;
  if (pr.state !== "OPEN" || pr.isDraft !== false ||
      pr.reviewDecision === "CHANGES_REQUESTED" || pr.reviewDecision === "REVIEW_REQUIRED")
    throw new Error("PR is closed, draft or has a blocking review decision");
  if (repository.freezes.totalCount !== 0 || pr.labels.pageInfo.hasNextPage !== false ||
      pr.labels.nodes.some(({ name }) => holdLabels.has(name)))
    throw new Error("PR hold, repository freeze or incomplete hold evidence");
  if (beforeEnqueue && !pr.isInMergeQueue && repository.main?.target?.oid !== receipt.base)
    throw new Error("main moved; the recorded fold base is stale");
  return pr;
}

async function readProtectedSquashRules(github, repo) {
  // The branch rules endpoint exposes effective queue policy with Metadata read.
  const rules = await github.paginate("GET /repos/{owner}/{repo}/rules/branches/{branch}",
    { ...repo, branch: "main", per_page: 100 });
  const queues = rules.filter(rule => rule.type === "merge_queue");
  if (!queues.length || queues.some(rule => rule.parameters?.merge_method !== "SQUASH"))
    throw new Error("Protected squash merge queue is unavailable");
  return rules;
}

async function currentRequiredChecks({ github, repo, number, receipt }) {
  const rules = await readProtectedSquashRules(github, repo);
  const checks = rules.filter(rule => rule.type === "required_status_checks")
    .flatMap(rule => rule.parameters.required_status_checks);
  // GitHub Actions is the source of this repository's CI. Any other required
  // integration needs an explicit admission contract before automation.
  if (!requiredMinimum.every(name => checks.some(check => check.context === name)) ||
      checks.some(check => check.integration_id != null && check.integration_id !== 15368))
    throw new Error("Current required CI policy differs");

  const runs = await github.paginate(github.rest.actions.listWorkflowRuns,
    { ...repo, workflow_id: "ci.yml", event: "pull_request", head_sha: receipt.head, per_page: 100 });
  const run = runs.filter(run => run.pull_requests.some(pr => pr.number === number))
    .sort((a, b) => b.id - a.id)[0];
  if (!run) return null;
  const fullName = `${repo.owner}/${repo.repo}`;
  const eventPR = run.pull_requests.find(pr => pr.number === number);
  if (run.event !== "pull_request" || run.path !== ".github/workflows/ci.yml" ||
      run.head_repository?.full_name !== fullName || run.head_sha !== receipt.head ||
      eventPR.head.sha !== receipt.head || eventPR.head.ref !== receipt.branch ||
      eventPR.base.sha !== receipt.base || eventPR.base.ref !== "main")
    throw new Error("CI provenance does not match the exact fold PR, head and base");
  if (run.status !== "completed") return null;
  if (run.conclusion !== "success") throw new Error("Required PR CI failed");
  const jobs = await github.paginate(github.rest.actions.listJobsForWorkflowRunAttempt,
    { ...repo, run_id: run.id, attempt_number: run.run_attempt, per_page: 100 });
  for (const { context } of checks) {
    const matches = jobs.filter(job => job.name === context);
    if (matches.length !== 1 || matches[0].head_sha !== receipt.head ||
        matches[0].status !== "completed" || matches[0].conclusion !== "success")
      throw new Error(`Current required check did not pass: ${context}`);
  }
  return run.html_url;
}

function matchesQueueEntry(entry, prId, head) {
  // The queue's headCommit belongs to the merge group.
  return entry?.id && entry.pullRequest?.id === prId && entry.pullRequest.headRefOid === head;
}

function readMerge({ git, receipt, pr }) {
  const merge = pr.mergeCommit?.oid;
  if (!oid(merge)) return null;
  git("fetch", "--no-tags", "origin", "refs/heads/main");
  const main = git("rev-parse", "FETCH_HEAD");
  git("merge-base", "--is-ancestor", merge, main);
  for (const ref of [merge, main]) {
    if (git("--literal-pathspecs", "ls-tree", "-r", "--name-only", ref, "--", ...receipt.removed))
      throw new Error("A captured fragment still exists in the merged result or main");
  }
  return { status: "merged", mergeSha: merge, main, capturedPaths: receipt.removed };
}

async function admitReleaseFold({ github, writer, repo, candidate, receipt, app, root, env, note,
  git = checkoutGit(root, env), pause = ms => new Promise(resolve => setTimeout(resolve, ms)),
  clock = Date.now }) {
  if (receipt.status === "empty") {
    await note("empty", { status: "empty" });
    return;
  }
  if (!Number.isSafeInteger(candidate?.number) || !candidate.node_id)
    throw new Error("Missing provider PR identity");
  const { number, node_id: id } = candidate;
  const prUrl = `https://github.com/${repo.owner}/${repo.repo}/pull/${number}`;
  await note("admission", { pr: prUrl });
  const holdLabels = readHoldLabels(git, receipt.base);
  const read = (beforeEnqueue = true) =>
    readCandidate({ github, repo, number, id, receipt, app, holdLabels, beforeEnqueue });
  const readWithQueuePolicy = async (beforeEnqueue = true) => {
    const pr = await read(beforeEnqueue);
    if (pr.state === "OPEN") await readProtectedSquashRules(github, repo);
    return pr;
  };
  let pr = await readWithQueuePolicy();
  let submitted = false;

  if (pr.state === "OPEN" && !pr.isInMergeQueue) {
    if (pr.queueHistory.nodes.length) throw new Error("Fold already left the merge queue");
    await note("required-checks");
    const deadline = clock() + 30 * 60_000;
    while (!await currentRequiredChecks({ github, repo, number, receipt })) {
      if (clock() >= deadline) throw new Error("Required PR CI did not finish within 30 minutes");
      await pause(15_000);
      pr = await readWithQueuePolicy();
      if (pr.state !== "OPEN" || pr.isInMergeQueue)
        throw new Error("PR queue state changed while waiting for required CI");
    }
    // Read fresh policy and CI a second time, then reread base, head and holds
    // immediately before asking GitHub to CAS the exact approved head.
    const ci = await currentRequiredChecks({ github, repo, number, receipt });
    if (!ci) throw new Error("Required PR CI is no longer passing");
    await note("enqueue", { ci });
    await readProtectedSquashRules(github, repo);
    pr = await read();
    if (pr.state !== "OPEN" || pr.isInMergeQueue || pr.queueHistory.nodes.length)
      throw new Error("PR queue state changed before exact-head enqueue");
    const { enqueuePullRequest } = await writer.graphql(ENQUEUE, { id: pr.id, head: receipt.head });
    const entry = enqueuePullRequest?.mergeQueueEntry;
    if (!matchesQueueEntry(entry, pr.id, receipt.head))
      throw new Error("GitHub did not return an exact-head queue receipt");
    submitted = true;
    await note("queued", { queueId: entry.id, queueState: entry.state });
  } else if (pr.isInMergeQueue) {
    const entry = pr.mergeQueueEntry;
    if (!matchesQueueEntry(entry, pr.id, receipt.head) ||
        pr.lastEnqueue.nodes[0]?.actor?.login !== app)
      throw new Error("Existing queue entry is not this App's exact head");
    await note("queued", { queueId: entry.id, queueState: entry.state });
  }

  await note("merge-receipt");
  const deadline = clock() + 95 * 60_000;
  for (;;) {
    pr = await readWithQueuePolicy(false);
    if (pr.state === "MERGED") {
      const event = pr.lastEnqueue.nodes[0];
      if ((!submitted || event) && event?.actor?.login !== app)
        throw new Error("Merged PR has no automatic App queue receipt");
      if (event?.actor?.login === app) await note("merge-receipt", { queueEventId: event.id });
      const result = readMerge({ git, receipt, pr });
      if (result) {
        await note("merged", result);
        return;
      }
    } else if (!pr.isInMergeQueue &&
        pr.queueHistory.nodes[0]?.__typename === "RemovedFromMergeQueueEvent" &&
        pr.queueHistory.nodes[0].reason !== "merged") {
      throw new Error("GitHub removed the fold from the protected merge queue");
    }
    if (clock() >= deadline) throw new Error("Protected queue did not yield a merge receipt within 95 minutes");
    await pause(15_000);
  }
}

module.exports = { admitReleaseFold };
