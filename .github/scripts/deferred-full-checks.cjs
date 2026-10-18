"use strict";

const TITLE = "deferred-full: runs in merge group";

async function labelDeferredChecks({ github, context, core }) {
  const repo = { owner: context.repo.owner, repo: context.repo.repo };
  const jobs = await github.paginate(github.rest.actions.listJobsForWorkflowRun, {
    ...repo, run_id: context.runId, filter: "latest", per_page: 100,
  });
  const targets = ["test_macos", "coverage"].map((name) => {
    const matches = jobs.filter((job) => job.name === name);
    if (matches.length !== 1 || matches[0].conclusion !== "skipped") {
      throw new Error(`Expected one completed skipped ${name} job`);
    }
    const job = matches[0];
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
