'use strict';
const { test } = require('node:test');
const assert = require('node:assert/strict');
const { trackNightly, MARKER, END } = require('../../../.github/scripts/nightly-tracker.cjs');
function fixture() {
  const state = { issues: [], writes: [], runs: [], failed: [] };
  const listRuns = () => {};
  const listIssues = () => {};
  const github = {
    paginate: async (method, params) => {
      if (method === listRuns) {
        assert.equal(params.event, 'schedule');
        assert.equal(params.branch, 'main');
        assert.equal(params.workflow_id, 'ci.yml');
        return state.runs;
      }
      assert.equal(method, listIssues);
      assert.equal(params.state, 'all');
      return state.issues;
    },
    rest: {
      actions: { listWorkflowRuns: listRuns, getWorkflowRun: async () => ({ data: {
        id: 30, created_at: '2026-10-20T05:00:00Z', head_sha: 'b'.repeat(40),
      } }) },
      issues: {
        listForRepo: listIssues,
        create: async row => {
          state.writes.push(row);
          state.issues.push({ ...row, number: 7, state: 'open', user: { login: 'github-actions[bot]' } });
          return { data: { html_url: 'https://github.com/example/repo/issues/7' } };
        },
        update: async row => {
          state.writes.push(row);
          Object.assign(state.issues.find(i => i.number === row.issue_number), row);
        },
      },
    },
  };
  const core = { setFailed: message => state.failed.push(message),
    summary: { addRaw() { return this; }, async write() {} } };
  const context = { repo: { owner: 'example', repo: 'repo' }, runId: 30,
    sha: 'b'.repeat(40), ref: 'refs/heads/main', eventName: 'schedule' };
  return { state, options: { github, core, context, result: 'failure' } };
}
test('failure creates one record and includes the commit range since last green scheduled main', async () => {
  const { state, options } = fixture();
  state.runs = [{ id: 20, status: 'completed', conclusion: 'success', event: 'schedule',
    head_branch: 'main', head_sha: 'a'.repeat(40), created_at: '2026-10-19T05:00:00Z',
    repository: { full_name: 'example/repo' } }];
  await trackNightly(options);
  assert.equal(state.writes.length, 1);
  assert.match(state.writes[0].body, new RegExp(`compare/${'a'.repeat(40)}\\.\\.\\.${'b'.repeat(40)}`));
  assert.match(state.writes[0].body, /actions\/runs\/30/);
  await trackNightly(options);
  assert.equal(state.issues.length, 1);
  assert.equal(state.writes[1].issue_number, 7);
});
test('closed tracker is reopened; maintainer notes survive updates and recovery closes it', async () => {
  const { state, options } = fixture();
  state.issues = [{ number: 7, state: 'closed', user: { login: 'github-actions[bot]' },
    body: `${MARKER}\nold\n${END}\nMaintainer notes` }];
  await trackNightly(options);
  assert.equal(state.writes[0].state, 'open');
  assert.match(state.writes[0].body, /Maintainer notes$/);
  assert.match(state.writes[0].body, /No previous green nightly/);
  await trackNightly({ ...options, result: 'success' });
  assert.equal(state.writes[1].state, 'closed');
});
test('green nightly leaves a closed tracker unchanged across repeated runs', async () => {
  const { state, options } = fixture();
  state.issues = [{ number: 7, state: 'closed', user: { login: 'github-actions[bot]' },
    body: `${MARKER}\nrecovered\n${END}\nMaintainer notes` }];
  const original = state.issues[0].body;
  await trackNightly({ ...options, result: 'success' });
  await trackNightly({ ...options, result: 'success' });
  assert.deepEqual(state.writes, []);
  assert.equal(state.issues[0].body, original);
});
test('green nightly creates no issue', async () => {
  const { state, options } = fixture();
  await trackNightly({ ...options, result: 'success' });
  assert.deepEqual(state.writes, []);
});
for (const result of ['cancelled', 'skipped']) {
  test(`${result} validation is tracked as failure`, async () => {
    const { state, options } = fixture();
    await trackNightly({ ...options, result });
    assert.equal(state.issues.length, 1);
  });
}
test('duplicate markers, foreign ownership and API errors fail without duplicate writes', async () => {
  for (const kind of ['duplicate', 'foreign', 'api']) {
    const { state, options } = fixture();
    const record = { number: 7, user: { login: 'github-actions[bot]' }, body: `${MARKER}\n${END}` };
    state.issues = kind === 'duplicate' ? [record, { ...record, number: 8 }] : [record];
    if (kind === 'foreign') record.user.login = 'maintainer';
    if (kind === 'api') options.github.paginate = async () => { throw new Error('private diagnostic'); };
    await assert.rejects(trackNightly(options));
    assert.deepEqual(state.writes, []);
  }
});
test('future, foreign and non-schedule green runs cannot supply the range baseline', async () => {
  const { state, options } = fixture();
  const run = { id: 20, status: 'completed', conclusion: 'success', event: 'schedule',
    head_branch: 'main', head_sha: 'a'.repeat(40), created_at: '2026-10-21T05:00:00Z',
    repository: { full_name: 'example/repo' } };
  state.runs = [run, { ...run, event: 'workflow_dispatch', created_at: '2026-10-19T05:00:00Z' }, { ...run, repository: { full_name: 'other/repo' }, created_at: '2026-10-19T05:00:00Z' }];
  await trackNightly(options);
  assert.match(state.writes[0].body, /No previous green nightly/);
});
