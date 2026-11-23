'use strict';

// GitHub's hosted acquisition failure is a cancellation, not an execution timeout.
const NOT_ACQUIRED = 'The job was not acquired by Runner of type hosted even after multiple attempts';

function unassignedMacosJob(job) {
  return /(?:^| \/ )(?:macos_clippy(?:_retry)?|test_macos_partition(?:_retry)? \([12]\/2\))$/.test(job.name || '') &&
    job.conclusion === 'cancelled' && Array.isArray(job.steps) && job.steps.length === 0 &&
    job.runner_id === 0 && job.runner_name === '' &&
    job.labels?.some(label => ['macos-latest', 'macos-15'].includes(label));
}

function capacityReason(jobs = []) {
  const names = jobs.filter(capacityMiss).map(job => job.name);
  return names.length ? `hosted runner capacity (not acquired): ${names.join(', ')}` : '';
}

function capacityMiss(job) {
  return unassignedMacosJob(job) && job.annotations?.some(annotation =>
    annotation.annotation_level === 'failure' && annotation.message === NOT_ACQUIRED);
}

async function collectMacosJobs({ github, context, expected }) {
  // Scope to this attempt so a recovered job cannot inherit an older capacity failure.
  const jobs = await github.paginate(github.rest.actions.listJobsForWorkflowRunAttempt, {
    ...context.repo, run_id: context.runId, attempt_number: Number(process.env.GITHUB_RUN_ATTEMPT),
    per_page: 100,
  });
  const candidates = [];
  for (const [name, label] of expected) {
    const matches = jobs.filter(job => job.name === `full_validation / ${name}`);
    const job = matches.length === 1 ? matches[0] : undefined;
    if (job?.status !== 'completed' || job.head_sha !== context.sha || !job.labels?.includes(label)) {
      candidates.push({ name });
      continue;
    }
    const check = /\/check-runs\/(\d+)$/.exec(job.check_run_url || '');
    const annotations = check && unassignedMacosJob(job) ? await github.paginate(github.rest.checks.listAnnotations, {
      ...context.repo, check_run_id: Number(check[1]), per_page: 100,
    }) : [];
    candidates.push({ ...job, annotations });
  }
  return candidates;
}

async function reportMacosCapacity({ github, context, core, clippy, partitions, retriesUsed = 0, allowRetry = true }) {
  const suffix = retriesUsed === 0 ? '' : '_retry';
  const clippyName = `macos_clippy${suffix}`;
  const partitionName = partition => `test_macos_partition${suffix} (${partition}/2)`;
  const expected = partitions.map(partition => [partitionName(partition), 'macos-15']);
  if (clippy) expected.push([clippyName, 'macos-latest']);
  if (!expected.length) throw new Error('Missing required macOS jobs');

  const jobs = await collectMacosJobs({ github, context, expected });
  const failed = jobs.filter(job => job.conclusion !== 'success');
  const capacity = failed.filter(capacityMiss);
  const reason = capacityReason(capacity);
  const capacityOnly = capacity.length > 0 && capacity.length === failed.length;
  const retry = allowRetry && retriesUsed === 0 && capacityOnly;
  const missed = name => capacity.some(job => job.name === `full_validation / ${name}`);
  core.setOutput('retry_clippy', retry && missed(clippyName));
  core.setOutput('retry_partitions', retry ? partitions.filter(partition => missed(partitionName(partition))) : []);

  let outcome = 'Required macOS jobs passed.';
  if (retry) {
    outcome = `${reason}. Retrying the unacquired jobs.`;
  } else if (failed.length) {
    const failure = capacityOnly && retriesUsed > 0
      ? 'capacity, retries exhausted'
      : 'required macOS jobs did not pass';
    outcome = reason ? `${failure}. ${reason}.` : `${failure}.`;
    core.error(outcome);
  }
  await core.summary.addRaw(`macOS runner acquisition limit: 1 retry. ${outcome}\n`).write();
  return failed.length === 0;
}

async function requireMacosValidation({ github, context, core, needs, partitionsRequired }) {
  const checkClippy = context.eventName === 'merge_group';
  const clippyMiss = checkClippy && needs.macos_clippy.result !== 'success';
  const partitionMiss = partitionsRequired && needs.test_macos_partition.result !== 'success';
  if (!clippyMiss && !partitionMiss) return;

  const plan = needs.macos_retry_plan;
  const clippy = plan.outputs.retry_clippy === 'true';
  const partitions = JSON.parse(plan.outputs.retry_partitions || '[]');
  if (plan.result === 'success' && clippy === clippyMiss && (partitions.length > 0) === partitionMiss) {
    const passed = await reportMacosCapacity({ github, context, core, clippy, partitions, retriesUsed: 1 });
    if (passed && (!clippy || needs.macos_clippy_retry.result === 'success') &&
        (!partitions.length || needs.test_macos_partition_retry.result === 'success')) return;
  }
  throw new Error('Required macOS validation did not succeed');
}

module.exports = { capacityReason, reportMacosCapacity, requireMacosValidation };
