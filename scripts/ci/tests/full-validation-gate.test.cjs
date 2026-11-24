'use strict';
const { test } = require('node:test');
const assert = require('node:assert/strict');
const { requireFullValidation, requireQueueValidation } = require('../../../.github/scripts/full-validation-gate.cjs');
const retryRun = require('./fixtures/queue-validation-retry-run.json');
const sha = 'a'.repeat(40);
const names = ['test', 'test_macos', 'coverage', 'test_containment', 'cargo_deny', 'macos_check'];
function fixture() {
  return Object.fromEntries(names.map(name => [name, { result: 'success' }]));
}
test('full exact-commit validation admits publication', () => {
  assert.equal(requireFullValidation({ expected: sha, actual: sha, needs: fixture() }), true);
});
for (const name of names) {
  for (const result of ['failure', 'cancelled', 'skipped', undefined]) {
    test(`${name}/${result} blocks asset publication and tap dispatch`, () => {
      const needs = fixture();
      needs[name] = { result };
      const writes = [];
      assert.throws(() => {
        requireFullValidation({ expected: sha, actual: sha, needs });
        writes.push('assets', 'tap');
      }, /full validation/i);
      assert.deepEqual(writes, []);
    });
  }
}
test('the isolation guard is required only when requested', () => {
  const needs = { ...fixture(), isolation_guard: { result: 'skipped' } };
  assert.equal(requireFullValidation({ expected: sha, actual: sha, needs }), true);
  assert.throws(() => requireFullValidation({ expected: sha, actual: sha, needs, isolationGuard: true }), /isolation_guard/);
  needs.isolation_guard.result = 'success';
  assert.equal(requireFullValidation({ expected: sha, actual: sha, needs, isolationGuard: true }), true);
});
test('a different or missing commit fails closed', () => {
  for (const actual of ['b'.repeat(40), '', undefined]) {
    assert.throws(() => requireFullValidation({ expected: sha, actual, needs: fixture() }), /commit/i);
  }
});

test('ordinary queue requires the strict macOS result without full Linux or coverage lanes', () => {
  const queue = { expected: sha, actual: sha, macosPartitionsOnly: true };
  assert.equal(requireFullValidation({
    ...queue, needs: { macos_check: { result: 'success' } },
  }), true);
  for (const result of ['failure', 'cancelled', 'skipped', undefined]) {
    const needs = { ...fixture(), macos_check: { result } };
    assert.throws(() => requireFullValidation({ ...queue, needs }), /macos_check/);
  }
  assert.throws(() => requireFullValidation({
    ...queue, actual: 'b'.repeat(40), needs: { macos_check: { result: 'success' } },
  }), /commit/i);
});

test('partitions alone cannot satisfy full validation', () => {
  assert.throws(() => requireFullValidation({
    expected: sha, actual: sha, needs: { test_macos_partition: { result: 'success' } },
  }), /test_macos, coverage/);
});

// Replay Actions run 45411646819, whose caller received an empty reusable output.
const retryJobs = retryRun.jobs.map(job => ({
  ...job, run_id: retryRun.runId, run_attempt: retryRun.attempt, head_sha: retryRun.sha,
}));
const retryGate = retryJobs.find(job => job.name === 'full_validation / gate');
const otherJobs = retryJobs.filter(job => job !== retryGate);

function replayQueueGate(jobs) {
  const context = { repo: { owner: 'owner', repo: 'repo' }, runId: retryRun.runId, sha: retryRun.sha };
  const listJobsForWorkflowRunAttempt = Symbol('listJobsForWorkflowRunAttempt');
  const github = {
    rest: { actions: { listJobsForWorkflowRunAttempt } },
    async paginate(method, query) {
      assert.equal(method, listJobsForWorkflowRunAttempt);
      assert.deepEqual(query, {
        ...context.repo, run_id: retryRun.runId, attempt_number: retryRun.attempt, per_page: 100,
      });
      return jobs;
    },
  };
  return requireQueueValidation({ github, context, attempt: retryRun.attempt });
}

test('the recorded successful retry validates the exact merge-group commit', async () => {
  assert.equal(await replayQueueGate(retryJobs), retryRun.sha);
});

for (const [description, gates] of [
  ['missing final gate', []],
  ['ambiguous final gates', [retryGate, retryGate]],
  ['failed final gate', [{ ...retryGate, conclusion: 'failure' }]],
  ['cancelled final gate', [{ ...retryGate, conclusion: 'cancelled' }]],
  ['unfinished final gate', [{ ...retryGate, status: 'in_progress', conclusion: null }]],
  ['wrong run', [{ ...retryGate, run_id: retryRun.runId + 1 }]],
  ['wrong attempt', [{ ...retryGate, run_attempt: retryRun.attempt + 1 }]],
  ['wrong commit', [{ ...retryGate, head_sha: 'b'.repeat(40) }]],
]) {
  test(`queue validation rejects ${description}`, async () => {
    await assert.rejects(replayQueueGate([...otherJobs, ...gates]), /required queue validation/i);
  });
}
