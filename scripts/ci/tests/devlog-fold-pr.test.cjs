const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const { test } = require("node:test");
const root = path.resolve(__dirname, "../../..");

test("release publication has a fold workflow with manual dry-run recovery", () => {
  const file = path.join(root, ".github/workflows/devlog-fold.yml");
  assert.ok(fs.existsSync(file), "release publication must have a devlog fold workflow");
  const workflow = fs.readFileSync(file, "utf8");
  assert.match(workflow, /release:\s*\n\s+types: \[published\]/);
  assert.match(workflow, /workflow_dispatch:/);
  assert.match(workflow, /dry_run:/);
  assert.match(workflow, /BOT_APP_ID/);
  assert.match(workflow, /BOT_APP_PRIVATE_KEY/);
  assert.match(workflow, /persist-credentials: false/);
  assert.match(workflow, /ref: main/);
  assert.match(workflow, /if \(!dryRun && !process.env.BOT_TOKEN\)/);
  assert.match(workflow, /const client = dryRun \? github : getOctokit\(process.env.BOT_TOKEN\)/);
});

const os = require("node:os");
const { execFileSync } = require("node:child_process");
const temporary = [];
test.after(() => temporary.forEach(dir => fs.rmSync(dir, { recursive: true, force: true })));
const subject = "chore(devlog): fold accepted fragments";
function fixture({ pending = true, entryDate = "2020-01-02" } = {}) {
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
  for (const file of ["scripts/ci/fold-devlog.sh", "scripts/dev-env.sh"])
    write(file, fs.readFileSync(path.join(root, file), "utf8"));
  if (pending) run("devlog", ["new", "--date", entryDate, "--slug", "accepted",
    "--title", "Accepted outcome", "--result", "An outcome.", "--why", "A reason.", "--evidence", "A check."]);
  run("devlog", ["new", "--date", "2099-01-01", "--slug", "future", "--title", "Future outcome",
    "--result", "An outcome.", "--why", "A reason.", "--evidence", "A check."]);
  git("add", ".");
  git("commit", "-qm", "Fixture baseline");
  const base = git("rev-parse", "HEAD");
  const date = new Date().toISOString().slice(0, 10);
  const branch = `chore/devlog-fold-${date}`;
  const state = { pulls: [], writes: [], verified: true, base, occupied: false, moved: false };
  const rest = {
    pulls: {
      list: async args => {
        assert.equal(args.state, "all");
        assert.equal(args.head, `example:${branch}`);
        return { data: state.pulls };
      },
      create: async args => {
        state.writes.push(["pr", args]);
        const pr = { ...args, html_url: "https://github.com/example/project/pull/1",
          head: { repo: { full_name: "example/project" } } };
        state.pulls.push(pr);
        return { data: pr };
      },
    },
    git: {
      getRef: async ({ ref }) => {
        if (ref === "heads/main") return { data: { object: { sha: state.moved ? "f".repeat(40) : base } } };
        assert.equal(ref, `heads/${branch}`);
        if (!state.occupied) throw Object.assign(new Error("not found"), { status: 404 });
        return { data: { object: { sha: "e".repeat(40) } } };
      },
      getCommit: async () => ({ data: { tree: { sha: "b".repeat(40) } } }),
      createTree: async args => { state.writes.push(["tree", args]); return { data: { sha: "c".repeat(40) } }; },
      createCommit: async args => {
        state.writes.push(["commit", args]);
        return { data: { sha: "d".repeat(40), verification: { verified: state.verified } } };
      },
      createRef: async args => { state.writes.push(["ref", args]); return { data: {} }; },
    },
  };
  const github = { rest, paginate: async (method, args) => (await method(args)).data };
  return { state, git, dir, branch, options: { github,
    context: { repo: { owner: "example", repo: "project" } }, root: dir, env } };
}
function runFold(options) {
  return require("../../../.github/scripts/devlog-fold-pr.cjs").runFoldPR(options);
}

test("dry-run executes the real fold and leaves every remote ref untouched", async () => {
  const f = fixture();
  const result = await runFold({ ...f.options, dryRun: true });
  assert.equal(result.status, "dry-run");
  assert.equal(result.branch, f.branch);
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
