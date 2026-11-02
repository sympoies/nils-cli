#!/usr/bin/env node

const assert = require("node:assert/strict");
const test = require("node:test");

// Release reuse must prove the full reusable workflow, never a scoped job with
// the same leaf name. This suite stays in the existing required CI entrypoint.
const {
  findReusableValidation,
  requireReleaseValidation,
} = require('../../../.github/scripts/release-validation-reuse.cjs');

const reuseSha = '1'.repeat(40);
const queueSha = '2'.repeat(40);
const reuseTree = '3'.repeat(40);
const reuseRepo = { owner: 'example', repo: 'cli' };
const reuseContext = { repo: reuseRepo, sha: reuseSha };
const fullJobNames = ['test', 'test_macos', 'coverage', 'test_containment', 'cargo-deny (advisories + bans)', 'gate'];
function reuseFixture() {
  const state = {
    runs: [{
      id: 42, event: 'merge_group', status: 'completed', conclusion: 'success',
      head_sha: reuseSha, repository: { full_name: 'example/cli' },
      head_repository: { full_name: 'example/cli' },
      html_url: 'https://github.com/example/cli/actions/runs/42',
    }],
    jobs: fullJobNames.map(name => ({
      name: `full_validation / ${name}`, status: 'completed', conclusion: 'success',
      steps: [{ name: 'Full validation marker', status: 'completed', conclusion: 'success' }],
    })),
    commits: Object.fromEntries([reuseSha, queueSha].map(sha =>
      [sha, { sha, commit: { tree: { sha: reuseTree } } }])),
    queries: [],
  };
  const github = { rest: {
    actions: {
      listWorkflowRuns: async params => {
        state.queries.push(params);
        return { data: { workflow_runs: state.runs.filter(run => !params.head_sha || run.head_sha === params.head_sha) } };
      },
      listJobsForWorkflowRun: async () => ({ data: { jobs: state.jobs } }),
    },
    repos: { getCommit: async ({ ref }) => ({ data: state.commits[ref] }) },
  } };
  const logs = [];
  const core = { info: message => logs.push(message), warning: message => logs.push(message) };
  return { state, github, core, logs };
}

test('release reuses full Linux/macOS/coverage/containment/dependency evidence on its exact SHA', async () => {
  const f = reuseFixture();
  const receipt = await findReusableValidation({ ...f, context: reuseContext });
  assert.equal(receipt.runId, 42);
  assert.equal(receipt.sha, reuseSha);
  assert.equal(f.state.queries[0].workflow_id, 'ci.yml');
  assert.equal(f.state.queries[0].event, 'merge_group');
  assert.equal(f.state.queries[0].head_sha, reuseSha);
});

test('release reuses a queue commit only after verifying its identical tree', async () => {
  const f = reuseFixture();
  f.state.runs[0].head_sha = queueSha;
  const receipt = await findReusableValidation({ ...f, context: reuseContext });
  assert.equal(receipt.sha, queueSha);
  assert.equal(receipt.tree, reuseTree);
});

for (const [label, mutate] of Object.entries({
  'missing macOS': s => { s.jobs = s.jobs.filter(j => j.name !== 'full_validation / test_macos'); },
  'skipped Linux': s => { s.jobs[0].conclusion = 'skipped'; },
  'failed macOS': s => { s.jobs[1].conclusion = 'failure'; },
  'cancelled coverage': s => { s.jobs[2].conclusion = 'cancelled'; },
  'missing containment': s => { s.jobs.splice(3, 1); },
  'missing dependency policy': s => { s.jobs.splice(4, 1); },
  'missing gate': s => { s.jobs.pop(); },
  'scoped Linux': s => { s.jobs[0].steps = []; },
  'scoped macOS': s => { s.jobs[1].steps = []; },
  'missing coverage marker': s => { s.jobs[2].steps = []; },
  'failed full gate': s => { s.jobs[5].conclusion = 'failure'; },
  'pending full gate': s => { s.jobs[5].status = 'in_progress'; },
  'scoped jobs with green names': s => { s.jobs.forEach(j => { j.name = j.name.split(' / ')[1]; }); },
  'duplicate Linux jobs': s => { s.jobs.push({ ...s.jobs[0] }); },
  'pending run': s => { s.runs[0].status = 'in_progress'; },
  'failed run': s => { s.runs[0].conclusion = 'failure'; },
  'pull request run': s => { s.runs[0].event = 'pull_request'; },
  'foreign repository': s => { s.runs[0].repository.full_name = 'other/cli'; },
  'foreign head repository': s => { s.runs[0].head_repository.full_name = 'other/cli'; },
  'different tree': s => { s.runs[0].head_sha = queueSha; s.commits[queueSha].commit.tree.sha = '4'.repeat(40); },
  'unverifiable tree': s => { s.runs[0].head_sha = queueSha; delete s.commits[queueSha]; },
  'mismatched commit response': s => { s.runs[0].head_sha = queueSha; s.commits[queueSha].sha = reuseSha; },
  'malformed tree': s => { s.runs[0].head_sha = queueSha; s.commits[queueSha].commit.tree.sha = ''; },
  'no runs': s => { s.runs = []; },
})) {
  test(`release runs fallback validation for ${label}`, async () => {
    const f = reuseFixture();
    mutate(f.state);
    assert.equal(await findReusableValidation({ ...f, context: reuseContext }), null);
  });
}

test('API failure runs fallback validation instead of authorizing reuse', async () => {
  const f = reuseFixture();
  f.github.rest.actions.listWorkflowRuns = async () => { throw new Error('unavailable'); };
  assert.equal(await findReusableValidation({ ...f, context: reuseContext }), null);
  assert.ok(f.logs.some(message => message.includes('unavailable')));
});

test('pagination includes required jobs beyond the first page', async () => {
  const f = reuseFixture();
  f.github.rest.actions.listJobsForWorkflowRun = async ({ page = 1 }) => ({
    data: { jobs: f.state.jobs.slice((page - 1) * 3, page * 3) },
  });
  f.github.paginate = async (method, params) => {
    const first = await method({ ...params, page: 1 });
    const second = await method({ ...params, page: 2 });
    return [...first.data.jobs, ...second.data.jobs];
  };
  assert.equal((await findReusableValidation({ ...f, context: reuseContext })).runId, 42);
});

test('a reused receipt or successful fallback is required, after authorization', () => {
  for (const authorization of ['success', 'failure', 'cancelled', 'skipped', '']) {
    for (const reused of ['true', 'false', '', undefined]) {
      for (const fallback of ['success', 'failure', 'cancelled', 'skipped', '']) {
        const allowed = authorization === 'success' && (
          reused === 'true' ? fallback === 'skipped' : fallback === 'success');
        const check = () => requireReleaseValidation({ authorization, reused, fallback });
        if (allowed) assert.equal(check(), true);
        else assert.throws(check, /validation|authorization/i);
      }
    }
  }
});
