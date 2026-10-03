#!/usr/bin/env node
"use strict";
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");
const modulePath = path.resolve(__dirname, "../../../.github/scripts/peekaboo-release-check.cjs");

test("release discovery implementation exists", () => assert.ok(fs.existsSync(modulePath)));
if (fs.existsSync(modulePath)) {
  const { runReleaseCheck } = require(modulePath);
  const release = (tag, extra = {}) => ({ tag_name: tag, draft: false, prerelease: false,
    assets: [{ name: "peekaboo-macos-universal.tar.gz" }, { name: `Peekaboo-${tag.slice(1)}.app.zip` }], ...extra });
  function fixture(releases = [release("v4.6.0")], issues = []) {
    const state = { releases, issues, writes: [], summary: "", failed: [], result: null, lists: [] };
    const listReleases = () => {};
    const listIssuesForRepo = () => {};
    const listUserIssues = () => {};
    const github = { paginate: async (method, args) => {
      state.lists.push(args);
      if (method === listReleases) return state.releases;
      if (method === listIssuesForRepo) return state.issues;
      if (method === listUserIssues) return state.issues.filter((issue) => issue.assigned_to_authenticated_user);
      throw new Error("unexpected endpoint");
    }, rest: { repos: { listReleases }, issues: { list: listUserIssues, listForRepo: listIssuesForRepo,
      create: async (args) => { state.writes.push(["create", args]); const issue = { number: 12, body: args.body,
        user: { login: "github-actions[bot]" }, html_url: "https://github.com/example/project/issues/12" };
        state.issues.push(issue); return { data: issue }; },
      update: async (args) => { state.writes.push(["update", args]); return { data: { html_url: "https://github.com/example/project/issues/12" } }; }
    } } };
    const core = { summary: { addRaw: (body) => { state.summary = body; return core.summary; }, write: async () => {} },
      setFailed: (message) => state.failed.push(message), warning: () => {} };
    const options = { github, core, context: { repo: { owner: "example", repo: "project" }, runId: 42 },
      lock: { repository: "https://github.com/openclaw/Peekaboo", tag: "v4.4.0" },
      now: () => new Date("2026-10-03T00:00:00Z"), writeResult: (value) => { state.result = value; } };
    return { state, options };
  }
  test("discovers the highest stable official SemVer, independent of API order", async () => {
    const { state, options } = fixture([release("v4.6.0"), release("v4.10.0"), release("v5.0.0", { draft: true }),
      release("v5.1.0", { prerelease: true }), release("v5.2.0-rc.1"), release("nightly"), release("v4.5.0")]);
    const before = JSON.stringify(options.lock);
    await runReleaseCheck(options);
    assert.equal(state.result.candidate, "v4.10.0");
    assert.equal(state.result.status, "candidate");
    assert.equal(JSON.stringify(options.lock), before);
    assert.equal(state.writes[0][0], "create");
    assert.deepEqual(state.lists[0], { owner: "openclaw", repo: "Peekaboo", per_page: 100 });
    for (const text of ["2026-10-03T00:00:00.000Z", "v4.4.0", "v4.10.0", "not selected", "42"]) {
      assert.ok(state.summary.includes(text), text);
    }
  });
  test("equal or older releases create no candidate records", async () => {
    for (const tag of ["v4.4.0", "v4.3.0"]) {
      const { state, options } = fixture([release(tag)]);
      await runReleaseCheck(options);
      assert.equal(state.result.status, "current");
      assert.equal(state.result.candidate, null);
      assert.equal(state.writes.length, 0);
    }
  });
  test("repeat runs update the same record and preserve maintainer notes and closed decisions", async () => {
    const { state, options } = fixture();
    await runReleaseCheck(options);
    state.issues[0].state = "closed";
    state.issues[0].body += "\nMaintainer: rejected after compatibility testing.\n";
    options.now = () => new Date("2026-10-04T00:00:00Z");
    await runReleaseCheck(options);
    assert.deepEqual(state.writes.map(([kind]) => kind), ["create", "update"]);
    assert.equal(state.lists[3].state, "all");
    const update = state.writes[1][1];
    assert.ok(update.body.includes("Maintainer: rejected"));
    assert.ok(update.body.includes("2026-10-04T00:00:00.000Z"));
    assert.equal(update.state, undefined);
  });
  test("repository discovery reuses a closed unassigned candidate without creating one", async () => {
    const marker = "<!-- peekaboo-release-check:v4.6.0 -->";
    const record = { number: 17, state: "closed", user: { login: "github-actions[bot]" },
      body: `${marker}\nOld check\n<!-- peekaboo-release-check:end -->\nMaintainer decision`,
      html_url: "https://github.com/example/project/issues/17" };
    const { state, options } = fixture([release("v4.6.0")], [record]);
    await runReleaseCheck(options);
    assert.deepEqual(state.writes.map(([kind]) => kind), ["update"]);
    assert.equal(state.writes[0][1].issue_number, 17);
    assert.deepEqual(state.lists[1], { owner: "example", repo: "project", state: "all", per_page: 100 });
    assert.equal(state.writes[0][1].state, undefined);
  });
  test("a foreign-owned exact candidate marker refuses all writes", async () => {
    const { state, options } = fixture([release("v4.6.0")], [{ number: 18, state: "closed",
      user: { login: "maintainer" }, body: "<!-- peekaboo-release-check:v4.6.0 -->\nMaintainer decision" }]);
    await runReleaseCheck(options);
    assert.equal(state.result.status, "error");
    assert.equal(state.failed.length, 1);
    assert.equal(state.writes.length, 0);
    assert.match(state.summary, /foreign|owner|ownership/);
  });
  test("missing official install assets reports an unsupported candidate without touching the lock", async () => {
    const { state, options } = fixture([release("v4.6.0", { assets: [] })]);
    await runReleaseCheck(options);
    assert.equal(state.result.status, "unsupported");
    assert.ok(state.summary.includes("peekaboo-macos-universal.tar.gz"));
    assert.ok(state.writes[0][1].body.includes("keep the accepted backend"));
    assert.equal(options.lock.tag, "v4.4.0");
  });
  test("API errors and absent stable releases produce a visible failed report, without candidate writes", async () => {
    for (const unavailable of [false, true]) {
      const { state, options } = fixture([]);
      if (unavailable) options.github.paginate = async () => { throw new Error("upstream unavailable"); };
      await runReleaseCheck(options);
      assert.equal(state.result.status, "error");
      assert.equal(state.failed.length, 1);
      assert.ok(state.summary.includes("keep the accepted backend"));
      assert.equal(state.writes.length, 0);
    }
  });
  test("ambiguous existing candidate records fail closed instead of creating another", async () => {
    for (const owner of ["github-actions[bot]", "maintainer"]) {
      const { state, options } = fixture();
      await runReleaseCheck(options);
      state.issues.push({ ...state.issues[0], number: 13, user: { login: owner } });
      state.writes = [];
      await runReleaseCheck(options);
      assert.equal(state.result.status, "error");
      assert.equal(state.writes.length, 0);
      assert.ok(state.summary.includes("duplicate"));
    }
  });
  test("issue-write errors remain visible in summary and result", async () => {
    const { state, options } = fixture();
    options.github.rest.issues.create = async () => { throw new Error("write unavailable"); };
    await runReleaseCheck(options);
    assert.equal(state.result.status, "error");
    assert.equal(state.result.candidate, "v4.6.0");
    assert.equal(state.failed.length, 1);
  });
  test("a foreign lock repository fails closed before any API requests", async () => {
    const { state, options } = fixture();
    options.lock.repository = "https://github.com/example/Peekaboo";
    await runReleaseCheck(options);
    assert.equal(state.result.status, "error");
    assert.equal(state.lists.length, 0);
  });
  test("daily and manual workflow serializes writes and runs fixtures", () => {
    const workflow = fs.readFileSync(path.resolve(__dirname, "../../../.github/workflows/peekaboo-release-check.yml"), "utf8");
    for (const pattern of [/schedule:/, /cron: "\d+ \d+ \* \* \*"/, /workflow_dispatch:/,
      /group: peekaboo-release-check/, /cancel-in-progress: false/, /issues: write/,
      /persist-credentials: false/, /node scripts\/ci\/tests\/peekaboo-release-check.test.cjs/,
      /if: always\(\)/, /github.ref == 'refs\/heads\/main'/]) assert.match(workflow, pattern);
  });
}
