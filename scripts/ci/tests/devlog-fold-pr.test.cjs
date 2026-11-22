const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const { test } = require("node:test");
const root = path.resolve(__dirname, "../../..");

test("release publication has a fold workflow with manual dry-run recovery", () => {
  const file = path.join(root, ".github/workflows/devlog-fold.yml");
  assert.ok(fs.existsSync(file), "release publication must have a devlog fold workflow");
  const workflow = fs.readFileSync(file, "utf8");
  const release = fs.readFileSync(path.join(root, ".github/workflows/release.yml"), "utf8");
  const call = release.split("\n  fold_release_devlog:\n")[1]?.split(/\n  \w+:\n/)[0];
  assert.match(call, /needs: release\n    if: .*needs.release.result == 'success'/);
  assert.match(call, /uses: \.\/\.github\/workflows\/devlog-fold\.yml/);
  assert.match(call, /release_id: \$\{\{ needs.release.outputs.release_id \}\}/);
  assert.match(release, /release_id: \$\{\{ steps.publish.outputs.id \}\}/);
  assert.doesNotMatch(workflow, /^  release:/m);
  assert.match(workflow, /workflow_call:/);
  assert.match(workflow, /workflow_dispatch:/);
  assert.match(workflow, /dry_run:/);
  assert.match(workflow, /BOT_APP_ID/);
  assert.match(workflow, /BOT_APP_PRIVATE_KEY/);
  assert.match(workflow, /persist-credentials: false/);
  assert.match(workflow, /ref: main/);
  assert.match(workflow, /if \(!dryRun && !BOT_TOKEN\)/);
  assert.match(workflow, /const client = dryRun \? github : getOctokit\(BOT_TOKEN\)/);
});

const os = require("node:os");
const { execFileSync } = require("node:child_process");
const temporary = [];
test.after(() => temporary.forEach(dir => fs.rmSync(dir, { recursive: true, force: true })));
const subject = "chore(devlog): fold accepted fragments";
// The default instant also checks conversion to the previous UTC date.
function fixture({ pending = true, entryDate = "2020-01-02", pulls = [],
  releaseId = "123", tag = "v1.2.3", head = "d".repeat(40),
  now = new Date("2030-01-02T00:30:00+02:00") } = {}) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "devlog-fold-pr-"));
  temporary.push(dir);
  const env = Object.fromEntries(Object.entries(process.env).filter(([key]) =>
    !/^(GIT_|FORGE_IDENTITY_|GITHUB_)/.test(key)));
  Object.assign(env, { GIT_CONFIG_NOSYSTEM: "1", GIT_CONFIG_GLOBAL: os.devNull,
    DEVLOG_LAYOUT: "fragments", TZ: "UTC" });
  const run = (command, args) => execFileSync(command, args, { cwd: dir, env, encoding: "utf8" }).trim();
  const git = (...args) => run("git", args);
  const write = (file, text) => {
    fs.mkdirSync(path.dirname(path.join(dir, file)), { recursive: true });
    fs.writeFileSync(path.join(dir, file), text);
  };
  git("init", "-q", "-b", "main");
  git("config", "user.name", "Fixture Author");
  git("config", "user.email", "fixture@example.invalid");
  git("config", "commit.gpgsign", "false");
  write("docs/devlog/README.md", "# Development log\n\n## Months\n\n- [2020-01](2020-01.md)\n");
  write("docs/devlog/2020-01.md", "# Development log - 2020-01\n");
  write(".forge-cli.toml", "[merge]\nhold_labels = [\"review::paused\"]\n");
  for (const file of ["scripts/ci/fold-devlog.sh", "scripts/dev-env.sh"])
    write(file, fs.readFileSync(path.join(root, file), "utf8"));
  if (pending) run("devlog", ["new", "--date", entryDate, "--slug", "accepted",
    "--title", "Accepted outcome", "--result", "An outcome.", "--why", "A reason.", "--evidence", "A check."]);
  run("devlog", ["new", "--date", "2099-01-01", "--slug", "future", "--title", "Future outcome",
    "--result", "An outcome.", "--why", "A reason.", "--evidence", "A check."]);
  git("add", ".");
  git("commit", "-qm", "Fixture baseline");
  const base = git("rev-parse", "HEAD");
  const date = now.toISOString().slice(0, 10);
  const branch = `chore/devlog-fold-${date}`;
  const state = { pulls, writes: [], verified: true, base, occupied: false, moved: false };
  const repo = { owner: "example", repo: "project" };
  const fullName = "example/project";
  const rest = {
    pulls: {
      list: async args => {
        assert.equal(args.state, "all");
        assert.ok(args.head.startsWith("example:"));
        return { data: state.pulls.filter(pr => args.head === `example:${pr.head?.ref ?? branch}`) };
      },
      create: async args => {
        state.writes.push(["pr", args]);
        const number = state.pulls.length + 1;
        const pr = { ...args, number, node_id: `PR-${number}`,
          html_url: `https://github.com/example/project/pull/${number}`,
          state: "open", user: { login: "fold-app[bot]" },
          base: { ref: args.base, sha: state.base, repo: { full_name: fullName } },
          head: { ref: args.head, sha: head, repo: { full_name: fullName } } };
        state.pulls.push(pr);
        return { data: pr };
      },
    },
    git: {
      getTag: async () => ({ data: { object: { type: "commit", sha: state.base } } }),
      getRef: async ({ ref }) => {
        if (ref === `tags/${tag}`) return { data: { object: { type: "tag", sha: "e".repeat(40) } } };
        if (ref === "heads/main") return { data: { object: { sha: state.moved ? "f".repeat(40) : state.base } } };
        assert.ok(ref.startsWith("heads/chore/devlog-fold-"));
        if (!state.occupied) throw Object.assign(new Error("not found"), { status: 404 });
        return { data: { object: { sha: state.occupied === true ? "e".repeat(40) : state.occupied } } };
      },
      getCommit: async () => ({ data: { tree: { sha: "b".repeat(40) } } }),
      createTree: async args => { state.writes.push(["tree", args]); return { data: { sha: git("write-tree") } }; },
      createCommit: async args => {
        state.writes.push(["commit", args]);
        return { data: { sha: head, verification: { verified: state.verified } } };
      },
      createRef: async args => {
        state.writes.push(["ref", args]);
        state.occupied = args.sha;
        return { data: {} };
      },
    },
    repos: {
      getReleaseByTag: async () => ({ data: { id: Number(releaseId), tag_name: tag, published_at: `${date}T00:00:00Z`,
        draft: false, prerelease: false, author: { login: "github-actions[bot]" } } }),
      getCommit: async () => {
        const signed = state.writes.find(([stage]) => stage === "commit")[1];
        return { data: { sha: head, parents: signed.parents.map(sha => ({ sha })),
          author: { type: "Bot", login: state.author ?? "fold-app[bot]" }, committer: { login: "web-flow" },
          commit: { message: signed.message, tree: { sha: signed.tree },
            verification: { verified: state.verified, reason: "valid" } } } };
      },
    },
  };
  const github = { rest, paginate: async (method, args) => (await method(args)).data };
  return { state, git, dir, branch, releaseId, tag, options: { github,
    context: { repo }, root: dir, env, now }, github, repo, fullName, head };
}
function runFold(options) {
  return require("../../../.github/scripts/devlog-fold-pr.cjs").runFoldPR(options);
}

test("dry-run executes the real fold and leaves every remote ref untouched", async () => {
  const f = fixture();
  const result = await runFold({ ...f.options, dryRun: true });
  assert.equal(result.status, "dry-run");
  assert.equal(result.branch, f.branch);
  assert.equal(result.branch, "chore/devlog-fold-2030-01-01");
  assert.deepEqual(f.state.writes, []);
  assert.equal(f.git("rev-parse", "main"), f.state.base);
  assert.equal(f.git("rev-parse", "HEAD"), f.state.base);
  assert.equal(f.git("branch", "--show-current"), f.branch);
  const month = fs.readFileSync(path.join(f.dir, "docs/devlog/2020-01.md"), "utf8");
  assert.equal(month.split("## 2020-01-02 - Accepted outcome").length - 1, 1);
  assert.equal(fs.existsSync(path.join(f.dir, "docs/devlog/pending/2020-01-02-accepted.md")), false);
  assert.equal(fs.existsSync(path.join(f.dir, "docs/devlog/pending/2099-01-01-future.md")), true);
});

test("a fold with only future fragments opens no PR or branch remotely", async () => {
  const f = fixture({ pending: false });
  assert.equal((await runFold(f.options)).status, "empty");
  assert.deepEqual(f.state.writes, []);
});

for (const state of ["open", "closed"]) test(`an existing ${state} date PR is preserved before folding`, async () => {
  const f = fixture();
  f.state.pulls.push({ state, html_url: "https://github.com/example/project/pull/1",
    head: { repo: { full_name: "example/project" } } });
  assert.equal((await runFold(f.options)).status, "existing");
  assert.equal(f.git("branch", "--show-current"), "main");
  assert.deepEqual(f.state.writes, []);
});

test("App commit transfers the exact fold diff and opens exactly one date PR", async () => {
  const f = fixture();
  // A fork branch with the same name must not suppress this repository's PR.
  f.state.pulls.push({ head: { repo: { full_name: "outside/project" } } });
  assert.equal((await runFold(f.options)).status, "created");
  const writes = Object.fromEntries(f.state.writes);
  assert.deepEqual(f.state.writes.map(([stage]) => stage), ["tree", "commit", "ref", "pr"]);
  assert.equal(writes.commit.message, subject);
  assert.deepEqual(writes.commit.parents, [f.state.base]);
  assert.equal(writes.commit.author, undefined); // GitHub supplies the App identity/signature.
  assert.equal(writes.ref.ref, `refs/heads/${f.branch}`);
  assert.equal(writes.pr.base, "main");
  assert.equal(writes.pr.head, f.branch);
  assert.match(writes.pr.body, /^## Test changes\n\n## Test plan\n\nThe generated maintenance PR moves devlog fragments/m);
  assert.equal(writes.pr.title, subject);
  const tree = writes.tree.tree;
  assert.deepEqual(tree.map(item => item.path).sort(), [
    "docs/devlog/2020-01.md", "docs/devlog/README.md", "docs/devlog/pending/2020-01-02-accepted.md",
  ]);
  assert.equal(tree.find(item => item.path.includes("pending/")).sha, null);
  for (const item of tree.filter(item => !item.path.includes("pending/")))
    assert.equal(item.content, fs.readFileSync(path.join(f.dir, item.path), "utf8"));
  assert.equal((await runFold(f.options)).status, "existing");
  assert.equal(f.state.writes.length, 4);
});

test("an unverified API commit is never published", async () => {
  const f = fixture();
  f.state.verified = false;
  await assert.rejects(runFold(f.options), /verified/);
  assert.deepEqual(f.state.writes.map(([stage]) => stage), ["tree", "commit"]);
});

test("a newly created month is included in the published tree", async () => {
  const f = fixture({ entryDate: "2020-02-02" });
  await runFold(f.options);
  const tree = Object.fromEntries(f.state.writes).tree.tree;
  const month = tree.find(item => item.path === "docs/devlog/2020-02.md");
  assert.ok(month, "new month must accompany its retired fragments");
  assert.match(month.content, /## 2020-02-02 - Accepted outcome/);
});

test("an occupied date branch is preserved for maintainer recovery", async () => {
  const f = fixture();
  f.state.occupied = true;
  await assert.rejects(runFold(f.options), /branch.*exists/i);
  assert.deepEqual(f.state.writes, []);
});

test("a moved main is rejected before creating remote objects", async () => {
  const f = fixture();
  f.state.moved = true;
  await assert.rejects(runFold(f.options), /main.*moved/i);
  assert.deepEqual(f.state.writes, []);
});

test("dirty source trees fail before running the fold or publishing", async () => {
  const f = fixture();
  fs.writeFileSync(path.join(f.dir, "untracked.txt"), "unrelated work");
  await assert.rejects(runFold(f.options), /clean/);
  assert.deepEqual(f.state.writes, []);
  assert.equal(f.git("branch", "--show-current"), "main");
});

test("PR fixture runs cannot displace queued fold publication", () => {
  const workflow = fs.readFileSync(path.join(root, ".github/workflows/devlog-fold.yml"), "utf8");
  assert.doesNotMatch(workflow, /^concurrency:/m, "PR fixtures must not share the publication lock");
  const foldJob = workflow.split("\n  fold:\n")[1];
  assert.match(foldJob, /    concurrency:\n      group: devlog-fold[^\n]*\n      cancel-in-progress: false/);
});

test("manual dry runs cannot displace queued fold publication", () => {
  const workflow = fs.readFileSync(path.join(root, ".github/workflows/devlog-fold.yml"), "utf8");
  const foldJob = workflow.split("\n  fold:\n")[1];
  const group = foldJob.match(/^      group: (.+)$/m)?.[1] ?? "";
  assert.match(group, /\$\{\{ inputs[.]dry_run && '-dry-run' \|\| '' \}\}/,
    "Manual dry runs must use a separate concurrency group");
});

const { findReleasePreparation, readPublication, prepareReleaseFold, publishReleaseFold } =
  require("../../../.github/scripts/devlog-fold-release.cjs");
const { admitReleaseFold } = require("../../../.github/scripts/devlog-fold-admission.cjs");

function withRelease(f, source = f.state.base) {
  const ref = `refs/tags/${f.tag}`;
  const args = { github: f.github, git: f.git, releaseId: f.releaseId,
    context: { ...f.options.context, ref, sha: source,
      eventName: "push", runId: 12345 },
    workflowRef: `example/project/.github/workflows/release.yml@${ref}` };
  return args;
}

test("the publication proof binds the release workflow, tag, ID and protected source", async () => {
  const f = fixture();
  const args = withRelease(f);
  const release = await readPublication(args);
  assert.equal(release.runId, args.context.runId);
  assert.equal(release.mainAnchor, args.context.sha);
  await assert.rejects(readPublication({ ...args, workflowRef: "example/project/other.yml@refs/heads/main" }),
    /authorized release workflow/);
  await assert.rejects(readPublication({ ...args, releaseId: "124" }), /does not match/);
});

test("recovery only accepts preparation artifacts from trusted runs for the same release source", async () => {
  const f = fixture();
  const release = await readPublication(withRelease(f));
  const trusted = { id: 100, event: "push", path: ".github/workflows/release.yml",
    head_sha: release.source, head_branch: release.tag, head_repository: { full_name: f.fullName } };
  const seen = [];
  f.github.rest.actions = {
    listWorkflowRuns: async ({ workflow_id, head_sha }) => {
      assert.equal(workflow_id, "release.yml");
      assert.equal(head_sha, release.source);
      return { data: [trusted, { ...trusted, id: 101, event: "pull_request" },
        { ...trusted, id: 102, head_branch: "v2.0.0" }, { ...trusted, id: 103, head_sha: f.head }] };
    },
    listWorkflowRunArtifacts: async ({ run_id, name }) => {
      assert.equal(name, "devlog-fold-preparation-123");
      seen.push(run_id);
      return { data: run_id === trusted.id ? [{ id: 10, expired: false }] : [] };
    },
  };
  assert.deepEqual(await findReleasePreparation(f.github, f.repo, release),
    { id: 10, expired: false, runId: 100 });
  assert.deepEqual(seen, [release.runId, trusted.id]);
});

async function production(f, publication = withRelease(f)) {
  const events = [];
  const release = await readPublication(publication);
  const options = { ...f.options, repo: f.repo, writer: f.github, release, app: "fold-app",
    note: async (stage, details) => events.push({ stage, ...details }) };
  const receipt = await prepareReleaseFold(options);
  return { options: { ...options, receipt }, events, receipt };
}

function governedRelease(f) {
  const version = f.tag.slice(1);
  const branch = `chore/release-${version.replaceAll(".", "-")}`;
  const releaseSubject = `chore(release): bump cli versions to ${version}`;
  const manifest = path.join(f.dir, "Cargo.toml");
  fs.writeFileSync(manifest, '[workspace.package]\nversion = "1.2.2"\n');
  f.git("add", "Cargo.toml");
  f.git("commit", "-qm", "Record the previous workspace version");
  const releaseParent = f.git("rev-parse", "HEAD");
  f.git("switch", "-qc", branch);
  fs.writeFileSync(manifest, `[workspace.package]\nversion = "${version}"\n`);
  f.git("commit", "-qam", releaseSubject);
  const source = f.git("rev-parse", "HEAD");
  f.git("tag", "-a", f.tag, "-m", f.tag);
  const tagOid = f.git("rev-parse", f.tag);

  f.git("switch", "-q", "main");
  fs.writeFileSync(path.join(f.dir, "README.md"), "Protected main update\n");
  f.git("add", "README.md");
  f.git("commit", "-qm", "Protected main advances before the squash");
  const base = f.git("rev-parse", "HEAD");
  f.git("merge", "--squash", source);
  const number = 27;
  f.git("commit", "-qm", `${releaseSubject} (#${number})`);
  const merge = f.git("rev-parse", "HEAD");
  f.git("branch", "-D", branch);
  f.git("commit", "--allow-empty", "-qm", "Protected main advances after the squash");
  f.state.base = f.git("rev-parse", "HEAD");

  const repo = { full_name: f.fullName };
  const pr = { number, node_id: "RELEASE-PR", state: "closed", merged: true,
    merged_at: "2029-12-31T23:00:00Z", merge_commit_sha: merge,
    head: { ref: branch, sha: source, repo }, base: { ref: "main", sha: base, repo } };
  const annotated = { sha: tagOid, tag: f.tag, object: { type: "commit", sha: source },
    verification: { verified: true, reason: "valid" } };
  const signed = { sha: source, parents: [{ sha: releaseParent }],
    commit: { message: releaseSubject, tree: { sha: f.git("rev-parse", `${source}^{tree}`) },
      verification: { verified: true, reason: "valid" } } };
  const provider = { associated: [structuredClone(pr)], headers: {}, pr, annotated, signed };
  const getRef = f.github.rest.git.getRef;
  f.github.rest.git.getRef = async args => args.ref === `tags/${f.tag}`
    ? { data: { object: { type: "tag", sha: tagOid } } } : getRef(args);
  f.github.rest.git.getTag = async ({ tag_sha }) => {
    assert.equal(tag_sha, tagOid);
    return { data: annotated };
  };
  const getCommit = f.github.rest.repos.getCommit;
  f.github.rest.repos.getCommit = async args => args.ref === source ? { data: signed } : getCommit(args);
  f.github.rest.repos.listPullRequestsAssociatedWithCommit = async ({ commit_sha, per_page }) => {
    assert.equal(commit_sha, source);
    assert.equal(per_page, 2);
    return { data: provider.associated, headers: provider.headers };
  };
  f.github.rest.pulls.get = async ({ pull_number }) => {
    assert.equal(pull_number, number);
    return { data: pr };
  };
  return { publication: withRelease(f, source), provider, source, merge };
}

test("a governed signed release head folds after its protected squash and release branch deletion", async () => {
  const f = fixture();
  const { publication, source, merge } = governedRelease(f);
  assert.throws(() => f.git("merge-base", "--is-ancestor", source, "HEAD"), { status: 1 });
  f.git("merge-base", "--is-ancestor", merge, "HEAD");
  const { options, receipt } = await production(f, publication);
  assert.equal(receipt.release.source, source);
  assert.equal(receipt.release.mainAnchor, merge);
  assert.equal(receipt.base, f.state.base);
  assert.deepEqual(receipt.removed, ["docs/devlog/pending/2020-01-02-accepted.md"]);
  const candidate = await publishReleaseFold(options);
  assert.equal(candidate.head.ref, "chore/devlog-fold-release-123");
  assert.equal((await publishReleaseFold(options)).number, candidate.number);
  await assert.rejects(publishReleaseFold({ ...options, receipt: { ...receipt,
    release: { ...receipt.release, mainAnchor: receipt.base },
  } }), /Invalid trusted fold receipt/);
  assert.deepEqual(f.state.writes.map(([stage]) => stage), ["tree", "commit", "ref", "pr"]);
});

for (const defect of [
  "wrong tag target", "unsigned tag", "unsigned head", "absent PR", "ambiguous PRs",
  "partial PR history", "unmerged PR", "late merge", "foreign PR", "wrong PR head",
  "unreachable squash", "unrelated main commit",
]) test(`governed release publication refuses ${defect}`, async () => {
  const f = fixture();
  const { publication, provider, merge } = governedRelease(f);
  const { pr, annotated, signed } = provider;
  const expectations = {
    "wrong tag target": /release or tag does not match/,
    "unsigned tag": /verified annotated tag/,
    "unsigned head": /verified release bump commit/,
    "absent PR": /unambiguous associated release PR/,
    "ambiguous PRs": /unambiguous associated release PR/,
    "partial PR history": /unambiguous associated release PR/,
    "unmerged PR": /exact merged release PR/,
    "late merge": /exact merged release PR/,
    "foreign PR": /exact merged release PR/,
    "wrong PR head": /exact merged release PR/,
    "unreachable squash": /reachable protected main squash/,
    "unrelated main commit": /reachable protected main squash/,
  };
  switch (defect) {
    case "wrong tag target": annotated.object.sha = f.state.base; break;
    case "unsigned tag": annotated.verification.verified = false; break;
    case "unsigned head": signed.commit.verification.verified = false; break;
    case "absent PR": provider.associated = []; break;
    case "ambiguous PRs": provider.associated.push({ ...pr, number: 28 }); break;
    case "partial PR history": provider.headers.link = '<https://api.example.invalid/page/2>; rel="next"'; break;
    case "unmerged PR": pr.state = "open"; pr.merged = false; break;
    case "late merge":
      pr.merged_at = "2030-01-01T00:00:01Z";
      provider.associated[0].merged_at = pr.merged_at;
      break;
    case "foreign PR": pr.head.repo = { full_name: "fork/project" }; break;
    case "wrong PR head": pr.head.sha = f.state.base; break;
    case "unreachable squash": {
      const subject = f.git("show", "-s", "--format=%s", merge);
      pr.merge_commit_sha = f.git("commit-tree", f.git("rev-parse", `${merge}^{tree}`),
        "-p", pr.base.sha, "-m", subject, "-m", "An unrelated commit object");
      provider.associated[0].merge_commit_sha = pr.merge_commit_sha;
      break;
    }
    case "unrelated main commit":
      pr.merge_commit_sha = f.state.base;
      provider.associated[0].merge_commit_sha = pr.merge_commit_sha;
      break;
  }
  await assert.rejects(readPublication(publication), expectations[defect]);
  assert.deepEqual(f.state.writes, []);
});

test("a release folds eligible fragments when the same-day manual PR is already merged", async () => {
  const f = fixture();
  const manual = { number: 1, state: "closed", merged_at: "2030-01-01T00:00:01Z",
    html_url: "https://github.com/example/project/pull/1",
    head: { ref: f.branch, repo: { full_name: f.fullName } } };
  f.state.pulls.push(manual);
  const { options, receipt } = await production(f);
  const candidate = await publishReleaseFold(options);
  assert.equal(candidate.number, 2);
  assert.equal(receipt.branch, "chore/devlog-fold-release-123");
  assert.deepEqual(receipt.removed, ["docs/devlog/pending/2020-01-02-accepted.md"]);
  assert.deepEqual(f.state.pulls, [manual, candidate]);
  assert.deepEqual(f.state.writes.map(([stage]) => stage), ["tree", "commit", "ref", "pr"]);
});

test("distinct same-day releases have independent PR history and replay only their own PR", async () => {
  const first = fixture();
  const second = fixture({ pulls: first.state.pulls, releaseId: "456", tag: "v1.2.4",
    head: "c".repeat(40), entryDate: "2020-01-03" });
  const a = await production(first);
  const aPR = await publishReleaseFold(a.options);
  aPR.state = "closed";
  aPR.merged_at = "2030-01-01T00:00:01Z";
  const b = await production(second);
  const bPR = await publishReleaseFold(b.options);
  assert.equal(a.receipt.release.publishedAt, b.receipt.release.publishedAt);
  assert.equal(a.receipt.branch, "chore/devlog-fold-release-123");
  assert.equal(b.receipt.branch, "chore/devlog-fold-release-456");
  assert.deepEqual(b.receipt.removed, ["docs/devlog/pending/2020-01-03-accepted.md"]);
  assert.notEqual(aPR.number, bPR.number);
  for (const [f, attempt, pr] of [[first, a, aPR], [second, b, bPR]]) {
    assert.equal((await publishReleaseFold({ ...attempt.options,
      release: { ...attempt.options.release, runId: 54321 },
      preparationRunId: attempt.receipt.release.runId,
    })).number, pr.number);
    assert.deepEqual(f.state.writes.map(([stage]) => stage), ["tree", "commit", "ref", "pr"]);
  }
});

test("replay refuses another release's receipt or a mismatched or ambiguous PR on its own branch", async () => {
  const f = fixture();
  const { options, receipt } = await production(f);
  await assert.rejects(publishReleaseFold({ ...options,
    receipt: { ...receipt, branch: "chore/devlog-fold-release-456" },
  }), /Invalid trusted fold receipt/);
  const pr = await publishReleaseFold(options);
  const writes = f.state.writes.length;
  pr.head.sha = "f".repeat(40);
  await assert.rejects(publishReleaseFold(options), /PR identity, recorded base or exact head changed/);
  pr.head.sha = receipt.head;
  f.state.pulls.push({ ...pr, number: 2 });
  await assert.rejects(publishReleaseFold(options), /Multiple pull requests/);
  assert.equal(f.state.writes.length, writes);
});

test("release preparation and replay reuse the signed head and never open a duplicate PR", async () => {
  const f = fixture();
  const { options, receipt } = await production(f);
  assert.equal(receipt.base, f.state.base);
  assert.equal(receipt.head, f.head);
  assert.equal(receipt.release.tag, "v1.2.3");
  assert.deepEqual(f.state.writes.map(([stage]) => stage), ["tree", "commit"]);
  // Reuse the immutable receipt after a PR creation failure left the exact ref.
  f.state.occupied = receipt.head;
  const pr = await publishReleaseFold(options);
  assert.equal(pr.number, 1);
  assert.equal((await publishReleaseFold({ ...options,
    release: { ...options.release, runId: 54321 }, preparationRunId: receipt.release.runId,
  })).number, 1);
  assert.deepEqual(f.state.writes.map(([stage]) => stage), ["tree", "commit", "pr"]);
  assert.match(pr.body, /^## Test changes\n\n## Test plan\n\nThe generated maintenance PR moves devlog fragments/m);
});

test("an empty release retains a no-op receipt and never creates a ref or PR on replay", async () => {
  const f = fixture({ pending: false });
  const { options, receipt, events } = await production(f);
  assert.equal(receipt.status, "empty");
  for (const runId of [receipt.release.runId, 54321]) {
    const candidate = await publishReleaseFold({ ...options,
      release: { ...options.release, runId }, preparationRunId: receipt.release.runId,
    });
    assert.equal(candidate, null);
    await admitReleaseFold({ ...options, candidate });
  }
  assert.equal(events.at(-1).status, "empty");
  assert.deepEqual(f.state.writes, []);
});

test("admission replays the full tree from the recorded base and rejects invalid App provenance", async () => {
  const f = fixture({ entryDate: "2021-01-01", now: new Date("2021-01-01T00:00:00Z") });
  // The production cutoff is strict. The same-day entry stays pending even
  // when the real writer clock is later than the release date.
  f.git("checkout", "-q", "main");
  execFileSync("devlog", ["new", "--date", "2020-01-02", "--slug", "earlier",
    "--title", "Earlier fragment", "--result", "An outcome.", "--why", "A reason.", "--evidence", "A check."],
    { cwd: f.dir, env: f.options.env });
  f.git("add", "docs/devlog");
  f.git("commit", "-qm", "Fixture baseline before release");
  f.state.base = f.git("rev-parse", "HEAD");
  const { options, receipt } = await production(f);
  assert.deepEqual(receipt.removed, ["docs/devlog/pending/2020-01-02-earlier.md"]);
  await assert.rejects(publishReleaseFold({ ...options, receipt: { ...receipt, tree: "a".repeat(40) } }),
    /Complete fold tree differs/);
  f.state.author = "unrelated[bot]";
  await assert.rejects(publishReleaseFold(options), /verified App commit/);
  assert.deepEqual(f.state.writes.map(([stage]) => stage), ["tree", "commit"]);
});

async function queueFixture({ mergeGroupHead = { oid: "e".repeat(40) } } = {}) {
  const f = fixture();
  const { options, receipt, events } = await production(f);
  const candidate = await publishReleaseFold(options);
  const checkNames = ["test", "test_containment", "cargo-deny (advisories + bans)"];
  const state = { reads: 0, runs: 0, queued: false, merged: false, mutations: [] };
  const entry = { id: "QUEUE", state: "QUEUED", headCommit: mergeGroupHead,
    pullRequest: { id: candidate.node_id, headRefOid: receipt.head } };
  const attempts = { number: 1, base: { sha: receipt.base, ref: "main" },
    head: { sha: receipt.head, ref: receipt.branch } };
  const pr = { id: candidate.node_id, state: "OPEN", isDraft: false, isCrossRepository: false,
    author: { __typename: "Bot", login: "fold-app" }, baseRefName: "main", baseRefOid: receipt.base,
    headRefName: receipt.branch, headRefOid: receipt.head,
    labels: { pageInfo: { hasNextPage: false }, nodes: [] },
    queueHistory: { nodes: [] }, lastEnqueue: { nodes: [] } };
  f.git("commit", "-qm", "Synthetic protected squash");
  const mergedSha = f.git("rev-parse", "HEAD");
  f.git("branch", "--force", "main", mergedSha);
  f.git("remote", "add", "origin", f.dir);
  f.github.graphql = async (query, vars) => {
    if (query.includes("mutation FoldEnqueue")) {
      state.mutations.push(vars);
      assert.deepEqual(vars, { id: candidate.node_id, head: receipt.head });
      state.queued = true;
      return { enqueuePullRequest: { mergeQueueEntry: entry } };
    }
    state.reads++;
    state.denyCandidateRead?.(query);
    const current = { ...pr };
    state.onRead?.(current);
    current.isInMergeQueue = state.queued && !state.merged;
    if (state.queued) current.lastEnqueue = { nodes: [{ id: "ENQUEUE", actor: { login: "fold-app" } }] };
    if (state.evicted && state.mutations.length) {
      current.isInMergeQueue = false;
      current.queueHistory = { nodes: [{ __typename: "RemovedFromMergeQueueEvent", reason: "check_failed" }] };
    }
    if (state.merged) {
      current.state = "MERGED";
      current.mergeCommit = { oid: mergedSha };
    } else if (state.queued) current.mergeQueueEntry = entry;
    return { repository: { main: { target: { oid: state.main ?? receipt.base } },
      freezes: { totalCount: state.freeze ?? 0 }, pullRequest: current } };
  };
  f.github.rest.actions = {
    listWorkflowRuns: async ({ event, head_sha }) => {
      assert.equal(event, "pull_request");
      assert.equal(head_sha, receipt.head);
      state.runs++;
      return { data: [{ id: 24, head_sha, event, path: ".github/workflows/ci.yml",
        head_repository: { full_name: f.fullName }, pull_requests: [attempts],
        status: "completed", conclusion: "success", run_attempt: 1,
        html_url: "https://github.com/example/project/actions/runs/24" }] };
    },
    listJobsForWorkflowRunAttempt: async ({ run_id, attempt_number }) => {
      assert.equal(run_id, 24);
      assert.equal(attempt_number, 1);
      return { data: checkNames.map(name => ({ name, head_sha: receipt.head, status: "completed",
        conclusion: name === state.failedCheck ? "failure" : "success" })) };
    },
  };
  const paginate = f.github.paginate;
  f.github.paginate = async (method, args) =>
    typeof method === "string" ? [
      { type: "required_status_checks", parameters: {
        required_status_checks: checkNames.map(context => ({ context })),
      } },
      { type: "merge_queue", parameters: { merge_method: state.mergeMethod ?? "SQUASH" } },
    ] : paginate(method, args);
  const run = () => admitReleaseFold({ ...options, candidate, pause: async () => { state.merged = !state.evicted; } });
  return { state, run, receipt, events, mergedSha, entry, git: f.git };
}

function forbiddenCandidateRead(path) {
  return Object.assign(new Error("untrusted provider message"), {
    errors: [{ type: "FORBIDDEN", path, message: "untrusted provider detail" }],
    data: { repository: { pullRequest: { state: "OPEN" } } },
    request: { headers: { authorization: "fixture-only authorization" } },
  });
}

test("the workflow admits a fold without GraphQL repository queue configuration access", async () => {
  const f = await queueFixture();
  f.state.denyCandidateRead = query => {
    if (query.includes("mergeQueue(branch:")) {
      throw forbiddenCandidateRead(["repository", "mergeQueue"]);
    }
  };
  await f.run();
  assert.equal(f.state.mutations.length, 1);
  assert.equal(f.events.at(-1).mergeSha, f.mergedSha);
});

test("effective branch rules still require squash before adopting an existing queue entry", async () => {
  const f = await queueFixture();
  f.state.queued = true;
  f.state.mergeMethod = "MERGE";
  await assert.rejects(f.run(), /Protected squash merge queue is unavailable/);
  assert.deepEqual(f.state.mutations, []);
  assert.equal(f.events.at(-1).stage, "admission");
});

test("a denied candidate field fails closed and reports only a sanitized GraphQL path", async () => {
  const f = await queueFixture();
  for (const [path, suffix] of [
    [["repository", "pullRequest", "queueHistory", "nodes", 0, "actor"],
      " at repository.pullRequest.queueHistory.nodes[].actor"],
    [["repository", "untrusted path segment"], " (field path unavailable)"],
    [undefined, " (field path unavailable)"],
  ]) {
    f.state.denyCandidateRead = () => { throw forbiddenCandidateRead(path); };
    await assert.rejects(f.run(), error => {
      assert.equal(error.message, "GitHub GraphQL FoldAdmission: FORBIDDEN" + suffix);
      assert.equal(error.cause, undefined);
      assert.deepEqual(Object.keys(error), []);
      return true;
    });
    assert.equal(f.state.runs, 0);
    assert.deepEqual(f.state.mutations, []);
    assert.equal(f.events.at(-1).stage, "admission");
  }
});

const mergeGroupHeads = [{ oid: "e".repeat(40) }, null];

test("green CI enqueues the exact head and verifies merged SHA and path retirement on replay", async () => {
  for (const mergeGroupHead of mergeGroupHeads) {
    const f = await queueFixture({ mergeGroupHead });
    await f.run();
    assert.equal(f.state.runs, 2);
    assert.equal(f.state.mutations.length, 1);
    assert.ok(f.events.some(event => event.queueId === "QUEUE"));
    assert.equal(f.events.at(-1).mergeSha, f.mergedSha);
    assert.deepEqual(f.events.at(-1).capturedPaths, f.receipt.removed);
    await f.run();
    assert.equal(f.state.mutations.length, 1);
    assert.equal(f.events.at(-1).mergeSha, f.mergedSha);
  }
});

test("queued replay accepts the exact PR with or without a merge-group head", async () => {
  for (const mergeGroupHead of mergeGroupHeads) {
    const f = await queueFixture({ mergeGroupHead });
    f.state.queued = true;
    await f.run();
    assert.deepEqual(f.state.mutations, []);
    assert.ok(f.events.some(event => event.queueId === "QUEUE"));
    assert.equal(f.events.at(-1).mergeSha, f.mergedSha);
  }
});

for (const [field, wrong] of [["id", "OTHER_PR"], ["headRefOid", "f".repeat(40)]]) {
  test(`a mismatched ${field} in a queue receipt is rejected`, async () => {
    for (const replay of [false, true]) {
      const f = await queueFixture();
      f.state.queued = replay;
      f.entry.pullRequest[field] = wrong;
      await assert.rejects(f.run(), replay ? /App's exact head/ : /exact-head queue receipt/);
      assert.equal(f.state.mutations.length, replay ? 0 : 1);
      assert.ok(!f.events.some(event => event.queueId));
    }
  });
}

for (const variation of ["PR replaced", "base advanced", "head moved", "late hold", "freeze", "failed required check"]) {
  test(`${variation} prevents unattended enqueue`, async () => {
    const f = await queueFixture();
    const { state } = f;
    const onFinalRead = change => { state.onRead = pr => { if (state.reads === 2) change(pr); }; };
    const expected = {
      "PR replaced": /PR identity, recorded base or exact head changed/,
      "base advanced": /recorded fold base is stale/,
      "head moved": /exact head changed/,
      "late hold": /PR hold/,
      freeze: /repository freeze/,
      "failed required check": /Current required check did not pass/,
    };
    if (variation === "PR replaced") onFinalRead(pr => { pr.id = "OTHER_PR"; });
    if (variation === "base advanced") onFinalRead(() => { state.main = "f".repeat(40); });
    if (variation === "head moved") onFinalRead(pr => { pr.headRefOid = "f".repeat(40); });
    if (variation === "late hold") onFinalRead(pr => { pr.labels = { ...pr.labels, nodes: [{ name: "review::paused" }] }; });
    if (variation === "freeze") state.freeze = 1;
    if (variation === "failed required check") state.failedCheck = "test";
    await assert.rejects(f.run(), expected[variation]);
    assert.deepEqual(state.mutations, []);
  });
}

test("queue eviction fails the run without reenqueuing the removed head", async () => {
  const f = await queueFixture();
  f.state.evicted = true;
  await assert.rejects(f.run(), /removed the fold from the protected merge queue/);
  await assert.rejects(f.run(), /already left the merge queue/);
  assert.equal(f.state.mutations.length, 1);
  assert.equal(f.events.at(-1).stage, "admission");
});

test("merged provider state does not pass readback when the fragment still exists on current main", async () => {
  const f = await queueFixture();
  f.git("switch", "main");
  f.git("checkout", f.receipt.base, "--", ...f.receipt.removed);
  f.git("commit", "-qm", "Restore captured fragment on main");
  f.state.queued = true;
  f.state.merged = true;
  await assert.rejects(f.run(), /captured fragment still exists/);
  assert.deepEqual(f.state.mutations, []);
  assert.equal(f.events.at(-1).stage, "merge-receipt");
});
