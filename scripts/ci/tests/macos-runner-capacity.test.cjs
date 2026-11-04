'use strict';
const { test } = require('node:test');
const assert = require('node:assert/strict');
const { requireFullValidation } = require('../../../.github/scripts/full-validation-gate.cjs');
const { capacityReason, reportMacosCapacity } = require('../../../.github/scripts/macos-runner-capacity.cjs');

const message = 'The job was not acquired by Runner of type hosted even after multiple attempts';
function unacquired() {
  return {
    name: 'full_validation / test_macos_partition (2/2)',
    conclusion: 'cancelled', steps: [], runner_id: 0, runner_name: '',
    labels: ['macos-15'],
    annotations: [{ annotation_level: 'failure', message }],
  };
}
const sha = 'a'.repeat(40);
const needs = Object.fromEntries(['test', 'test_macos', 'coverage', 'test_containment', 'cargo_deny']
  .map(name => [name, { result: name === 'test_macos' ? 'skipped' : 'success' }]));

test('the full gate rejects unexecuted macOS validation with a capacity reason', () => {
  assert.throws(() => requireFullValidation({ expected: sha, actual: sha, needs,
    macosJobs: [unacquired()] }), /hosted runner capacity \(not acquired\).*test_macos_partition/);
});

test('capacity evidence never admits a failed, cancelled or timed-out lane', () => {
  for (const result of ['failure', 'cancelled', 'timed_out', 'skipped']) {
    assert.throws(() => requireFullValidation({ expected: sha, actual: sha,
      needs: { ...needs, test_macos: { result } }, macosJobs: [unacquired()] }), /Full validation did not succeed/);
  }
});

test('only the complete hosted acquisition signature is capacity', () => {
  for (const change of [
    { conclusion: 'failure' }, { conclusion: 'timed_out' }, { conclusion: 'success' },
    { steps: [{ name: 'Set up job' }] }, { steps: undefined },
    { runner_id: 12 }, { runner_id: undefined }, { runner_name: 'assigned-runner' },
    { annotations: [] }, { annotations: [{ annotation_level: 'notice', message }] },
    { annotations: [{ annotation_level: 'failure', message: 'The operation was canceled.' }] },
    { labels: ['ubuntu-latest'] }, { name: 'test' },
  ]) {
    assert.equal(capacityReason([{ ...unacquired(), ...change }]), '', JSON.stringify(change));
  }
  assert.match(capacityReason([{ ...unacquired(), name: 'macos_clippy', labels: ['macos-latest'] }]),
    /hosted runner capacity \(not acquired\): macos_clippy/);
});

function apiFixture() {
  const calls = [];
  const errors = [];
  const outputs = {};
  const acquired = { ...unacquired(), runner_id: 12, check_run_url: 'https://api.github.com/repos/example/project/check-runs/2' };
  const github = {
    rest: { actions: { listJobsForWorkflowRunAttempt: 'jobs' }, checks: { listAnnotations: 'annotations' } },
    paginate: async (route, args) => {
      calls.push({ route, args });
      if (route === 'jobs') return [{ ...unacquired(), check_run_url: 'https://api.github.com/repos/example/project/check-runs/1' }, acquired];
      return unacquired().annotations;
    },
  };
  return { github, context: { repo: { owner: 'example', repo: 'project' }, runId: 123 },
    core: { error: reason => errors.push(reason), setOutput: (key, value) => { outputs[key] = value; } },
    calls, errors, outputs };
}

test('reporter reads the current attempt and annotates capacity without setting success', async () => {
  const fixture = apiFixture();
  const oldAttempt = process.env.GITHUB_RUN_ATTEMPT;
  process.env.GITHUB_RUN_ATTEMPT = '2';
  try {
    await reportMacosCapacity(fixture);
  } finally {
    if (oldAttempt === undefined) delete process.env.GITHUB_RUN_ATTEMPT;
    else process.env.GITHUB_RUN_ATTEMPT = oldAttempt;
  }
  assert.equal(fixture.calls[0].args.attempt_number, 2);
  assert.equal(fixture.calls[0].args.run_id, 123);
  assert.deepEqual(fixture.calls.map(call => call.route), ['jobs', 'annotations']);
  assert.equal(fixture.calls[1].args.check_run_id, 1);
  assert.match(fixture.errors[0], /hosted runner capacity \(not acquired\)/);
  assert.equal(JSON.parse(fixture.outputs.jobs).length, 1);
});

test('API failures cannot manufacture a capacity classification', async () => {
  const fixture = apiFixture();
  fixture.github.paginate = async () => { throw new Error('provider unavailable'); };
  await assert.rejects(reportMacosCapacity(fixture), /provider unavailable/);
  assert.deepEqual(fixture.errors, []);
  assert.deepEqual(fixture.outputs, {});
});
