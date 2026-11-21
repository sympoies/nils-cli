"use strict";
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { execFileSync } = require("node:child_process");
const { SUBJECT, branchFor, checkoutGit, findDatePR, getRef, openFoldPR,
  prepareFold, requireMain, signFold } = require("./devlog-fold-pr.cjs");

const oid = value => typeof value === "string" && /^[a-f0-9]{40}$/.test(value);

async function readPublication({ github, context, workflowRef, releaseId, git }) {
  const repo = context.repo;
  const fullName = `${repo.owner}/${repo.repo}`;
  const tag = context.ref.match(/^refs\/tags\/(v\d+\.\d+\.\d+)$/)?.[1];
  if (!tag || !["push", "workflow_dispatch"].includes(context.eventName) ||
      workflowRef !== `${fullName}/.github/workflows/release.yml@${context.ref}` ||
      !oid(context.sha) || !Number.isSafeInteger(context.runId) ||
      !/^[1-9][0-9]*$/.test(releaseId))
    throw new Error("Fold requires an authorized release workflow publication");

  // The caller's tag must resolve to its run SHA and be part of protected main.
  git("merge-base", "--is-ancestor", context.sha, "HEAD");
  const ref = (await github.rest.git.getRef({ ...repo, ref: `tags/${tag}` })).data.object;
  const target = ref.type === "tag" && oid(ref.sha)
    ? (await github.rest.git.getTag({ ...repo, tag_sha: ref.sha })).data.object : ref;
  const release = (await github.rest.repos.getReleaseByTag({ ...repo, tag })).data;
  if (target.type !== "commit" || target.sha !== context.sha || String(release.id) !== releaseId ||
      release.tag_name !== tag || release.draft !== false || release.prerelease !== false ||
      release.author?.login !== "github-actions[bot]" ||
      !/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$/.test(release.published_at))
    throw new Error("Provider release or tag does not match the successful publication");
  return { id: releaseId, tag, publishedAt: release.published_at, source: context.sha,
    runId: context.runId, repository: fullName };
}

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
      receipt.branch !== branchFor(release.publishedAt.slice(0, 10)) ||
      !oid(receipt.base) || !oid(receipt.tree) || !Array.isArray(receipt.removed) ||
      !(receipt.status === "empty" && !receipt.removed.length && receipt.head === null ||
        receipt.status === "prepared" && receipt.removed.length && oid(receipt.head)))
    throw new Error("Invalid trusted fold receipt");
}

async function prepareReleaseFold({ github, writer, repo, release, root, env, note }) {
  const cutoff = release.publishedAt.slice(0, 10);
  const branch = branchFor(cutoff);
  const base = checkoutGit(root, env)("rev-parse", "HEAD");
  await note("prepare", { branch, base });
  const existing = await findDatePR(github, repo, branch);
  if (existing) {
    await note("prepare", { pr: existing.html_url });
    throw new Error("Canonical date PR exists without this run's receipt; inspect the original run");
  }
  if (await getRef(github, repo, `heads/${branch}`))
    throw new Error("Canonical date ref already exists without this run's receipt");
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
  git("merge-base", "--is-ancestor", receipt.release.source, receipt.base);
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
      head.committer?.login !== "web-flow" || head.commit.verification?.verified !== true ||
      head.commit.verification.reason !== "valid" || head.commit.message !== foldMessage(receipt.release) ||
      head.commit.tree.sha !== receipt.tree)
    throw new Error("Head is not the exact verified App commit for this release and base");
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
  if (receipt.status === "empty") return null;
  await verifyHead(github, repo, receipt, app);
  await note("publish");
  const existing = await findDatePR(github, repo, branch);
  if (existing) return existing;
  await requireMain(github, repo, base);
  const branchHead = await getRef(github, repo, `heads/${branch}`);
  if (branchHead === null)
    await writer.rest.git.createRef({ ...repo, ref: `refs/heads/${branch}`, sha: head });
  else if (branchHead !== head)
    throw new Error("Canonical date branch has moved from the trusted head");
  await requireMain(github, repo, base);
  return openFoldPR(writer, repo, branch, release.publishedAt.slice(0, 10));
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
