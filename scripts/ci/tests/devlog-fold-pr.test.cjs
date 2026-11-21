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
function fixture({ pending = true, entryDate = "2020-01-02",
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
  const state = { pulls: [], writes: [], verified: true, base, occupied: false, moved: false };
  const repo = { owner: "example", repo: "project" };
  const fullName = "example/project";
  const head = "d".repeat(40);
  const rest = {
    pulls: {
      list: async args => {
        assert.equal(args.state, "all");
        assert.equal(args.head, `example:${branch}`);
        return { data: state.pulls };
      },
      create: async args => {
        state.writes.push(["pr", args]);
        const pr = { ...args, number: 1, html_url: "https://github.com/example/project/pull/1",
          head: { repo: { full_name: "example/project" } } };
        state.pulls.push(pr);
        return { data: pr };
      },
    },
    git: {
      getTag: async () => ({ data: { object: { type: "commit", sha: state.base } } }),
      getRef: async ({ ref }) => {
        if (ref === "tags/v1.2.3") return { data: { object: { type: "tag", sha: "e".repeat(40) } } };
        if (ref === "heads/main") return { data: { object: { sha: state.moved ? "f".repeat(40) : state.base } } };
        assert.equal(ref, `heads/${branch}`);
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
      getReleaseByTag: async () => ({ data: { id: 123, tag_name: "v1.2.3", published_at: `${date}T00:00:00Z`,
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
  return { state, git, dir, branch, options: { github,
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

function withRelease(f) {
  const args = { github: f.github, git: f.git, releaseId: "123",
    context: { ...f.options.context, ref: "refs/tags/v1.2.3", sha: f.state.base,
      eventName: "push", runId: 12345 },
    workflowRef: "example/project/.github/workflows/release.yml@refs/tags/v1.2.3" };
  return args;
}

test("the publication proof binds the release workflow, tag, ID and protected source", async () => {
  const f = fixture();
  const args = withRelease(f);
  const release = await readPublication(args);
  assert.equal(release.runId, args.context.runId);
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

async function production(f) {
  const events = [];
  const release = await readPublication(withRelease(f));
  const options = { ...f.options, repo: f.repo, writer: f.github, release, app: "fold-app",
    note: async (stage, details) => events.push({ stage, ...details }) };
  const receipt = await prepareReleaseFold(options);
  return { options: { ...options, receipt }, events, receipt };
}

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

async function queueFixture() {
  const f = fixture();
  const { options, receipt, events } = await production(f);
  const candidate = await publishReleaseFold(options);
  const checkNames = ["test", "test_containment", "cargo-deny (advisories + bans)"];
  const state = { reads: 0, runs: 0, queued: false, merged: false, mutations: [] };
  const merge = { id: "QUEUE", state: "QUEUED", headCommit: { oid: receipt.head } };
  const attempts = { number: 1, base: { sha: receipt.base, ref: "main" },
    head: { sha: receipt.head, ref: receipt.branch } };
  const pr = { id: "PR", state: "OPEN", isDraft: false, isCrossRepository: false,
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
      assert.deepEqual(vars, { id: "PR", head: receipt.head });
      state.queued = true;
      return { enqueuePullRequest: { mergeQueueEntry: merge } };
    }
    state.reads++;
    const current = { ...pr };
    state.onRead?.(current);
    current.isInMergeQueue = state.queued && !state.merged;
    if (state.mutations.length) current.lastEnqueue = { nodes: [{ id: "ENQUEUE", actor: { login: "fold-app" } }] };
    if (state.evicted && state.mutations.length) {
      current.isInMergeQueue = false;
      current.queueHistory = { nodes: [{ __typename: "RemovedFromMergeQueueEvent", reason: "check_failed" }] };
    }
    if (state.merged) {
      current.state = "MERGED";
      current.mergeCommit = { oid: mergedSha };
    } else if (state.queued) current.mergeQueueEntry = merge;
    return { repository: { main: { target: { oid: state.main ?? receipt.base } },
      mergeQueue: { configuration: { mergeMethod: "SQUASH" } },
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
      { type: "merge_queue", parameters: { merge_method: "SQUASH" } },
    ] : paginate(method, args);
  const run = () => admitReleaseFold({ ...options, candidate, pause: async () => { state.merged = !state.evicted; } });
  return { state, run, receipt, events, mergedSha };
}

test("green CI enqueues the exact head and verifies merged SHA and path retirement on replay", async () => {
  const f = await queueFixture();
  await f.run();
  assert.equal(f.state.runs, 2);
  assert.equal(f.state.mutations.length, 1);
  assert.ok(f.events.some(event => event.queueId === "QUEUE"));
  assert.equal(f.events.at(-1).mergeSha, f.mergedSha);
  assert.deepEqual(f.events.at(-1).capturedPaths, f.receipt.removed);
  await f.run();
  assert.equal(f.state.mutations.length, 1);
  assert.equal(f.events.at(-1).mergeSha, f.mergedSha);
});

for (const variation of ["base advanced", "head moved", "late hold", "freeze", "failed required check"]) {
  test(`${variation} prevents unattended enqueue`, async () => {
    const f = await queueFixture();
    const { state } = f;
    const onFinalRead = change => { state.onRead = pr => { if (state.reads === 2) change(pr); }; };
    const expected = {
      "base advanced": /recorded fold base is stale/,
      "head moved": /exact head changed/,
      "late hold": /PR hold/,
      freeze: /repository freeze/,
      "failed required check": /Current required check did not pass/,
    };
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
