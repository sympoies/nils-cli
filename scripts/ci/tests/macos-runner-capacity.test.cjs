'use strict';
const { test } = require('node:test');
const assert = require('node:assert/strict');
const { capacityReason, reportMacosCapacity, requireMacosValidation } = require('../../../.github/scripts/macos-runner-capacity.cjs');

const message = 'The job was not acquired by Runner of type hosted even after multiple attempts';
const sha = 'a'.repeat(40);
function unacquired() {
  return {
    name: 'full_validation / test_macos_partition (2/2)',
    conclusion: 'cancelled', steps: [], runner_id: 0, runner_name: '',
    labels: ['macos-15'], status: 'completed', head_sha: sha,
    check_run_url: 'https://api.github.com/repos/example/project/check-runs/1',
    annotations: [{ annotation_level: 'failure', message }],
  };
}

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

function apiFixture(jobs = [unacquired()]) {
  const calls = [];
  const errors = [];
  const outputs = {};
  const github = {
    rest: { actions: { listJobsForWorkflowRunAttempt: 'jobs' }, checks: { listAnnotations: 'annotations' } },
    paginate: async (route, args) => {
      calls.push({ route, args });
      if (route === 'jobs') return jobs;
      return unacquired().annotations;
    },
  };
  const fixture = { github, context: { repo: { owner: 'example', repo: 'project' }, runId: 123, sha },
    core: { error: reason => errors.push(reason), setOutput: (key, value) => { outputs[key] = value; } },
    calls, errors, outputs, summary: '' };
  fixture.core.summary = {
    addRaw: text => { fixture.summary += text; return fixture.core.summary; },
    write: async () => {},
  };
  return fixture;
}

test('reporter reads the current attempt and annotates capacity without setting success', async () => {
  const fixture = apiFixture();
  const oldAttempt = process.env.GITHUB_RUN_ATTEMPT;
  process.env.GITHUB_RUN_ATTEMPT = '2';
  try {
    await reportMacosCapacity({ ...fixture, partitions: [2], allowRetry: false });
  } finally {
    if (oldAttempt === undefined) delete process.env.GITHUB_RUN_ATTEMPT;
    else process.env.GITHUB_RUN_ATTEMPT = oldAttempt;
  }
  assert.equal(fixture.calls[0].args.attempt_number, 2);
  assert.equal(fixture.calls[0].args.run_id, 123);
  assert.deepEqual(fixture.calls.map(call => call.route), ['jobs', 'annotations']);
  assert.equal(fixture.calls[1].args.check_run_id, 1);
  assert.match(fixture.errors[0], /hosted runner capacity \(not acquired\)/);
  assert.deepEqual(fixture.outputs.retry_partitions, []);
});

test('API failures cannot manufacture a capacity classification', async () => {
  const fixture = apiFixture();
  fixture.github.paginate = async () => { throw new Error('provider unavailable'); };
  await assert.rejects(reportMacosCapacity({ ...fixture, partitions: [2] }), /provider unavailable/);
  assert.deepEqual(fixture.errors, []);
  assert.deepEqual(fixture.outputs, {});
});

const clippy = { ...unacquired(), name: 'full_validation / macos_clippy',
  conclusion: 'success', labels: ['macos-latest'] };

test('only the capacity-missed partition is scheduled for one retry', async () => {
  const missed = unacquired();
  const passed = { ...missed, name: 'full_validation / test_macos_partition (1/2)', conclusion: 'success' };
  const fixture = apiFixture([clippy, passed, missed]);
  await reportMacosCapacity({ ...fixture, clippy: true, partitions: [1, 2] });
  assert.deepEqual(fixture.outputs.retry_partitions, [2]);
  assert.equal(fixture.outputs.retry_clippy, false);
  assert.match(fixture.summary, /1 retry/);
  const clippyOnly = apiFixture([{ ...clippy, conclusion: 'cancelled' }]);
  await reportMacosCapacity({ ...clippyOnly, clippy: true, partitions: [] });
  assert.equal(clippyOnly.outputs.retry_clippy, true);
});

test('a Mac failure, a mixed run, and missing evidence never schedule retries', async () => {
  const missed = unacquired();
  const failed = { ...clippy, conclusion: 'failure' };
  for (const [jobs, partitions] of [
    [[failed], []],
    [[failed, missed], [2]],
    [[missed], [2]],
    [[clippy, missed, missed], [2]],
    [[clippy, { ...missed, head_sha: 'b'.repeat(40) }], [2]],
  ]) {
    const fixture = apiFixture(jobs);
    await reportMacosCapacity({ ...fixture, clippy: true, partitions });
    assert.deepEqual(fixture.outputs.retry_partitions, []);
    assert.equal(fixture.outputs.retry_clippy, false);
  }
});

test('a second capacity miss fails with an exhausted retry summary', async () => {
  const missed = { ...unacquired(), name: 'full_validation / test_macos_partition_retry (2/2)' };
  const fixture = apiFixture([missed]);
  const needs = {
    test_macos_partition: { result: 'cancelled' },
    macos_retry_plan: { result: 'success', outputs: { retry_clippy: 'false', retry_partitions: '[2]' } },
    test_macos_partition_retry: { result: 'cancelled' },
  };
  await assert.rejects(requireMacosValidation({ ...fixture, needs, partitionsRequired: true }),
    /Required macOS validation did not succeed/);
  assert.match(fixture.summary, /capacity, retries exhausted/);

  needs.test_macos_partition_retry.result = 'success';
  const recovered = apiFixture([{ ...missed, conclusion: 'success' }]);
  await requireMacosValidation({ ...recovered, needs, partitionsRequired: true });
  assert.match(recovered.summary, /Required macOS jobs passed/);
});
