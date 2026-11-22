"use strict";
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { SUBJECT, checkoutGit, findFoldPR, getRef, openFoldPR,
  prepareFold, requireMain, signFold } = require("./devlog-fold-pr.cjs");

const oid = value => typeof value === "string" && /^[a-f0-9]{40}$/.test(value);
const utcTimestamp = value => typeof value === "string" && /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$/.test(value);
const verified = verification => verification?.verified === true && verification.reason === "valid";

function isAncestor(git, ancestor, descendant) {
  try {
    git("merge-base", "--is-ancestor", ancestor, descendant);
    return true;
  } catch (error) {
    if (error.status !== 1) throw error;
    return false;
  }
}

async function governedMainAnchor({ github, repo, fullName, git, tag, ref, annotated, source, publishedAt }) {
  if (annotated?.sha !== ref.sha || annotated?.tag !== tag || !verified(annotated?.verification))
    throw new Error("Governed release source requires the exact verified annotated tag");
  const version = tag.slice(1);
  const subject = `chore(release): bump cli versions to ${version}`;
  const { data: head } = await github.rest.repos.getCommit({ ...repo, ref: source, per_page: 1 });
  const message = head?.commit?.message;
  if (head?.sha !== source || head.parents?.length !== 1 || !oid(head.parents[0]?.sha) ||
      typeof message !== "string" || message.split("\n", 1)[0] !== subject || !verified(head.commit.verification))
    throw new Error("Governed release source is not the exact verified release bump commit");

  // Query the immutable head even after GitHub has deleted its release branch.
  const associations = await github.rest.repos.listPullRequestsAssociatedWithCommit({
    ...repo, commit_sha: source, per_page: 2,
  });
  const candidate = associations.data?.[0];
  if (!Array.isArray(associations.data) || associations.data.length !== 1 ||
      /rel="next"/.test(associations.headers?.link ?? "") || candidate?.head?.sha !== source ||
      !Number.isSafeInteger(candidate.number) || candidate.number <= 0 ||
      typeof candidate.node_id !== "string" || !candidate.node_id)
    throw new Error("Governed release source requires an unambiguous associated release PR");
  const { data: pr } = await github.rest.pulls.get({ ...repo, pull_number: candidate.number });
  if (pr?.number !== candidate.number || pr.node_id !== candidate.node_id ||
      pr.state !== "closed" || pr.merged !== true || !utcTimestamp(pr.merged_at) ||
      pr.merged_at !== candidate.merged_at || pr.merged_at > publishedAt ||
      pr.head?.sha !== source || pr.head.ref !== `chore/release-${version.replaceAll(".", "-")}` ||
      pr.head.repo?.full_name !== fullName || pr.base?.repo?.full_name !== fullName ||
      pr.base.ref !== "main" || !oid(pr.base.sha) || !oid(pr.merge_commit_sha) ||
      pr.merge_commit_sha !== candidate.merge_commit_sha)
    throw new Error("Governed release source does not match the exact merged release PR");

  const merge = pr.merge_commit_sha;
  if (!isAncestor(git, merge, "HEAD") ||
      git("show", "-s", "--format=%P%n%s", merge) !== `${pr.base.sha}\n${subject} (#${pr.number})` ||
      !isAncestor(git, head.parents[0].sha, pr.base.sha))
    throw new Error("Governed release source does not match a reachable protected main squash");
  return merge;
}

async function readPublication({ github, context, workflowRef, releaseId, git }) {
  const repo = context.repo;
  const fullName = `${repo.owner}/${repo.repo}`;
  const tag = context.ref.match(/^refs\/tags\/(v\d+\.\d+\.\d+)$/)?.[1];
  if (!tag || !["push", "workflow_dispatch"].includes(context.eventName) ||
      workflowRef !== `${fullName}/.github/workflows/release.yml@${context.ref}` ||
      !oid(context.sha) || !Number.isSafeInteger(context.runId) ||
      !/^[1-9][0-9]*$/.test(releaseId))
    throw new Error("Fold requires an authorized release workflow publication");

  // Verify the publication before selecting its proof of protected main history.
  const ref = (await github.rest.git.getRef({ ...repo, ref: `tags/${tag}` })).data.object;
  const annotated = ref.type === "tag" && oid(ref.sha)
    ? (await github.rest.git.getTag({ ...repo, tag_sha: ref.sha })).data : null;
  const target = annotated ? annotated.object : ref;
  const release = (await github.rest.repos.getReleaseByTag({ ...repo, tag })).data;
  if (target?.type !== "commit" || target.sha !== context.sha || String(release.id) !== releaseId ||
      release.tag_name !== tag || release.draft !== false || release.prerelease !== false ||
      release.author?.login !== "github-actions[bot]" || !utcTimestamp(release.published_at))
    throw new Error("Provider release or tag does not match the successful publication");
  const mainAnchor = isAncestor(git, context.sha, "HEAD") ? context.sha
    : await governedMainAnchor({ github, repo, fullName, git, tag, ref, annotated,
      source: context.sha, publishedAt: release.published_at });
  return { id: releaseId, tag, publishedAt: release.published_at, source: context.sha,
    mainAnchor, runId: context.runId, repository: fullName };
}

const releaseBranch = release => `chore/devlog-fold-release-${release.id}`;
const receiptName = release => `devlog-fold-preparation-${release.id}`;

async function findReleasePreparation(github, repo, release) {
  const runs = await github.paginate(github.rest.actions.listWorkflowRuns,
    { ...repo, workflow_id: "release.yml", head_sha: release.source, per_page: 100 });
  const trusted = runs.filter(run => run.path === ".github/workflows/release.yml" &&
    ["push", "workflow_dispatch"].includes(run.event) && run.head_branch === release.tag &&
    run.head_sha === release.source && run.head_repository?.full_name === release.repository &&
    Number.isSafeInteger(run.id));
  const runIds = new Set([release.runId, ...trusted.map(run => run.id)]);
  const byRun = await Promise.all([...runIds].map(async runId => {
    const artifacts = await github.paginate(github.rest.actions.listWorkflowRunArtifacts,
      { ...repo, run_id: runId, name: receiptName(release), per_page: 100 });
    return artifacts.map(artifact => ({ ...artifact, runId }));
  }));
  const receipts = byRun.flat();
  if (receipts.length > 1 || receipts.some(artifact => artifact.expired || !Number.isSafeInteger(artifact.id)))
    throw new Error("Trusted preparation receipt is ambiguous or expired");
  return receipts[0];
}

function foldMessage(release) {
  return `${SUBJECT}\n\nRelease: ${release.tag} (${release.id})\nRelease run: ${release.runId}`;
}

function validateReceipt(receipt, release) {
  if (!receipt || Object.keys(release).some(key => release[key] !== receipt.release?.[key]) ||
      receipt.branch !== releaseBranch(release) ||
      !oid(receipt.base) || !oid(receipt.tree) || !Array.isArray(receipt.removed) ||
      !(receipt.status === "empty" && !receipt.removed.length && receipt.head === null ||
        receipt.status === "prepared" && receipt.removed.length && oid(receipt.head)))
    throw new Error("Invalid trusted fold receipt");
}

async function requireUnusedReleaseBranch(github, repo, branch, note, stage) {
  const existing = await findFoldPR(github, repo, branch);
  if (existing) {
    await note(stage, { pr: existing.html_url });
    throw new Error("Canonical release PR conflicts with this preparation; inspect the original run");
  }
  if (await getRef(github, repo, `heads/${branch}`))
    throw new Error("Canonical release ref conflicts with this preparation");
}

async function prepareReleaseFold({ github, writer, repo, release, root, env, note }) {
  const cutoff = release.publishedAt.slice(0, 10);
  const branch = releaseBranch(release);
  const base = checkoutGit(root, env)("rev-parse", "HEAD");
  await note("prepare", { branch, base });
  await requireUnusedReleaseBranch(github, repo, branch, note, "prepare");
  await requireMain(github, repo, base);
  const plan = prepareFold({ root, env, branch, cutoff });
  await requireMain(github, repo, base);
  const head = plan.removed.length
    ? await signFold({ github: writer, repo, root, plan, message: foldMessage(release) }) : null;
  return { release, branch, base, head, tree: plan.tree, removed: plan.removed,
    status: head ? "prepared" : "empty" };
}

function reproduceFold({ root, env, receipt }) {
  const git = checkoutGit(root, env);
  // Only the recorded base, reachable from protected main, is ever checked out.
  git("merge-base", "--is-ancestor", receipt.base, "HEAD");
  git("merge-base", "--is-ancestor", receipt.release.mainAnchor, receipt.base);
  const temporary = fs.mkdtempSync(path.join(os.tmpdir(), "devlog-fold-replay-"));
  try {
    git("clone", "-q", "--shared", "--no-checkout", "--single-branch", "--branch", "main",
      "--", root, temporary);
    checkoutGit(temporary, env)("switch", "--detach", receipt.base);
    return prepareFold({ root: temporary, env, branch: receipt.branch,
      cutoff: receipt.release.publishedAt.slice(0, 10),
      script: path.join(root, "scripts/ci/fold-devlog.sh") });
  } finally {
    fs.rmSync(temporary, { recursive: true, force: true });
  }
}

async function verifyHead(github, repo, receipt, app) {
  const head = (await github.rest.repos.getCommit({ ...repo, ref: receipt.head })).data;
  if (head.sha !== receipt.head || head.parents.length !== 1 || head.parents[0].sha !== receipt.base ||
      head.author?.type !== "Bot" || head.author.login !== `${app}[bot]` ||
      head.committer?.login !== "web-flow" || !verified(head.commit.verification) ||
      head.commit.message !== foldMessage(receipt.release) ||
      head.commit.tree.sha !== receipt.tree)
    throw new Error("Head is not the exact verified App commit for this release and base");
}

function verifyPR(pr, receipt, app) {
  if (!Number.isSafeInteger(pr?.number) || pr.number <= 0 || !pr.node_id ||
      pr.user?.login !== `${app}[bot]` ||
      pr.head?.repo?.full_name !== receipt.release.repository ||
      pr.base?.repo?.full_name !== receipt.release.repository ||
      pr.head.ref !== receipt.branch || pr.head.sha !== receipt.head ||
      pr.base.ref !== "main" || pr.base.sha !== receipt.base ||
      !(pr.state === "open" || (pr.state === "closed" && pr.merged_at)))
    throw new Error("PR identity, recorded base or exact head changed");
  return pr;
}

async function publishReleaseFold({ github, writer, repo, release, root, env, receipt, app, note,
  preparationRunId = release.runId }) {
  validateReceipt(receipt, { ...release, runId: preparationRunId });
  const { branch, base, head, tree, removed } = receipt;
  await note("reproduce", { branch, base, head, tree, preparationRunId, capturedPaths: removed });
  const replay = reproduceFold({ root, env, receipt });
  if (replay.base !== base || replay.tree !== tree ||
      JSON.stringify(replay.removed) !== JSON.stringify(removed))
    throw new Error("Complete fold tree differs from trusted replay of the recorded base");
  if (receipt.status === "empty") {
    await requireUnusedReleaseBranch(github, repo, branch, note, "reproduce");
    return null;
  }
  await verifyHead(github, repo, receipt, app);
  await note("publish");
  const existing = await findFoldPR(github, repo, branch);
  if (existing) {
    await note("publish", { pr: existing.html_url });
    return verifyPR(existing, receipt, app);
  }
  await requireMain(github, repo, base);
  const branchHead = await getRef(github, repo, `heads/${branch}`);
  if (branchHead === null)
    await writer.rest.git.createRef({ ...repo, ref: `refs/heads/${branch}`, sha: head });
  else if (branchHead !== head)
    throw new Error("Canonical release branch has moved from the trusted head");
  await requireMain(github, repo, base);
  const created = await openFoldPR(writer, repo, branch, release.publishedAt.slice(0, 10));
  await note("publish", { pr: created.html_url });
  return verifyPR(created, receipt, app);
}

function tracker({ context, core, file }) {
  const run = `${context.serverUrl}/${context.repo.owner}/${context.repo.repo}/actions/runs/${context.runId}`;
  let result = fs.existsSync(file) ? JSON.parse(fs.readFileSync(file, "utf8")) : { run };
  return async (stage, details) => {
    result = { ...result, ...details, stage };
    fs.writeFileSync(file, JSON.stringify(result, null, 2));
    core.info(JSON.stringify(result));
    await core.summary.addCodeBlock(JSON.stringify(result, null, 2), "json").write();
  };
}

module.exports = { findReleasePreparation, oid, prepareReleaseFold, publishReleaseFold,
  readPublication, receiptName, tracker };
