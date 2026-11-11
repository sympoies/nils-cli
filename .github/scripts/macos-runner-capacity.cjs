'use strict';

// GitHub's hosted acquisition failure is a cancellation, not an execution timeout.
const NOT_ACQUIRED = 'The job was not acquired by Runner of type hosted even after multiple attempts';

function unassignedMacosJob(job) {
  return /(?:^| \/ )(?:macos_clippy|test_macos_partition \([12]\/2\))$/.test(job.name || '') &&
    job.conclusion === 'cancelled' && Array.isArray(job.steps) && job.steps.length === 0 &&
    job.runner_id === 0 && job.runner_name === '' &&
    job.labels?.some(label => ['macos-latest', 'macos-15'].includes(label));
}

function capacityReason(jobs = []) {
  const names = jobs.filter(job => unassignedMacosJob(job) &&
    job.annotations?.some(annotation => annotation.annotation_level === 'failure' &&
      annotation.message === NOT_ACQUIRED)).map(job => job.name);
  return names.length ? `hosted runner capacity (not acquired): ${names.join(', ')}` : '';
}

async function collectMacosJobs({ github, context }) {
  // Scope to this attempt so a recovered job cannot inherit an older capacity failure.
  const jobs = await github.paginate(github.rest.actions.listJobsForWorkflowRunAttempt, {
    ...context.repo, run_id: context.runId, attempt_number: Number(process.env.GITHUB_RUN_ATTEMPT),
    per_page: 100,
  });
  const candidates = [];
  for (const job of jobs.filter(unassignedMacosJob)) {
    const check = /\/check-runs\/(\d+)$/.exec(job.check_run_url || '');
    if (!check) continue;
    const annotations = await github.paginate(github.rest.checks.listAnnotations, {
      ...context.repo, check_run_id: Number(check[1]), per_page: 100,
    });
    candidates.push({ ...job, annotations });
  }
  return candidates;
}

async function reportMacosCapacity({ github, context, core }) {
  const jobs = await collectMacosJobs({ github, context });
  const reason = capacityReason(jobs);
  if (reason) core.error(reason);
  core.setOutput('jobs', JSON.stringify(jobs));
}

module.exports = { capacityReason, collectMacosJobs, reportMacosCapacity };
