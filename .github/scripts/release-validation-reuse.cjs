'use strict';

const SHA = /^[0-9a-f]{40}$/;
const FULL_JOBS = Object.freeze([
  'test', 'test_macos', 'coverage', 'test_containment',
  'cargo-deny (advisories + bans)', 'gate',
]);
const MARKED_JOBS = new Set(['test', 'test_macos', 'coverage']);

function successful(row) {
  return row?.status === 'completed' && row.conclusion === 'success';
}

async function fullJobsSucceeded(github, params) {
  const method = github.rest.actions.listJobsForWorkflowRun;
  const query = { ...params, filter: 'latest', per_page: 100 };
  const jobs = typeof github.paginate === 'function'
    ? await github.paginate(method, query)
    : (await method(query)).data.jobs;
  return FULL_JOBS.every(name => {
    const matches = jobs.filter(job => job.name === `full_validation / ${name}`);
    if (matches.length !== 1 || !successful(matches[0])) return false;
    if (!MARKED_JOBS.has(name)) return true;
    const markers = (matches[0].steps || []).filter(step => step.name === 'Full validation marker');
    return markers.length === 1 && successful(markers[0]);
  });
}

// Only the CI merge-group caller can supply reusable evidence. PR leaf jobs
// may be reduced or affected-only, and check-run names alone prove no provenance.
// Bound the identical-tree search to one recent page; a miss simply runs tests.
async function findReusableValidation({ github, context, core }) {
  if (!SHA.test(context.sha || '')) return null;
  const params = { owner: context.repo.owner, repo: context.repo.repo };
  const fullName = `${params.owner}/${params.repo}`;
  try {
    const query = { ...params, workflow_id: 'ci.yml', event: 'merge_group', per_page: 100 };
    const exact = await github.rest.actions.listWorkflowRuns({ ...query, head_sha: context.sha });
    let releasedTree;
    const seen = new Set();
    const candidates = [...exact.data.workflow_runs];
    let searchedRecent = false;
    for (let index = 0; ; index += 1) {
      if (index === candidates.length) {
        if (searchedRecent) break;
        candidates.push(...(await github.rest.actions.listWorkflowRuns(query)).data.workflow_runs);
        searchedRecent = true;
        if (index === candidates.length) break;
      }
      const run = candidates[index];
      if (seen.has(run.id)) continue;
      seen.add(run.id);
      if (!successful(run) || run.event !== 'merge_group' || !SHA.test(run.head_sha || '') ||
          run.repository?.full_name !== fullName || run.head_repository?.full_name !== fullName) continue;
      let tree;
      if (run.head_sha !== context.sha) {
        if (!releasedTree) {
          const commit = (await github.rest.repos.getCommit({ ...params, ref: context.sha })).data;
          if (commit?.sha !== context.sha || !SHA.test(commit?.commit?.tree?.sha || '')) return null;
          releasedTree = commit.commit.tree.sha;
        }
        const commit = (await github.rest.repos.getCommit({ ...params, ref: run.head_sha })).data;
        tree = commit?.commit?.tree?.sha;
        if (commit?.sha !== run.head_sha || !SHA.test(tree || '') || tree !== releasedTree) continue;
      }
      if (!await fullJobsSucceeded(github, { ...params, run_id: run.id })) continue;
      core.info(`Reusing full validation from merge-group run ${run.id} on ${run.head_sha}: ${run.html_url}`);
      return { runId: run.id, runUrl: run.html_url, sha: run.head_sha, tree };
    }
  } catch (error) {
    core.warning(`Could not verify reusable full validation; running tag validation: ${error.message}`);
  }
  core.info('No reusable full validation found; running tag validation.');
  return null;
}

function requireReleaseValidation({ authorization, reused, fallback }) {
  if (authorization !== 'success') throw new Error('Release authorization did not succeed');
  if (!(reused === 'true' ? fallback === 'skipped' : fallback === 'success')) {
    throw new Error('Release requires reusable full validation or successful tag validation');
  }
  return true;
}

module.exports = { findReusableValidation, requireReleaseValidation };
