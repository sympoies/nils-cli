'use strict';
const { test } = require('node:test');
const assert = require('node:assert/strict');
const { readFileSync } = require('node:fs');
const path = require('node:path');
const gate = require('../../../.github/scripts/full-validation-gate.cjs');
const { requireFullValidation, requireValidatedCommit } = gate;
const retryRun = require('./fixtures/queue-validation-retry-run.json');
const nightlyRetryRun = require('./fixtures/nightly-validation-retry-run.json');
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
  return requireValidatedCommit(replayContext(retryRun, jobs, 'merge_group'));
}

function replayContext(run, jobs, eventName) {
  const { runId, sha, attempt } = run;
  const context = { repo: { owner: 'owner', repo: 'repo' }, runId, sha, eventName };
  const listJobsForWorkflowRunAttempt = Symbol('listJobsForWorkflowRunAttempt');
  const github = {
    rest: { actions: { listJobsForWorkflowRunAttempt } },
    async paginate(method, query) {
      assert.equal(method, listJobsForWorkflowRunAttempt);
      assert.deepEqual(query, {
        ...context.repo, run_id: runId, attempt_number: attempt, per_page: 100,
      });
      return jobs;
    },
  };
  return { github, context, attempt };
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
    await assert.rejects(replayQueueGate([...otherJobs, ...gates]), /full validation/i);
  });
}

const ci = readFileSync(path.join(__dirname, '../../../.github/workflows/ci.yml'), 'utf8');
const AsyncFunction = Object.getPrototypeOf(async function () {}).constructor;

function workflowScript(job, step) {
  const body = ci.split(`\n  ${job}:\n`)[1].split(/\n  [a-z_]+:\n/)[0];
  const script = body.split(`      - name: ${step}\n`)[1]?.match(/          script: \|\n((?: {12}[^\n]*\n)+)/)?.[1];
  assert.ok(script, `missing script for ${job} / ${step}`);
  return new AsyncFunction('github', 'context', 'core', 'process', 'require', script);
}

for (const event of ['schedule', 'workflow_dispatch']) {
  for (const conclusion of ['success', 'failure']) {
    const jobs = nightlyRetryRun.jobs.map(job => ({
      ...job,
      run_id: nightlyRetryRun.runId,
      run_attempt: nightlyRetryRun.attempt,
      head_sha: nightlyRetryRun.sha,
      conclusion: job.name === 'full_validation / gate' ? conclusion : job.conclusion,
    }));
    const { github, context, attempt } = replayContext(nightlyRetryRun, jobs, event);
    const process = { env: { GITHUB_RUN_ATTEMPT: String(attempt) } };
    test(`${event} reports ${conclusion} after the macOS retry`, async () => {
      const update = workflowScript('nightly_report', 'Update nightly tracking issue');
      const results = [];
      const modules = {
        './.github/scripts/nightly-tracker.cjs': { trackNightly: async ({ result }) => results.push(result) },
        './.github/scripts/full-validation-gate.cjs': gate,
        'node:fs': { existsSync: () => false },
      };
      await update(github, context, { warning() {} }, process, name => modules[name]);
      assert.deepEqual(results, [conclusion]);
    });
    test(`${event} badge proof for a ${conclusion} final gate`, async () => {
      const verify = workflowScript('coverage_badge', 'Require exact-commit validation');
      const run = verify(github, context, {}, process, () => gate);
      if (conclusion === 'success') {
        assert.equal(await run, nightlyRetryRun.sha);
      } else {
        await assert.rejects(run, /validation/i);
      }
    });
  }
}
