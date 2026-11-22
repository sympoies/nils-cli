"use strict";
const fs = require("node:fs");
const path = require("node:path");
const { execFileSync } = require("node:child_process");

const SUBJECT = "chore(devlog): fold accepted fragments";
const dateBranch = date => `chore/devlog-fold-${date}`;

function checkoutGit(root, env) {
  return (...args) => execFileSync("git", args, { cwd: root, env, encoding: "utf8" }).trim();
}

function prepareFold({ root, env, branch, cutoff, script = "scripts/ci/fold-devlog.sh" }) {
  const git = checkoutGit(root, env);
  if (git("status", "--porcelain=v1", "--untracked-files=all"))
    throw new Error("Fold preparation requires a clean checkout");
  const base = git("rev-parse", "HEAD");
  git("switch", "-c", branch);
  // This is the sole writer of month files and the index. No local commit/push.
  execFileSync("bash", [script, ...(cutoff ? ["--before", cutoff] : [])],
    { cwd: root, env, stdio: "pipe" });
  // Stage new month files too; an unstaged diff omits untracked files.
  git("add", "--", "docs/devlog");
  const fields = git("diff", "--cached", "--name-status", "--no-renames", "--no-ext-diff", "-z", "HEAD").split("\0");
  const changes = [];
  for (let i = 0; i + 1 < fields.length; i += 2) {
    const [status, file] = [fields[i], fields[i + 1]];
    const retired = status === "D" && /^docs\/devlog\/pending\/[^/]+[.]md$/.test(file);
    const month = /^[AM]$/.test(status) && /^docs\/devlog\/[0-9]{4}-[0-9]{2}[.]md$/.test(file);
    const index = status === "M" && file === "docs/devlog/README.md";
    if (!retired && !month && !index) throw new Error(`Unexpected fold change: ${status} ${file}`);
    changes.push({ status, file });
  }
  if (git("diff", "--name-only", "HEAD", "--", ".", ":(exclude)docs/devlog"))
    throw new Error("Fold modified files outside docs/devlog");
  const removed = changes.filter(({ status }) => status === "D").map(({ file }) => file);
  return { base, changes, removed, tree: git("write-tree") };
}

async function getRef(github, repo, ref) {
  try {
    return (await github.rest.git.getRef({ ...repo, ref })).data.object.sha;
  } catch (error) {
    if (error.status !== 404) throw error;
    return null;
  }
}

async function requireMain(github, repo, base) {
  if (await getRef(github, repo, "heads/main") !== base)
    throw new Error("main moved; the recorded fold base is stale");
}

async function findFoldPR(github, repo, branch) {
  const pulls = await github.paginate(github.rest.pulls.list, {
    ...repo, state: "all", head: `${repo.owner}:${branch}`, per_page: 100,
  });
  // Include closed and merged PRs so a completed fold keeps its history.
  const matching = pulls.filter(pr => pr.head?.repo?.full_name === `${repo.owner}/${repo.repo}`);
  if (matching.length > 1) throw new Error("Multiple pull requests for the canonical fold branch");
  return matching[0];
}

async function signFold({ github, repo, root, plan, message = SUBJECT }) {
  const baseCommit = (await github.rest.git.getCommit({ ...repo, commit_sha: plan.base })).data;
  const tree = (await github.rest.git.createTree({ ...repo, base_tree: baseCommit.tree.sha,
    tree: plan.changes.map(({ status, file }) => ({ path: file, mode: "100644", type: "blob",
      ...(status === "D" ? { sha: null } : { content: fs.readFileSync(path.join(root, file), "utf8") }),
    })),
  })).data.sha;
  if (tree !== plan.tree) throw new Error("API tree differs from the complete prepared fold tree");
  // GitHub supplies the App author and the signed web-flow committer.
  const commit = (await github.rest.git.createCommit({ ...repo, message,
    tree, parents: [plan.base] })).data;
  if (commit.verification?.verified !== true)
    throw new Error("Bot API commit is not verified; refusing to publish the fold");
  return commit.sha;
}

async function openFoldPR(github, repo, branch, cutoff) {
  const dates = cutoff ? `before ${cutoff}` : "before today";
  const body = `## Summary

Fold accepted devlog fragments dated ${dates} in UTC using \`scripts/ci/fold-devlog.sh\`.
The branch starts from current main. Later fragments remain pending.

## Test changes

## Test plan

The generated maintenance PR moves devlog fragments and regenerates the index.

- \`devlog check --base HEAD\` passed before and after the fold.
- Normal required checks and protected merge-queue checks must pass before merge.${cutoff ? "" : "\n- The steward owns designated review and queue delivery."}
`;
  return (await github.rest.pulls.create({ ...repo, head: branch, base: "main",
    title: SUBJECT, body })).data;
}

async function runFoldPR({ github, context, root = process.cwd(), dryRun = false,
  now = new Date(), env = process.env }) {
  const repo = context.repo;
  const branch = dateBranch(now.toISOString().slice(0, 10));
  const existing = await findFoldPR(github, repo, branch);
  if (existing) return { status: "existing", branch, url: existing.html_url };
  const base = checkoutGit(root, env)("rev-parse", "HEAD");
  await requireMain(github, repo, base);
  if (await getRef(github, repo, `heads/${branch}`))
    throw new Error(`Date branch already exists without a PR: ${branch}; maintainer reconciliation required`);

  const plan = prepareFold({ root, env, branch });
  if (!plan.removed.length) return { status: "empty", branch };
  const files = plan.changes.map(change => change.file);
  if (dryRun) return { status: "dry-run", branch, files };

  await requireMain(github, repo, base);
  const sha = await signFold({ github, repo, root, plan });
  await requireMain(github, repo, base);
  await github.rest.git.createRef({ ...repo, ref: `refs/heads/${branch}`, sha });
  const pr = await openFoldPR(github, repo, branch);
  return { status: "created", branch, sha, url: pr.html_url, files };
}

module.exports = { SUBJECT, checkoutGit, findFoldPR, getRef, openFoldPR,
  prepareFold, requireMain, runFoldPR, signFold };
