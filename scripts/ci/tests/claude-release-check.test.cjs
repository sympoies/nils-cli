#!/usr/bin/env node
"use strict";
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const crypto = require("node:crypto");
const test = require("node:test");
const { discover, verifyIntegrity, isolatedEnvironment, candidateChanges, checkCandidate,
  runReleaseCheck, range } = require("../../../.github/scripts/claude-release-check.cjs");
const REPOSITORY = path.resolve(__dirname, "../../..");
const VERSION = "crates/agent-session/src/send/claude_notice/version.rs";
const NOTICE = "crates/agent-session/tests/guarded_send/claude_notice.rs";
const SPEC = "crates/agent-session/docs/specs/session-notices-v1.md";
const temporary = [];
test.after(() => temporary.forEach(dir => fs.rmSync(dir, { recursive: true, force: true })));
// Stable owner snapshots keep discovery/upsert fixtures independent of the
// reviewed range after a future candidate merges. A separate smoke test below
// checks that the current real owners can still produce a range-only patch.
const ROOT = fs.mkdtempSync(path.join(os.tmpdir(), "claude-release-owners-"));
temporary.push(ROOT);
for (const [file, text] of Object.entries({
  [VERSION]: `pub(crate) const SUPPORTED_MIN: Version = Version(2, 1, 296);
pub(crate) const SUPPORTED_MAX: Version = Version(2, 1, 316);
let (min, max) = (Version(2, 1, 296), Version(2, 1, 316));
[("2.1.296", true), ("2.1.316", true), ("2.1.317", false), ("2.2.0", false)]
`,
  [NOTICE]: `("1", "2.1.316", true), ("1", "2.1.317", false),
{"supported_min": "2.1.296", "supported_max": "2.1.316"}
"notice-native-version-unsupported"
`,
  [SPEC]: "### Tested Claude Code range\n\nReviewed: `2.1.296` through `2.1.316` inclusive.\nThe current upper bound, `2.1.316`, rests on existing exact native evidence.\n",
})) {
  fs.mkdirSync(path.dirname(path.join(ROOT, file)), { recursive: true });
  fs.writeFileSync(path.join(ROOT, file), text);
}
const PACKAGE = "@anthropic-ai/claude-code";
const PLATFORM = `${PACKAGE}-linux-x64`;
const integrity = (bytes) => `sha512-${crypto.createHash("sha512").update(bytes).digest("base64")}`;
function metadata(version = "2.1.318", name = PACKAGE) {
  return { name, version, optionalDependencies: { [PLATFORM]: version },
    dist: { tarball: `https://registry.npmjs.org/${name}/-/claude-code${name === PLATFORM ? "-linux-x64" : ""}-${version}.tgz`,
      integrity: integrity(Buffer.from(name)) } };
}
function fixture({ newest = "2.1.318", passing = true } = {}) {
  const artifactDir = fs.mkdtempSync(path.join(os.tmpdir(), "claude-release-provider-")); temporary.push(artifactDir);
  const state = { issues: [], pulls: [], refs: new Map(), commits: new Map(), writes: [],
    failures: [], checked: 0, reviews: [], comments: [], source: "a".repeat(40) };
  const rest = {
    repos: { get: async () => ({ data: { default_branch: "main" } }) },
    issues: {
      listForRepo: async () => ({ data: state.issues }),
      create: async (args) => { const issue = { ...args, number: 9, state: "open", user: { login: "github-actions[bot]" }, html_url: "https://github.com/example/project/issues/9" };
        state.issues.push(issue); state.writes.push(["issue", args]); return { data: issue }; },
      update: async (args) => { Object.assign(state.issues.find(x => x.number === args.issue_number), args); state.writes.push(["issue-update", args]); return { data: {} }; },
      listComments: async () => ({ data: state.comments }),
    },
    pulls: {
      list: async () => ({ data: state.pulls }),
      listReviews: async () => ({ data: state.reviews }),
      listComments: async () => ({ data: state.comments }),
      create: async (args) => { const pr = { ...args, number: 10, state: "open", user: { login: "github-actions[bot]" },
        head: { ref: args.head, sha: "c".repeat(40), repo: { full_name: "example/project" } },
        base: { ref: "main" }, html_url: "https://github.com/example/project/pull/10" };
        state.pulls.push(pr); state.writes.push(["pr", args]); return { data: pr }; },
    },
    git: {
      getRef: async ({ ref }) => { if (ref === "heads/main") return { data: { object: { sha: state.source } } };
        if (!state.refs.has(ref)) throw Object.assign(new Error("not found"), { status: 404 });
        return { data: { object: { sha: state.refs.get(ref) } } }; },
      getCommit: async ({ commit_sha }) => ({ data: state.commits.get(commit_sha) || { tree: { sha: "b".repeat(40) } } }),
      createTree: async (args) => { state.writes.push(["tree", args]); return { data: { sha: "d".repeat(40) } }; },
      createCommit: async (args) => { state.commits.set("c".repeat(40), { ...args, parents: args.parents.map(sha => ({ sha })) }); state.writes.push(["commit", args]); return { data: { sha: "c".repeat(40) } }; },
      createRef: async (args) => { state.refs.set(args.ref.replace(/^refs\//, ""), args.sha); state.writes.push(["ref", args]); return { data: {} }; },
    },
  };
  const core = { setFailed: (x) => state.failures.push(x), summary: { addRaw: () => core.summary, write: async () => {} } };
  return { state, options: { github: { rest, paginate: async (method, args) => (await method(args)).data }, core,
    context: { repo: { owner: "example", repo: "project" }, ref: "refs/heads/main", sha: state.source, runId: 7 },
    root: ROOT, artifactDir, discoverRelease: async () => metadata(newest),
    checkRelease: async () => { state.checked++; return { passed: passing, commands: [{ command: "contract tests", exit: passing ? 0 : 1 }],
      packages: [{ name: PACKAGE, version: newest, ...metadata(newest).dist, verified: true }], failure: passing ? null : "module contract failed" }; },
    writeResult: (result) => { state.result = result; } } };
}

test("official latest registry discovery uses strict stable versions and exact package identity", async () => {
  const seen = [];
  const item = await discover(async (url) => { seen.push(url); return metadata(); });
  assert.equal(item.version, "2.1.318");
  assert.deepEqual(seen, ["https://registry.npmjs.org/@anthropic-ai%2Fclaude-code/latest"]);
  for (const item of [metadata("2.1.319-rc.1"), metadata("v2.1.319"), metadata("02.1.319"), metadata("2.1.319", "foreign"),
    { ...metadata(), dist: { ...metadata().dist, tarball: "https://untrusted.invalid/p.tgz" } },
    { ...metadata(), dist: { ...metadata().dist, integrity: "sha1-AAAA" } }]) {
    await assert.rejects(() => discover(async () => item));
  }
});

test("integrity compares actual archive bytes, refusing malformed or mismatched SRI", () => {
  const bytes = Buffer.from("official fixture archive");
  assert.equal(verifyIntegrity(bytes, integrity(bytes)), true);
  for (const sri of [integrity(Buffer.from("other")), "sha512-invalid", "sha256-AAAA", `${integrity(bytes)} injected`]) {
    assert.throws(() => verifyIntegrity(bytes, sri), /integrity/);
  }
});

test("child environment uses only private homes and fixed executable lookup; no ambient credentials or options", () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "claude-release-fixture-"));
  try {
    process.env.ANTHROPIC_API_KEY = "synthetic-only";
    process.env.NODE_OPTIONS = "synthetic-only";
    const env = isolatedEnvironment(dir);
    assert.equal(env.ANTHROPIC_API_KEY, undefined);
    assert.equal(env.NODE_OPTIONS, undefined);
    assert.equal(env.AGENT_HOME, undefined);
    assert.equal(env.GITHUB_TOKEN, undefined);
    assert.equal(env.CLAUDE_CONFIG_DIR, path.join(dir, "config"));
    assert.equal(env.DISABLE_AUTOUPDATER, "1");
    assert.equal(env.PATH, `${path.join(dir, "bin")}:${path.dirname(process.execPath)}:/usr/bin:/bin`);
    for (const key of ["HOME", "TMPDIR", "XDG_CONFIG_HOME", "XDG_CACHE_HOME", "XDG_DATA_HOME", "XDG_STATE_HOME"]) {
      assert.ok(env[key].startsWith(`${dir}/`));
      assert.ok(fs.statSync(env[key]).isDirectory());
    }
  } finally { delete process.env.ANTHROPIC_API_KEY; delete process.env.NODE_OPTIONS; fs.rmSync(dir, { recursive: true, force: true }); }
});

test("candidate patch changes only maximum, necessary expectations and prospective docs, keeping minimum and refusals", () => {
  const changes = candidateChanges(ROOT, "2.1.318");
  assert.deepEqual(Object.keys(changes).sort(), [VERSION, NOTICE, SPEC].sort());
  assert.match(changes[VERSION], /SUPPORTED_MIN: Version = Version\(2, 1, 296\)/);
  assert.match(changes[VERSION], /SUPPORTED_MAX: Version = Version\(2, 1, 318\)/);
  assert.match(changes[VERSION], /"2.1.319", false/);
  assert.match(changes[NOTICE], /"supported_max": "2.1.318"/);
  assert.ok(!changes[NOTICE].includes('"2.1.317"'));
  assert.match(changes[NOTICE], /notice-native-version-unsupported/);
  assert.match(changes[SPEC], /Native acceptance: PENDING/);
  assert.match(changes[SPEC], /2\.1\.316.*rests/);
});

test("in-range runs have no installation or provider mutation, with numeric comparison", async () => {
  for (const newest of ["2.1.316", "2.1.99"]) {
    const { state, options } = fixture({ newest });
    await runReleaseCheck(options);
    assert.equal(state.result.status, "current"); assert.equal(state.checked, 0); assert.deepEqual(state.writes, []);
  }
});

test("passing candidate creates one draft with exact source/integrity/exits/artifact and native pending", async () => {
  const { state, options } = fixture();
  await runReleaseCheck(options);
  assert.equal(state.result.status, "draft");
  const pr = state.pulls[0];
  assert.equal(pr.draft, true); assert.equal(pr.base.ref, "main");
  for (const text of ["2.1.318", "Native acceptance: PENDING", "exact-version native", "reviewed merge", "Refs #3138", "Test changes", metadata().dist.integrity,
    options.context.sha, "contract tests", "exit 0", "/actions/runs/7", "claude-release-check"]) assert.ok(pr.body.includes(text), text);
  assert.equal(state.issues.length, 0);
  const tree = state.writes.find(([kind]) => kind === "tree")[1];
  assert.deepEqual(tree.tree.map(x => x.path).sort(), [VERSION, NOTICE, SPEC].sort());
  const before = state.writes.length;
  await runReleaseCheck(options);
  assert.equal(state.pulls.length, 1); assert.equal(state.writes.length, before);
});

test("failing candidate upserts a single bounded issue and preserves human notes/closed state", async () => {
  const { state, options } = fixture({ passing: false });
  await runReleaseCheck(options);
  assert.equal(state.result.status, "failed"); assert.equal(state.pulls.length, 0);
  state.issues[0].state = "closed"; state.issues[0].body += "\nMaintainer decision: defer.\n";
  await runReleaseCheck(options);
  assert.equal(state.issues.length, 1); assert.equal(state.issues[0].state, "closed");
  assert.match(state.issues[0].body, /Maintainer decision: defer/);
  assert.ok(state.issues[0].body.length < 16000);
  assert.ok(!state.writes.some(([k, args]) => k === "issue-update" && args.state));
});

test("default branch and source movement fail closed before writing candidate refs", async () => {
  for (const variant of ["branch", "moved"]) {
    const { state, options } = fixture();
    if (variant === "branch") options.context.ref = "refs/heads/topic";
    else state.source = "e".repeat(40);
    await runReleaseCheck(options);
    assert.equal(state.result.status, "error"); assert.ok(state.failures.length);
    assert.ok(!state.writes.some(([kind]) => ["ref", "pr", "commit", "tree"].includes(kind)));
  }
});

test("existing human edits, reviews, ready/closed PRs, duplicates and occupied branches never get overwritten", async () => {
  for (const variant of ["body", "author", "review", "comment", "ready", "closed", "head", "duplicate", "branch"]) {
    const { state, options } = fixture();
    await runReleaseCheck(options);
    const pr = state.pulls[0];
    if (variant === "body") pr.body += "\nHuman edits";
    if (variant === "author") pr.user.login = "maintainer";
    if (variant === "review") state.reviews.push({ user: { login: "maintainer" } });
    if (variant === "comment") state.comments.push({ user: { login: "maintainer" } });
    if (variant === "ready") pr.draft = false;
    if (variant === "closed") pr.state = "closed";
    if (variant === "head") pr.head.sha = "f".repeat(40);
    if (variant === "duplicate") state.pulls.push({ ...pr });
    if (variant === "branch") { state.pulls = []; state.refs.set(`heads/${pr.head.ref}`, "f".repeat(40)); }
    const before = state.writes.length;
    await runReleaseCheck(options);
    assert.ok(["preserved", "error"].includes(state.result.status), `${variant}: ${state.result.status}`);
    assert.ok(!state.writes.slice(before).some(([kind]) => ["ref", "commit", "tree", "pr"].includes(kind)), variant);
    if (variant !== "branch") assert.equal(state.pulls[0]?.body, pr.body);
  }
});

test("candidate creation unavailable leaves an actionable bounded issue and failed run, retries without ref overwrite", async () => {
  for (const stage of ["createTree", "createRef", "pulls"]) {
    const { state, options } = fixture();
    const unavailable = async () => { throw Object.assign(new Error("synthetic credential diagnostic NEVER FORWARD"), { status: 403 }); };
    if (stage === "pulls") options.github.rest.pulls.create = unavailable;
    else options.github.rest.git[stage] = unavailable;
    await runReleaseCheck(options); await runReleaseCheck(options);
    assert.equal(state.result.status, "error"); assert.ok(state.failures.length);
    assert.equal(state.issues.length, 1);
    assert.match(state.issues[0].body, /candidate creation unavailable/, stage);
    assert.match(state.issues[0].body, /maintainer/);
    assert.ok(!JSON.stringify(state.result).includes("NEVER FORWARD"));
    assert.equal(state.writes.filter(([k]) => k === "ref").length, stage === "pulls" ? 1 : 0);
  }
});

test("foreign or edited failure issue refuses update rather than replacing human-owned evidence", async () => {
  for (const variant of ["author", "body", "duplicate"]) {
    const { state, options } = fixture({ passing: false });
    await runReleaseCheck(options);
    if (variant === "author") state.issues[0].user.login = "maintainer";
    if (variant === "body") state.issues[0].body = state.issues[0].body.replace("Module contract", "human correction");
    if (variant === "duplicate") state.issues.push({ ...state.issues[0] });
    const before = state.writes.length;
    await runReleaseCheck(options);
    assert.equal(state.result.status, "error"); assert.equal(state.writes.length, before);
  }
});

test("isolated candidate verifies both archives, exact banner and non-skipped existing contract test", async () => {
  for (const variant of ["pass", "integrity", "version", "skip", "exit"]) {
    const calls = [];
    const release = metadata();
    const fetchJSON = async () => metadata(release.version, PLATFORM);
    const fetchBytes = async (url) => Buffer.from(variant === "integrity" ? "tampered" : url.includes("claude-code-linux-x64/-") ? PLATFORM : PACKAGE);
    const execute = (command, args, options) => {
      calls.push({ command, args, options });
      if (command === "npm") {
        const target = path.join(options.cwd, "install/node_modules", PLATFORM, "claude");
        fs.mkdirSync(path.dirname(target), { recursive: true }); fs.writeFileSync(target, "synthetic executable");
      }
      return { status: variant === "exit" ? 1 : 0,
        stdout: args.includes("--version") ? `${variant === "version" ? "2.1.317" : release.version} (Claude Code)\n`
          : variant === "skip" ? "skipped: no Claude Code installed" : `checking the module against Claude Code ${release.version}\ntest result: ok. 3 passed; 0 failed; 0 ignored;`, stderr: "" };
    };
    const result = await checkCandidate({ release, contractBinary: "/synthetic/contract-tests", root: ROOT, fetchJSON, fetchBytes, execute });
    assert.equal(result.passed, variant === "pass", variant);
    if (variant === "integrity") assert.equal(calls.length, 0);
    for (const call of calls) {
      assert.equal(call.options.env.ANTHROPIC_API_KEY, undefined);
      assert.equal(call.options.env.CLAUDE_CONFIG_DIR.startsWith(call.options.cwd), true);
      assert.ok(call.options.timeout > 0); assert.ok(call.options.maxBuffer <= 1024 * 1024);
    }
    if (variant === "pass") {
      assert.equal(result.packages.length, 2);
      assert.ok(calls.find(x => x.command === "npm").args.includes("--ignore-scripts"));
      assert.ok(calls.find(x => x.command === "/synthetic/contract-tests").args.includes("send::claude_notice::module_contract::"));
    }
  }
});

test("the immediate next release remains the accepted maximum in its candidate patch", () => {
  const changes = candidateChanges(ROOT, "2.1.317");
  assert.match(changes[VERSION], /\("2\.1\.317", true\)/);
  assert.match(changes[NOTICE], /"supported_max": "2.1.317"/);
  assert.match(changes[NOTICE], /"2.1.318",/);
});

test("repeat failure runs reuse immutable first evidence when source/run details change", async () => {
  const { state, options } = fixture({ passing: false });
  await runReleaseCheck(options);
  const original = state.issues[0].body;
  options.context.runId = 8;
  options.context.sha = "e".repeat(40);
  const before = state.writes.length;
  await runReleaseCheck(options);
  assert.equal(state.writes.length, before);
  assert.equal(state.issues[0].body, original);
  assert.equal(state.result.issue_url, state.issues[0].html_url);
});


test("current repository owners can prepare their next maximum without changing implementation checkout", () => {
  const { max, min } = range(REPOSITORY);
  const parts = max.split(".").map(Number); parts[2]++;
  const candidate = parts.join(".");
  const original = fs.readFileSync(path.join(REPOSITORY, VERSION), "utf8");
  const changes = candidateChanges(REPOSITORY, candidate);
  assert.deepEqual(Object.keys(changes).sort(), [VERSION, NOTICE, SPEC].sort());
  assert.ok(changes[VERSION].includes(`SUPPORTED_MAX: Version = Version(${parts.join(", ")});`));
  assert.ok(changes[NOTICE].includes(`"supported_max": "${candidate}"`));
  assert.ok(changes[SPEC].includes(`Native acceptance: PENDING for candidate \`${candidate}\``));
  assert.ok(changes[VERSION].includes(`SUPPORTED_MIN: Version = Version(${min.replaceAll(".", ", ")});`));
  assert.equal(fs.readFileSync(path.join(REPOSITORY, VERSION), "utf8"), original);
});

test("workflow selects the real agent_session library executable from Cargo artifact messages", () => {
  const vm = require("node:vm");
  const workflow = fs.readFileSync(path.join(REPOSITORY, ".github/workflows/claude-release-check.yml"), "utf8");
  const compile = workflow.slice(workflow.indexOf("- name: Compile existing module contract tests"));
  const script = compile.match(/node <<'JS'\n([\s\S]*?)\n          JS/)[1].split("\n").map(line => line.slice(10)).join("\n");
  const writes = [];
  vm.runInNewContext(script, {
    require: (name) => name === "node:fs" ? { writeFileSync: () => {}, appendFileSync: (file, text) => writes.push([file, text]) }
      : { spawnSync: () => ({ status: 0, stdout: `${JSON.stringify({ reason: "compiler-artifact", profile: { test: true },
        target: { name: "agent_session" }, executable: "/synthetic/agent_session-tests" })}\n`, stderr: "" }) },
    process: { env: { GITHUB_ENV: "/synthetic/environment" }, exit: (code) => { throw new Error(`exit ${code}`); } },
  });
  assert.deepEqual(writes, [["/synthetic/environment", "CLAUDE_CONTRACT_TEST_BINARY=/synthetic/agent_session-tests\n"]]);
});
