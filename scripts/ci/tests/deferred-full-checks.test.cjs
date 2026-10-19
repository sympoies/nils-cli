"use strict";

const assert = require("node:assert/strict");
const { test } = require("node:test");
const { TITLE, labelDeferredChecks } = require("../../../.github/scripts/deferred-full-checks.cjs");

function fixture(jobs) {
  const updates = [];
  const summary = { addHeading: () => summary, addRaw: () => summary, write: async () => {} };
  return {
    updates,
    input: {
      context: { repo: { owner: "example", repo: "project" }, runId: 17 },
      core: { summary },
      github: {
        rest: { actions: { listJobsForWorkflowRun() {} }, checks: { update: async (data) => updates.push(data) } },
        paginate: async (_route, params) => {
          assert.deepEqual(params, { owner: "example", repo: "project", run_id: 17, filter: "latest", per_page: 100 });
          return jobs;
        },
      },
    },
  };
}

const skipped = [{ id: 21, name: "test_macos", conclusion: "skipped", check_run_url: "https://api.github.com/repos/example/project/check-runs/211" },
  { id: 22, name: "coverage", conclusion: "skipped", check_run_url: "https://api.github.com/repos/example/project/check-runs/222" }];

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
