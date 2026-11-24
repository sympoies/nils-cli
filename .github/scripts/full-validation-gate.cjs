'use strict';

const SHA = /^[0-9a-f]{40}$/;
const REQUIRED = Object.freeze(['test', 'test_macos', 'coverage', 'test_containment', 'cargo_deny', 'macos_check']);

function requireFullValidation({ expected, actual, needs, isolationGuard = false, macosPartitionsOnly = false }) {
  if (!SHA.test(expected || '') || actual !== expected) {
    throw new Error('Full validation commit does not match the requested commit');
  }
  const required = macosPartitionsOnly ? ['macos_check'] : [...REQUIRED];
  if (isolationGuard) required.push('isolation_guard');
  const failed = required.filter(name => needs?.[name]?.result !== 'success');
  if (failed.length) {
    throw new Error(`Full validation did not succeed: ${failed.join(', ')}`);
  }
  return true;
}

async function requireQueueValidation({ github, context, attempt }) {
  const { runId, sha } = context;
  if (!SHA.test(sha || '') || !Number.isSafeInteger(runId) || runId < 1 ||
      !Number.isSafeInteger(attempt) || attempt < 1) {
    throw new Error('Required queue validation needs an exact run attempt and commit');
  }
  // A capacity cancellation can leave the final gate successful while its output
  // is empty in the calling workflow. Read the final job from this attempt.
  const jobs = await github.paginate(github.rest.actions.listJobsForWorkflowRunAttempt, {
    ...context.repo, run_id: runId, attempt_number: attempt, per_page: 100,
  });
  const gates = jobs.filter(job => job.name === 'full_validation / gate');
  const [gate] = gates;
  if (gates.length !== 1 || gate.run_id !== runId || gate.run_attempt !== attempt ||
      gate.head_sha !== sha || gate.status !== 'completed' || gate.conclusion !== 'success') {
    throw new Error('Required queue validation did not succeed for the exact run attempt and commit');
  }
  return gate.head_sha;
}

module.exports = { requireFullValidation, requireQueueValidation };
