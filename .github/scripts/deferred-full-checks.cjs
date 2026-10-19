"use strict";

const { setTimeout: sleep } = require("node:timers/promises");

const MAX_ATTEMPTS = 25;
const RETRY_DELAY_MS = 5000;
const REQUIRED_JOBS = ["test_macos", "coverage"];
const TITLE = "deferred-full: runs in merge group";

async function labelDeferredChecks({ github, context, core, sleep: wait = sleep }) {
  const repo = { owner: context.repo.owner, repo: context.repo.repo };
  let jobs;
  for (let attempt = 1; attempt <= MAX_ATTEMPTS; attempt++) {
    const listed = await github.paginate(github.rest.actions.listJobsForWorkflowRun, {
      ...repo, run_id: context.runId, filter: "all", per_page: 100,
    });
    jobs = REQUIRED_JOBS.map((name) => {
      const matches = listed.filter((job) => job.name === name);
      // A labeler-only rerun has no new sibling jobs. Keep each sibling's
      // newest attempt, including a pending attempt superseding an older skip.
      const latestAttempt = Math.max(...matches.map((job) => job.run_attempt));
      const latest = matches.filter((job) => job.run_attempt === latestAttempt);
      if (latest.length > 1 || (latest[0]?.status === "completed" && latest[0].conclusion !== "skipped")) {
        throw new Error(`Expected one completed skipped ${name} job`);
      }
      return latest[0];
    });
    const pending = jobs.findIndex((job) => job?.status !== "completed");
    if (pending === -1) break;
    if (attempt === MAX_ATTEMPTS) {
      throw new Error(`Expected one completed skipped ${REQUIRED_JOBS[pending]} job after ${MAX_ATTEMPTS} attempts`);
    }
    await wait(RETRY_DELAY_MS);
  }
  const targets = jobs.map((job) => {
    const match = new URL(job.check_run_url).pathname.match(/\/check-runs\/([1-9]\d*)$/);
    const checkRunId = Number(match?.[1]);
    if (!Number.isSafeInteger(checkRunId)) throw new Error("Missing required check run identity");
    return { checkRunId };
  });
  for (const job of targets) {
    await github.rest.checks.update({
      ...repo, check_run_id: job.checkRunId,
      output: {
        title: TITLE,
        summary: "Full macOS tests and coverage did not run on this PR. The required merge-group checks must run the full workspace and enforce the 85% coverage floor before main advances.",
      },
    });
  }
  await core.summary.addHeading(TITLE).addRaw("The required test_macos and coverage checks are skipped, not full-test passes.").write();
}

module.exports = { TITLE, labelDeferredChecks };
