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

function replayReleaseTree(git, parent, source) {
  try {
    return git("merge-tree", "--write-tree", parent, source);
  } catch (error) {
    if (error.status !== 1) throw error;
    throw new Error("Governed release bump conflicts with the squash parent");
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
  const sourceParent = head.parents[0].sha;
  if (!isAncestor(git, merge, "HEAD"))
    throw new Error("Governed release source does not match a reachable protected main squash");
  const [parent, squashSubject, squashTree] = git("show", "-s", "--format=%P%n%s%n%T", merge).split("\n");
  // An earlier queue entry can advance main beyond the base recorded on the PR.
  if (!oid(parent) || squashSubject !== `${subject} (#${pr.number})` ||
      !isAncestor(git, sourceParent, pr.base.sha) || !isAncestor(git, pr.base.sha, parent))
    throw new Error("Governed release source does not match a reachable protected main squash");
  const { data: landing } = await github.rest.repos.getCommit({ ...repo, ref: merge, per_page: 1 });
  if (landing?.sha !== merge || !verified(landing.commit?.verification))
    throw new Error("Governed release squash is not the exact verified landing commit");

  // The signed bump's sole parent is the merge base. Replaying its change
  // preserves earlier queue edits, including independent edits to the same file.
  const replayedTree = replayReleaseTree(git, parent, source);
  if (replayedTree !== squashTree || replayedTree === git("rev-parse", `${parent}^{tree}`))
    throw new Error("Governed release squash does not preserve the signed release delta");
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

class FoldBaseMoved extends Error {
  constructor() {
    super("main moved; the recorded fold base is stale");
  }
}

async function requireFoldBase(github, repo, base) {
  if (await getRef(github, repo, "heads/main") !== base) throw new FoldBaseMoved();
}

const REPLACE = `mutation ReplaceReleaseFold($repositoryId: ID!, $updates: [RefUpdate!]!) {
  updateRefs(input: {repositoryId: $repositoryId, refUpdates: $updates}) { clientMutationId }
}`;

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
  // This only signs an unreferenced commit. Publication refreshes a stale base
  // after the immutable preparation is retained.
  const head = plan.removed.length
    ? await signFold({ github: writer, repo, root, plan, message: foldMessage(release) }) : null;
  return { release, branch, base, head, tree: plan.tree, removed: plan.removed,
    status: head ? "prepared" : "empty" };
}

async function withFoldReplay({ root, env, receipt, main = "HEAD" }, use) {
  const git = checkoutGit(root, env);
  // Only the recorded base, reachable from protected main, is ever checked out.
  git("merge-base", "--is-ancestor", receipt.base, main);
  git("merge-base", "--is-ancestor", receipt.release.mainAnchor, receipt.base);
  const temporary = fs.mkdtempSync(path.join(os.tmpdir(), "devlog-fold-replay-"));
  try {
    git("clone", "-q", "--shared", "--no-checkout", "--single-branch", "--branch", "main",
      "--", root, temporary);
    checkoutGit(temporary, env)("switch", "--detach", receipt.base);
    const plan = prepareFold({ root: temporary, env, branch: receipt.branch,
      cutoff: receipt.release.publishedAt.slice(0, 10),
      script: path.join(root, "scripts/ci/fold-devlog.sh") });
    return await use(plan, temporary);
  } finally {
    fs.rmSync(temporary, { recursive: true, force: true });
  }
}

function verifyHead(head, receipt, app) {
  if (head?.sha !== receipt.head || head.parents?.length !== 1 || head.parents[0].sha !== receipt.base ||
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
      pr.base.ref !== "main" || !["open", "closed"].includes(pr.state))
    throw new Error("PR identity, recorded base or exact head changed");
  return pr;
}

async function verifyReplay(options, receipt, main) {
  const replay = await withFoldReplay({ ...options, receipt, main }, plan => plan);
  if (replay.base !== receipt.base || replay.tree !== receipt.tree ||
      JSON.stringify(replay.removed) !== JSON.stringify(receipt.removed))
    throw new Error("Complete fold tree differs from trusted replay of the recorded base");
}

async function readFoldHead(options, head) {
  const { github, repo } = options;
  return (await github.rest.repos.getCommit({ ...repo, ref: head })).data;
}

function fetchMain(git, receipt, expected) {
  if (!oid(expected)) throw new Error("Cannot resolve current protected main for fold replay");
  git("fetch", "--no-tags", "origin", "refs/heads/main");
  const main = git("rev-parse", "FETCH_HEAD");
  if (main !== expected) throw new FoldBaseMoved();
  if (!isAncestor(git, receipt.base, main))
    throw new Error("Current protected main does not descend from the recorded fold base");
  return main;
}

async function regenerate(options, receipt, base) {
  const { github, writer, repo, root, env, app } = options;
  const main = fetchMain(checkoutGit(root, env), receipt, base);
  const updated = await withFoldReplay({ ...options, main, receipt: { ...receipt, base } },
    async (plan, temporary) => {
      const { tree, removed } = plan;
      const head = removed.length ? await signFold({ github: writer, repo, root: temporary,
        plan, message: foldMessage(receipt.release) }) : null;
      return { ...receipt, base, head, tree, removed, status: head ? "prepared" : "empty" };
    });
  await verifyReplay(options, updated, main);
  if (updated.head) verifyHead(await readFoldHead(options, updated.head), updated, app);
  return updated;
}

async function replaceFoldRef({ github, writer, repo }, receipt, previous) {
  const { data } = await github.rest.repos.get(repo);
  if (typeof data?.node_id !== "string" || !data.node_id)
    throw new Error("Missing repository identity for exact-head fold replacement");
  // Only the canonical fold branch is written. Do not replace intervening edits.
  await writer.graphql(REPLACE, { repositoryId: data.node_id, updates: [{
    name: `refs/heads/${receipt.branch}`, beforeOid: previous, afterOid: receipt.head, force: true,
  }] });
}

async function refreshPublishedFold(options, receipt, candidate, checkBeforeWrite) {
  const { github, writer, repo, note } = options;
  const base = await getRef(github, repo, "heads/main");
  if (!oid(base) || base === receipt.base) throw new Error("Cannot resolve an advanced fold base");
  const updated = await regenerate(options, receipt, base);
  await note("regenerate", { base, head: updated.head, tree: updated.tree, capturedPaths: updated.removed });
  await requireFoldBase(github, repo, base);
  await checkBeforeWrite(base);
  if (updated.head) {
    await replaceFoldRef(options, updated, receipt.head);
  } else {
    // Current main already contains all eligible entries. Retire only the
    // exact, never-queued App PR that was just checked by admission.
    if (await getRef(github, repo, `heads/${receipt.branch}`) !== receipt.head)
      throw new Error("Canonical release branch has moved from the trusted head");
    await writer.rest.pulls.update({ ...repo, pull_number: candidate.number, state: "closed" });
  }
}

async function publishReleaseFold(options) {
  const { github, writer, repo, release, root, env, app, note,
    preparationRunId = release.runId } = options;
  const original = options.receipt;
  validateReceipt(original, { ...release, runId: preparationRunId });
  let receipt = original;
  const { branch, base, head, tree, removed } = receipt;
  await note("reproduce", { branch, base, head, tree, preparationRunId, capturedPaths: removed });
  await verifyReplay(options, original);
  await note("publish");
  const existing = await findFoldPR(github, repo, branch);
  const branchHead = await getRef(github, repo, `heads/${branch}`);
  const activeHead = existing?.head?.sha ?? branchHead;
  if (existing && activeHead !== branchHead && (branchHead !== null || existing.state !== "closed"))
    throw new Error("PR identity, recorded base or exact head changed");
  if (activeHead && activeHead === head) {
    verifyHead(await readFoldHead(options, head), original, app);
  } else if (activeHead) {
    if (!oid(activeHead)) throw new Error("Canonical release branch has moved from the trusted head");
    const commit = await readFoldHead(options, activeHead);
    const parent = commit.parents?.[0]?.sha;
    if (!oid(parent) || parent === base)
      throw new Error("Canonical release branch has moved from the trusted head");
    const git = checkoutGit(root, env);
    const current = await getRef(github, repo, "heads/main");
    const main = fetchMain(git, original, current);
    if (!isAncestor(git, base, parent))
      throw new Error("Replacement fold base does not descend from the recorded base");
    const replay = await withFoldReplay({ ...options, main, receipt: { ...original, base: parent } }, plan => plan);
    if (!replay.removed.length)
      throw new Error("Replacement fold head has no eligible fragments");
    receipt = { ...original, base: parent, head: activeHead, tree: replay.tree,
      removed: replay.removed, status: "prepared" };
    verifyHead(commit, receipt, app);
  }
  if (existing) {
    await note("publish", { pr: existing.html_url });
    verifyPR(existing, receipt, app);
    if (existing.state === "open" || existing.merged_at) return { candidate: existing, receipt };
    const current = await getRef(github, repo, "heads/main");
    if (oid(current) && current !== receipt.base) {
      const main = fetchMain(checkoutGit(root, env), receipt, current);
      const empty = await withFoldReplay({ ...options, main, receipt: { ...receipt, base: current } }, plan => plan);
      if (!empty.removed.length)
        return { candidate: null, receipt: { ...receipt, base: current, tree: empty.tree,
          removed: [], head: null, status: "empty" } };
    }
    throw new Error("Canonical release PR is closed without a merge");
  }
  const current = await getRef(github, repo, "heads/main");
  if (!oid(current)) throw new Error("Cannot resolve current protected main for fold publication");
  if (current !== receipt.base) {
    receipt = await regenerate(options, receipt, current);
    await note("regenerate", { base: current, head: receipt.head, tree: receipt.tree, capturedPaths: receipt.removed });
  } else if (receipt.head && !activeHead) {
    verifyHead(await readFoldHead(options, receipt.head), receipt, app);
  }
  if (!receipt.head) return { candidate: null, receipt };
  await requireFoldBase(github, repo, receipt.base);
  if (branchHead === null)
    await writer.rest.git.createRef({ ...repo, ref: `refs/heads/${branch}`, sha: receipt.head });
  else if (branchHead !== receipt.head)
    await replaceFoldRef(options, receipt, branchHead);
  await requireFoldBase(github, repo, receipt.base);
  const created = await openFoldPR(writer, repo, branch, release.publishedAt.slice(0, 10));
  await note("publish", { pr: created.html_url });
  return { candidate: verifyPR(created, receipt, app), receipt };
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

module.exports = { findReleasePreparation, FoldBaseMoved, oid, prepareReleaseFold, publishReleaseFold,
  readPublication, receiptName, refreshPublishedFold, tracker };
