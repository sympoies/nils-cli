'use strict';

const MARKER = '<!-- nightly-full-ci:v1 -->';
const END = '<!-- nightly-full-ci:end -->';
const OID = /^[0-9a-f]{40}$/;
const fs = require('node:fs');
const path = require('node:path');

// Lane budgets in minutes at the current 2 test threads (docs/plans/ci-time-growth-analysis.md).
const BUDGET_MINUTES = { Linux: 25, macOS: 40 };
const SLOW_SECONDS = 1;

function parseJunit(xml) {
  const root = /<testsuites\b[^>]*>/.exec(xml)?.[0] || '';
  const attr = name => Number(new RegExp(`\\b${name}="([^"]*)"`).exec(root)?.[1]);
  const tests = attr('tests');
  const wall = attr('time');
  if (!Number.isFinite(tests) || !Number.isFinite(wall)) throw new Error('JUnit report has no totals');
  const slow = [...xml.matchAll(/<testcase\b[^>]*\btime="([^"]*)"/g)]
    .filter(m => Number(m[1]) >= SLOW_SECONDS).length;
  return { tests, wall, slow };
}

function stepSeconds(job, startsWith) {
  const step = (job.steps || []).find(s => s.name.startsWith(startsWith));
  return step?.completed_at && step.started_at
    ? (Date.parse(step.completed_at) - Date.parse(step.started_at)) / 1000 : undefined;
}

// Reads nightly JUnit artifacts (junit-nextest*/junit.xml under junitDir) and job
// step timings; returns Markdown with per-platform figures and any budget breach.
async function timingReport({ github, context, junitDir }) {
  const reports = { Linux: [], macOS: [] };
  for (const entry of fs.readdirSync(junitDir, { withFileTypes: true })) {
    const file = path.join(junitDir, entry.name, 'junit.xml');
    if (!entry.isDirectory() || !entry.name.startsWith('junit-nextest') || !fs.existsSync(file)) continue;
    reports[entry.name.startsWith('junit-nextest-macos') ? 'macOS' : 'Linux']
      .push(parseJunit(fs.readFileSync(file, 'utf8')));
  }
  const jobs = await github.paginate(github.rest.actions.listJobsForWorkflowRun,
    { ...context.repo, run_id: context.runId, per_page: 100 });
  const gate = {
    Linux: jobs.filter(j => /(^|\/ )test$/.test(j.name)).map(j => stepSeconds(j, 'Nils CLI checks')),
    macOS: jobs.filter(j => /test_macos_partition/.test(j.name)).map(j => stepSeconds(j, 'Nextest partition')),
  };
  const rows = [];
  const breaches = [];
  for (const lane of ['Linux', 'macOS']) {
    const parts = reports[lane];
    if (!parts.length) { rows.push(`| ${lane} | n/a | n/a | n/a | n/a |`); continue; }
    // macOS partitions run in parallel: the lane takes as long as its slowest partition.
    const wall = Math.max(...parts.map(p => p.wall));
    const gates = gate[lane].filter(Number.isFinite);
    const build = gates.length ? `${(Math.max(0, Math.max(...gates) - wall) / 60).toFixed(1)} min` : 'n/a';
    rows.push(`| ${lane} | ${parts.reduce((n, p) => n + p.tests, 0)} | ${(wall / 60).toFixed(1)} min | ${build} | ` +
      `${parts.reduce((n, p) => n + p.slow, 0)} |`);
    if (wall / 60 > BUDGET_MINUTES[lane]) {
      breaches.push(`- Budget breach: ${lane} nextest wall ${(wall / 60).toFixed(1)} min exceeds ${BUDGET_MINUTES[lane]} min.`);
    }
  }
  return ['### Lane timing (advisory)', '',
    `| Lane | Tests | Nextest wall | Build and other gate time | Tests of ${SLOW_SECONDS} s or more |`,
    '| --- | ---: | ---: | ---: | ---: |', ...rows, '', ...breaches].join('\n');
}

async function trackNightly({ github, context, core, result, junitDir }) {
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
  // The timing report is advisory: a bad artifact or API error must not block tracking.
  let timing = '';
  if (junitDir) {
    try {
      timing = await timingReport({ github, context, junitDir });
    } catch (error) {
      core.warning(`Lane timing report skipped: ${error.message}`);
    }
  }
  if (result === 'success' && (!issue || issue.state === 'closed')) {
    if (timing) await core.summary.addRaw(timing).write();
    return;
  }
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
    'Releases remain blocked until full validation succeeds on their exact tagged commit.',
    ...(timing ? ['', timing] : []), END].join('\n');
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

module.exports = { trackNightly, timingReport, MARKER, END };
