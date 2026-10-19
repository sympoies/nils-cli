"use strict";

const assert = require("node:assert/strict");
const { test } = require("node:test");
const { TITLE, labelDeferredChecks } = require("../../../.github/scripts/deferred-full-checks.cjs");

function fixture(jobs, snapshots = [jobs]) {
  let reads = 0;
  const sleeps = [];
  const updates = [];
  const summary = { addHeading: () => summary, addRaw: () => summary, write: async () => {} };
  return {
    updates,
    sleeps,
    reads: () => reads,
    input: {
      context: { repo: { owner: "example", repo: "project" }, runId: 17 },
      core: { summary },
      sleep: async (milliseconds) => sleeps.push(milliseconds),
      github: {
        rest: { actions: { listJobsForWorkflowRun() {} }, checks: { update: async (data) => updates.push(data) } },
        paginate: async (_route, params) => {
          assert.deepEqual(params, { owner: "example", repo: "project", run_id: 17, filter: "all", per_page: 100 });
          return snapshots[Math.min(reads++, snapshots.length - 1)];
        },
      },
    },
  };
}

const skipped = [{ id: 21, name: "test_macos", run_attempt: 1, status: "completed", conclusion: "skipped", check_run_url: "https://api.github.com/repos/example/project/check-runs/211" },
  { id: 22, name: "coverage", run_attempt: 1, status: "completed", conclusion: "skipped", check_run_url: "https://api.github.com/repos/example/project/check-runs/222" }];

test("labels the actual skipped required checks without forging a conclusion", async () => {
  const { input, updates } = fixture(skipped);
  await labelDeferredChecks(input);
  assert.deepEqual(updates.map((data) => data.check_run_id), [211, 222]);
  for (const update of updates) {
    assert.equal(update.output.title, TITLE);
    assert.match(update.output.summary, /did not run on this PR/);
    assert.equal("conclusion" in update, false);
  }
});

for (const state of ["success", "failure", "cancelled"]) {
  test(`does not label ${state} as a skipped deferral`, async () => {
    const { input, updates } = fixture([skipped[0], { ...skipped[1], conclusion: state }]);
    await assert.rejects(labelDeferredChecks(input), /completed skipped coverage/);
    assert.deepEqual(updates, []);
  });
}

test("does not label missing or ambiguous required jobs", async () => {
  for (const jobs of [[skipped[0]], [...skipped, skipped[0]]]) {
    const { input, updates } = fixture(jobs);
    await assert.rejects(labelDeferredChecks(input), /Expected one/);
    assert.deepEqual(updates, []);
  }
});

for (const name of ["test_macos", "coverage"]) {
  test(`waits for ${name} to finish before labeling either check`, async () => {
    const pending = skipped.map((job) => job.name === name
      ? { ...job, status: "in_progress", conclusion: null } : job);
    const { input, updates, sleeps, reads } = fixture(pending, [pending, skipped]);
    input.sleep = async (milliseconds) => {
      assert.deepEqual(updates, []);
      sleeps.push(milliseconds);
    };
    await labelDeferredChecks(input);
    assert.equal(reads(), 2);
    assert.deepEqual(sleeps, [5000]);
    assert.deepEqual(updates.map((data) => data.check_run_id), [211, 222]);
  });
}

test("waits for a required job to appear in the job list", async () => {
  const { input, updates, reads } = fixture([], [[skipped[0]], skipped]);
  await labelDeferredChecks(input);
  assert.equal(reads(), 2);
  assert.deepEqual(updates.map((data) => data.check_run_id), [211, 222]);
});

test("fails without labeling after a required job never settles", async () => {
  const pending = [skipped[0], { ...skipped[1], status: "queued", conclusion: null }];
  const { input, updates, sleeps, reads } = fixture(pending);
  await assert.rejects(labelDeferredChecks(input), /completed skipped coverage.*25 attempts/);
  assert.equal(reads(), 25);
  assert.deepEqual(sleeps, Array(24).fill(5000));
  assert.deepEqual(updates, []);
});

test("labels skipped siblings from an earlier attempt when only the labeler reruns", async () => {
  const { input, updates } = fixture([...skipped,
    { name: "deferred_full", run_attempt: 2, status: "in_progress", conclusion: null }]);
  await labelDeferredChecks(input);
  assert.deepEqual(updates.map((data) => data.check_run_id), [211, 222]);
});

test("waits for the newest sibling attempt instead of labeling its older skip", async () => {
  const newer = { ...skipped[1], id: 23, run_attempt: 2,
    check_run_url: "https://api.github.com/repos/example/project/check-runs/233" };
  const pending = [...skipped, { ...newer, status: "queued", conclusion: null }];
  const { input, updates, reads } = fixture(pending, [pending, [newer, ...skipped]]);
  await labelDeferredChecks(input);
  assert.equal(reads(), 2);
  assert.deepEqual(updates.map((data) => data.check_run_id), [211, 233]);
});

test("does not label an older skip when the newest sibling attempt succeeded", async () => {
  const { input, updates } = fixture([
    { ...skipped[1], id: 23, run_attempt: 2, conclusion: "success" }, ...skipped]);
  await assert.rejects(labelDeferredChecks(input), /completed skipped coverage/);
  assert.deepEqual(updates, []);
});
