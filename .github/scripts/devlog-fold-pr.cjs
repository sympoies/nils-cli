"use strict";
const fs = require("node:fs");
const path = require("node:path");
const { execFileSync } = require("node:child_process");

const SUBJECT = "chore(devlog): fold accepted fragments";

async function runFoldPR({ github, context, root = process.cwd(), dryRun = false,
  now = new Date(), env = process.env }) {
  const repo = context.repo;
  const fullName = `${repo.owner}/${repo.repo}`;
  const branch = `chore/devlog-fold-${now.toISOString().slice(0, 10)}`;
  const pulls = await github.paginate(github.rest.pulls.list, {
    ...repo, state: "all", head: `${repo.owner}:${branch}`, per_page: 100,
  });
  // Include closed/merged PRs: a retry must preserve the date's review decision.
  const existing = pulls.find(pr => pr.head?.repo?.full_name === fullName);
  if (existing) return { status: "existing", branch, url: existing.html_url };

  const git = (...args) => execFileSync("git", args, { cwd: root, env, encoding: "utf8" }).trim();
  if (git("status", "--porcelain=v1", "--untracked-files=all"))
    throw new Error("Fold preparation requires a clean checkout");
  const base = git("rev-parse", "HEAD");
  const checkMain = async () => {
    const current = (await github.rest.git.getRef({ ...repo, ref: "heads/main" })).data.object.sha;
    if (current !== base) throw new Error("main moved; rerun from current main");
  };
  await checkMain();
  try {
    await github.rest.git.getRef({ ...repo, ref: `heads/${branch}` });
    throw new Error(`Date branch already exists without a PR: ${branch}; maintainer reconciliation required`);
  } catch (error) { if (error.status !== 404) throw error; }

  git("switch", "-c", branch);
  // This is the sole writer of month files and the index. No local commit/push.
  execFileSync("bash", ["scripts/ci/fold-devlog.sh"], { cwd: root, env, stdio: "pipe" });
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
  // An index-only refresh is not a fragment fold.
  if (!changes.some(change => change.status === "D")) return { status: "empty", branch };
  const files = changes.map(change => change.file);
  if (dryRun) return { status: "dry-run", branch, files };

  await checkMain();
  const baseCommit = (await github.rest.git.getCommit({ ...repo, commit_sha: base })).data;
  const tree = (await github.rest.git.createTree({ ...repo, base_tree: baseCommit.tree.sha,
    tree: changes.map(({ status, file }) => ({ path: file, mode: "100644", type: "blob",
      ...(status === "D" ? { sha: null } : { content: fs.readFileSync(path.join(root, file), "utf8") }),
    })),
  })).data.sha;
  // GitHub supplies the bot App author and signed web-flow committer, matching
  // the repository's other bot PRs. Do not supply a local author/signing key.
  const commit = (await github.rest.git.createCommit({ ...repo, message: SUBJECT,
    tree, parents: [base] })).data;
  if (commit.verification?.verified !== true)
    throw new Error("Bot API commit is not verified; refusing to publish the fold");
  await checkMain();
  await github.rest.git.createRef({ ...repo, ref: `refs/heads/${branch}`, sha: commit.sha });
  const body = `## Summary

Fold accepted devlog fragments dated before today in UTC using \`scripts/ci/fold-devlog.sh\`.
The branch starts from current main; today's and future fragments remain pending.

## Test changes

No tests change in this generated maintenance PR. Existing devlog checks validate
and replay the exact fragment transfer and generated index.

## Test plan

- \`devlog check --base HEAD\` passed before and after the fold.
- Normal required checks, review and the merge queue must pass before merge.
`;
  const pr = (await github.rest.pulls.create({ ...repo, head: branch, base: "main",
    title: SUBJECT, body })).data;
  return { status: "created", branch, sha: commit.sha, url: pr.html_url, files };
}

module.exports = { runFoldPR };
