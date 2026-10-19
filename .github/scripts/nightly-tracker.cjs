'use strict';

const MARKER = '<!-- nightly-full-ci:v1 -->';
const END = '<!-- nightly-full-ci:end -->';
const OID = /^[0-9a-f]{40}$/;

async function trackNightly({ github, context, core, result }) {
  if (context.ref !== 'refs/heads/main' ||
      !['schedule', 'workflow_dispatch'].includes(context.eventName) || !OID.test(context.sha)) {
    throw new Error('Nightly tracking requires a full main run');
  }
  const repo = context.repo;
  const fullName = `${repo.owner}/${repo.repo}`;
  const url = `https://github.com/${fullName}`;
  const issues = await github.paginate(github.rest.issues.listForRepo, {
    ...repo, state: 'all', per_page: 100,
  });
  const matches = issues.filter(issue => !issue.pull_request && (issue.body || '').includes(MARKER));
  if (matches.length > 1) throw new Error('Duplicate nightly trackers; reconcile before retrying');
  const issue = matches[0];
  if (issue && (issue.user?.login !== 'github-actions[bot]' || !issue.body.includes(END))) {
    throw new Error('Nightly tracker ownership or section is invalid');
  }
  if (result === 'success' && !issue) return;
  const { data: current } = await github.rest.actions.getWorkflowRun({ ...repo, run_id: context.runId });
  if (current.head_sha !== context.sha || !Number.isFinite(Date.parse(current.created_at))) {
    throw new Error('Nightly run commit or timestamp is invalid');
  }
  const runs = await github.paginate(github.rest.actions.listWorkflowRuns, {
    ...repo, workflow_id: 'ci.yml', branch: 'main', event: 'schedule', per_page: 100,
  });
  const baseline = runs.filter(run => run.id !== context.runId && run.event === 'schedule' &&
    run.status === 'completed' && run.conclusion === 'success' && run.head_branch === 'main' &&
    run.repository?.full_name === fullName && OID.test(run.head_sha || '') &&
    Date.parse(run.created_at) < Date.parse(current.created_at))
    .sort((a, b) => Date.parse(b.created_at) - Date.parse(a.created_at))[0];
  const section = [MARKER, '## Nightly full validation', '',
    `- Status: ${result === 'success' ? 'recovered' : 'failed'} (${result})`,
    `- Commit: [${context.sha}](${url}/commit/${context.sha})`,
    `- Run: ${url}/actions/runs/${context.runId}`,
    baseline ? `- Since last green nightly: [${baseline.head_sha}...${context.sha}](${url}/compare/${baseline.head_sha}...${context.sha})`
      : '- No previous green nightly is available; inspect the failed commit and establish a green baseline.',
    '', 'Inspect the first failure and retained artifacts. Bisect the range, repair or revert through a PR,',
    'and run full CI on main again. If scope selection missed a dependency, add a selector regression.',
    'Releases remain blocked until full validation succeeds on their exact tagged commit.', END].join('\n');
  if (issue) {
    const start = issue.body.indexOf(MARKER);
    const end = issue.body.indexOf(END, start);
    if (end < start) throw new Error('Nightly tracker section is invalid');
    await github.rest.issues.update({ ...repo, issue_number: issue.number,
      state: result === 'success' ? 'closed' : 'open',
      body: issue.body.slice(0, start) + section + issue.body.slice(end + END.length) });
  } else {
    await github.rest.issues.create({ ...repo, title: 'ci: nightly full validation failure', body: section });
  }
  await core.summary.addRaw(section).write();
}

module.exports = { trackNightly, MARKER, END };
