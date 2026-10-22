#!/usr/bin/env node

const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");

const repoRoot = path.resolve(__dirname, "../../..");
const modulePath = path.join(repoRoot, ".github/scripts/release-ci-gate.cjs");

test("the checked-in release CI gate module exists", () => {
  assert.equal(
    fs.existsSync(modulePath),
    true,
    ".github/scripts/release-ci-gate.cjs must own the release provenance checks",
  );
});

if (fs.existsSync(modulePath)) {
  const {
    REQUIRED_CHECKS,
    canonicalReleaseBranch,
    classifyLatestChecks,
    findTrustedMainCi,
    findTrustedMergeGroupCi,
    findTrustedPullRequestCi,
    runReleaseGate,
    waitForTrustedMainCi,
  } = require(modulePath);

  const owner = "sympoies";
  const repo = "nils-cli";
  const fullName = `${owner}/${repo}`;
  const releaseSha = "a".repeat(40);
  const baseSha = "b".repeat(40);
  const pullHeadSha = "c".repeat(40);
  const releaseTreeSha = "d".repeat(40);
  const branch = "chore/release-1-22-10";
  const context = {
    ref: "refs/tags/v1.22.10",
    repo: { owner, repo },
  };

  function successfulJobs({ withFullMarkers = false } = {}) {
    return REQUIRED_CHECKS.map((name, index) => ({
      id: index + 1,
      name,
      status: "completed",
      conclusion: "success",
      steps: withFullMarkers
        ? [{ name: "Full validation marker", status: "completed", conclusion: "success" }]
        : [],
    }));
  }

  function fixture(overrides = {}) {
    const pull = {
      number: 1253,
      merged_at: "2026-07-16T12:00:00Z",
      merge_commit_sha: releaseSha,
      html_url: `https://github.com/${fullName}/pull/1253`,
      head: {
        ref: branch,
        sha: releaseSha,
        repo: { full_name: fullName },
      },
      base: {
        ref: "main",
        repo: { full_name: fullName },
      },
    };
    const pullRun = {
      id: 100,
      event: "pull_request",
      status: "completed",
      conclusion: "success",
      head_branch: branch,
      head_sha: releaseSha,
      html_url: `https://github.com/${fullName}/actions/runs/100`,
      repository: { full_name: fullName },
      head_repository: { full_name: fullName },
      pull_requests: [],
    };
    const mainRun = {
      id: 200,
      event: "push",
      status: "completed",
      conclusion: "success",
      head_branch: "main",
      head_sha: baseSha,
      html_url: `https://github.com/${fullName}/actions/runs/200`,
      repository: { full_name: fullName },
    };
    const state = {
      pulls: [pull],
      workflowRuns: [pullRun, mainRun],
      jobsByRun: {
        100: successfulJobs(),
        200: successfulJobs({ withFullMarkers: true }),
      },
      checkRuns: REQUIRED_CHECKS.map((name) => ({
        name,
        status: "completed",
        conclusion: "success",
        completed_at: "2026-07-16T12:00:00Z",
      })),
      commitsByRef: {
        [releaseSha]: { sha: releaseSha, commit: { tree: { sha: releaseTreeSha } } },
        [pullHeadSha]: { sha: pullHeadSha, commit: { tree: { sha: releaseTreeSha } } },
      },
      ...overrides,
    };
    return {
      state,
      github: {
        rest: {
          pulls: {
            list: async () => ({ data: state.pulls }),
          },
          actions: {
            listWorkflowRuns: async ({ event, head_sha: headSha }) => ({
              data: {
                workflow_runs: state.workflowRuns.filter(
                  (run) => run.event === event && run.head_sha === headSha,
                ),
              },
            }),
            listJobsForWorkflowRun: async ({ run_id: runId }) => ({
              data: { jobs: state.jobsByRun[runId] || [] },
            }),
          },
          checks: {
            listForRef: async () => ({ data: { check_runs: state.checkRuns } }),
          },
          repos: {
            getCommit: async ({ ref }) => ({ data: state.commitsByRef[ref] }),
          },
        },
      },
    };
  }

  test("canonicalReleaseBranch accepts only stable v-prefixed release tags", () => {
    assert.equal(canonicalReleaseBranch("refs/tags/v1.22.10"), branch);
    assert.equal(canonicalReleaseBranch("refs/tags/v1.22.10-rc.1"), null);
    assert.equal(canonicalReleaseBranch("refs/heads/main"), null);
  });

  test("a unique same-repository merged release PR with exact-SHA green jobs is trusted", async () => {
    const { github } = fixture();
    const trusted = await findTrustedPullRequestCi({ github, context, sha: releaseSha });

    assert.deepEqual(trusted, {
      prNumber: 1253,
      runId: 100,
      runUrl: `https://github.com/${fullName}/actions/runs/100`,
    });
  });

  test("a squash-merged release PR is trusted when its tested head tree matches the tagged tree", async () => {
    const { state, github } = fixture();
    state.pulls[0].head.sha = pullHeadSha;
    state.workflowRuns[0].head_sha = pullHeadSha;
    state.workflowRuns[0].pull_requests = [
      {
        number: 1253,
        head: { sha: pullHeadSha },
        base: { ref: "main" },
      },
    ];

    const trusted = await findTrustedPullRequestCi({ github, context, sha: releaseSha });

    assert.deepEqual(trusted, {
      prNumber: 1253,
      runId: 100,
      runUrl: `https://github.com/${fullName}/actions/runs/100`,
    });
  });

  test("a squash-merged release PR fails closed when its tested tree differs from the tagged tree", async () => {
    const { state, github } = fixture();
    state.pulls[0].head.sha = pullHeadSha;
    state.workflowRuns[0].head_sha = pullHeadSha;
    state.commitsByRef[pullHeadSha].commit.tree.sha = "e".repeat(40);

    assert.equal(await findTrustedPullRequestCi({ github, context, sha: releaseSha }), null);
  });

  test("ambiguous workflow runs fail closed", async () => {
    const { state, github } = fixture();
    state.workflowRuns.push({ ...state.workflowRuns[0], id: 101 });
    state.jobsByRun[101] = successfulJobs();

    assert.equal(await findTrustedPullRequestCi({ github, context, sha: releaseSha }), null);
  });

  test("forked, unmerged, wrong-SHA, and incomplete PR evidence fail closed", async (t) => {
    const mutations = {
      forked: (state) => {
        state.pulls[0].head.repo.full_name = "someone/nils-cli";
      },
      unmerged: (state) => {
        state.pulls[0].merged_at = null;
      },
      "wrong SHA": (state) => {
        state.pulls[0].merge_commit_sha = "c".repeat(40);
      },
      "missing required job": (state) => {
        state.jobsByRun[100] = state.jobsByRun[100].filter(({ name }) => name !== "coverage");
      },
      "failed required job": (state) => {
        state.jobsByRun[100][0].conclusion = "failure";
      },
      "fork workflow run": (state) => {
        state.workflowRuns[0].head_repository.full_name = "someone/nils-cli";
      },
      "mismatched associated PR": (state) => {
        state.workflowRuns[0].pull_requests = [
          {
            number: 9999,
            head: { sha: releaseSha },
            base: { ref: "main" },
          },
        ];
      },
      "missing tagged commit evidence": (state) => {
        state.pulls[0].head.sha = pullHeadSha;
        state.workflowRuns[0].head_sha = pullHeadSha;
        delete state.commitsByRef[releaseSha];
      },
    };

    for (const [name, mutate] of Object.entries(mutations)) {
      await t.test(name, async () => {
        const { state, github } = fixture();
        mutate(state);
        assert.equal(await findTrustedPullRequestCi({ github, context, sha: releaseSha }), null);
      });
    }
  });

  test("base main CI is trusted only when every full-validation marker succeeded", async () => {
    const { github } = fixture();
    assert.deepEqual(await findTrustedMainCi({ github, context, sha: baseSha }), {
      runId: 200,
      runUrl: `https://github.com/${fullName}/actions/runs/200`,
    });

    const missingMarker = fixture();
    missingMarker.state.jobsByRun[200][1].steps = [];
    assert.equal(
      await findTrustedMainCi({ github: missingMarker.github, context, sha: baseSha }),
      null,
    );
  });

  test("a successful full merge-group receipt can authorize release-only CI for its base", async () => {
    const mergeGroupRun = {
      id: 302,
      event: "merge_group",
      status: "completed",
      conclusion: "success",
      head_sha: baseSha,
      head_branch: "gh-readonly-queue/main/pr-1252",
      html_url: `https://github.com/${fullName}/actions/runs/302`,
      repository: { full_name: fullName },
    };
    const { github } = fixture({
      workflowRuns: [mergeGroupRun],
      jobsByRun: { 302: successfulJobs({ withFullMarkers: true }) },
    });

    assert.deepEqual(
      await waitForTrustedMainCi({ github, context, core: { info: () => {} }, sha: baseSha }),
      { runId: 302, runUrl: mergeGroupRun.html_url },
    );
  });

  test("an exact-SHA successful merge-group receipt with required jobs is trusted", async () => {
    const mergeGroupRun = {
      id: 300,
      event: "merge_group",
      status: "completed",
      conclusion: "success",
      head_sha: releaseSha,
      head_branch: "gh-readonly-queue/main/pr-1253",
      html_url: `https://github.com/${fullName}/actions/runs/300`,
      repository: { full_name: fullName },
    };
    const { state, github } = fixture({
      workflowRuns: [mergeGroupRun],
      jobsByRun: { 300: successfulJobs() },
    });

    assert.deepEqual(
      await findTrustedMergeGroupCi({ github, context, sha: releaseSha }),
      { runId: 300, runUrl: mergeGroupRun.html_url },
    );
    assert.equal(state.workflowRuns[0].head_sha, releaseSha);
  });

  test("merge-group receipt rejects a mismatched SHA, failed job, or missing coverage", async (t) => {
    const mergeGroupRun = {
      id: 300,
      event: "merge_group",
      status: "completed",
      conclusion: "success",
      head_sha: releaseSha,
      head_branch: "gh-readonly-queue/main/pr-1253",
      html_url: `https://github.com/${fullName}/actions/runs/300`,
      repository: { full_name: fullName },
    };
    const mutations = {
      "mismatched SHA": (state) => { state.workflowRuns[0].head_sha = pullHeadSha; },
      "failed required job": (state) => { state.jobsByRun[300][0].conclusion = "failure"; },
      "missing coverage": (state) => {
        state.jobsByRun[300] = state.jobsByRun[300].filter(({ name }) => name !== "coverage");
      },
    };

    for (const [name, mutate] of Object.entries(mutations)) {
      await t.test(name, async () => {
        const { state, github } = fixture({
          workflowRuns: [{ ...mergeGroupRun }],
          jobsByRun: { 300: successfulJobs() },
        });
        mutate(state);
        assert.equal(await findTrustedMergeGroupCi({ github, context, sha: releaseSha }), null);
      });
    }
  });

  test("push receipts remain available as a release-gate alternative", async () => {
    const { github } = fixture();
    assert.deepEqual(await findTrustedMainCi({ github, context, sha: baseSha }), {
      runId: 200,
      runUrl: `https://github.com/${fullName}/actions/runs/200`,
    });
  });

  function quietCore() {
    const messages = [];
    return {
      messages,
      core: {
        info: (message) => messages.push(message),
        warning: (message) => messages.push(message),
        setFailed: assert.fail,
      },
    };
  }

  test("base main CI still running is awaited until it succeeds", async () => {
    const { state, github } = fixture();
    const mainRun = state.workflowRuns[1];
    mainRun.status = "in_progress";
    mainRun.conclusion = null;
    let sleeps = 0;
    const sleep = async () => {
      sleeps += 1;
      if (sleeps === 2) {
        mainRun.status = "completed";
        mainRun.conclusion = "success";
      }
    };
    const { core, messages } = quietCore();

    assert.deepEqual(
      await waitForTrustedMainCi({ github, context, core, sha: baseSha, attempts: 5, sleep }),
      { runId: 200, runUrl: `https://github.com/${fullName}/actions/runs/200` },
    );
    assert.equal(sleeps, 2);
    assert.match(messages.join("\n"), /Waiting for base main CI/);
  });

  test("base main CI that fails while awaited stops polling and fails closed", async () => {
    const { state, github } = fixture();
    const mainRun = state.workflowRuns[1];
    mainRun.status = "in_progress";
    mainRun.conclusion = null;
    let sleeps = 0;
    const sleep = async () => {
      sleeps += 1;
      mainRun.status = "completed";
      mainRun.conclusion = "failure";
    };

    assert.equal(
      await waitForTrustedMainCi({
        github,
        context,
        core: quietCore().core,
        sha: baseSha,
        attempts: 5,
        sleep,
      }),
      null,
    );
    assert.equal(sleeps, 1);
  });

  test("base main CI without any push run fails closed without waiting", async () => {
    const { state, github } = fixture();
    state.workflowRuns = state.workflowRuns.filter((run) => run.event !== "push");
    const sleep = async () => assert.fail("nothing to wait for");

    assert.equal(
      await waitForTrustedMainCi({
        github,
        context,
        core: quietCore().core,
        sha: baseSha,
        sleep,
      }),
      null,
    );
  });

  test("base main CI that outlasts the wait budget fails closed", async () => {
    const { state, github } = fixture();
    state.workflowRuns[1].status = "queued";
    state.workflowRuns[1].conclusion = null;
    let sleeps = 0;
    const sleep = async () => {
      sleeps += 1;
    };
    const { core, messages } = quietCore();

    assert.equal(
      await waitForTrustedMainCi({ github, context, core, sha: baseSha, attempts: 3, sleep }),
      null,
    );
    assert.equal(sleeps, 2);
    assert.match(messages.join("\n"), /still running after 3 checks/);
  });

  test("each wait attempt decides from one run listing, so completion between reads is not missed", async () => {
    const { state, github } = fixture();
    const mainRun = state.workflowRuns[1];
    mainRun.status = "in_progress";
    mainRun.conclusion = null;
    const listRuns = github.rest.actions.listWorkflowRuns;
    let pushListings = 0;
    github.rest.actions.listWorkflowRuns = async (params) => {
      if (params.event === "push") {
        pushListings += 1;
        const { data } = await listRuns(params);
        const response = {
          data: { workflow_runs: data.workflow_runs.map((run) => ({ ...run })) },
        };
        // The run finishes right after the first listing is served.
        if (pushListings === 1) {
          mainRun.status = "completed";
          mainRun.conclusion = "success";
        }
        return response;
      }
      return listRuns(params);
    };
    let sleeps = 0;
    const sleep = async () => {
      sleeps += 1;
    };

    assert.deepEqual(
      await waitForTrustedMainCi({
        github,
        context,
        core: quietCore().core,
        sha: baseSha,
        attempts: 5,
        sleep,
      }),
      { runId: 200, runUrl: `https://github.com/${fullName}/actions/runs/200` },
    );
    assert.equal(sleeps, 1);
  });

  test("base main CI still running at the wall-clock deadline fails closed", async () => {
    const { state, github } = fixture();
    state.workflowRuns[1].status = "in_progress";
    state.workflowRuns[1].conclusion = null;
    let clock = 0;
    let sleeps = 0;
    const sleep = async (ms) => {
      sleeps += 1;
      clock += ms;
    };
    const { core, messages } = quietCore();

    assert.equal(
      await waitForTrustedMainCi({
        github,
        context,
        core,
        sha: baseSha,
        attempts: 100,
        intervalMs: 1_000,
        deadlineMs: 2_500,
        now: () => clock,
        sleep,
      }),
      null,
    );
    assert.equal(sleeps, 2);
    assert.match(messages.join("\n"), /still running at the wait deadline/);
  });

  test("default wait budget covers a base main CI run that queues before its ~30 minute run", async () => {
    const { state, github } = fixture();
    const mainRun = state.workflowRuns[1];
    mainRun.status = "in_progress";
    mainRun.conclusion = null;
    let clock = 0;
    const sleep = async (ms) => {
      clock += ms;
      // Base CI completes 42 minutes after the release PR starts waiting.
      if (clock >= 42 * 60_000) {
        mainRun.status = "completed";
        mainRun.conclusion = "success";
      }
    };

    assert.deepEqual(
      await waitForTrustedMainCi({
        github,
        context,
        core: quietCore().core,
        sha: baseSha,
        now: () => clock,
        sleep,
      }),
      { runId: 200, runUrl: `https://github.com/${fullName}/actions/runs/200` },
    );
  });

  test("a pending push run from another repository is not awaited", async () => {
    const { state, github } = fixture();
    state.workflowRuns[1].status = "in_progress";
    state.workflowRuns[1].conclusion = null;
    state.workflowRuns[1].repository.full_name = "someone/nils-cli";
    const sleep = async () => assert.fail("foreign runs must not hold the lane");

    assert.equal(
      await waitForTrustedMainCi({
        github,
        context,
        core: quietCore().core,
        sha: baseSha,
        sleep,
      }),
      null,
    );
  });

  test("release gate reuses trusted PR CI without polling duplicate checks", async () => {
    const { github } = fixture();
    github.rest.checks.listForRef = async () => {
      assert.fail("trusted PR CI should avoid exact-SHA check polling");
    };
    const messages = [];
    const core = {
      info: (message) => messages.push(message),
      warning: (message) => messages.push(message),
      setFailed: assert.fail,
    };

    assert.equal(await runReleaseGate({ github, context: { ...context, sha: releaseSha }, core }), true);
    assert.match(messages.join("\n"), /reuses trusted CI run 100/);
  });

  test("release gate accepts exact-SHA successful merge-group CI", async () => {
    const mergeGroupRun = {
      id: 300,
      event: "merge_group",
      status: "completed",
      conclusion: "success",
      head_sha: releaseSha,
      head_branch: "gh-readonly-queue/main/pr-1253",
      html_url: `https://github.com/${fullName}/actions/runs/300`,
      repository: { full_name: fullName },
    };
    const { state, github } = fixture({
      pulls: [],
      workflowRuns: [mergeGroupRun],
      jobsByRun: { 300: successfulJobs() },
      checkRuns: [],
    });
    const core = { info: () => {}, warning: () => {}, setFailed: assert.fail };

    assert.equal(
      await runReleaseGate({ github, context: { ...context, sha: releaseSha }, core, attempts: 1 }),
      true,
    );
    assert.equal(state.workflowRuns[0].head_sha, releaseSha);
  });

  test("release gate continues to accept exact-SHA successful main push CI", async () => {
    const pushRun = {
      id: 301,
      event: "push",
      status: "completed",
      conclusion: "success",
      head_sha: releaseSha,
      head_branch: "main",
      html_url: `https://github.com/${fullName}/actions/runs/301`,
      repository: { full_name: fullName },
    };
    const { github } = fixture({
      pulls: [],
      workflowRuns: [pushRun],
      jobsByRun: { 301: successfulJobs({ withFullMarkers: true }) },
      checkRuns: [],
    });
    const core = { info: () => {}, warning: () => {}, setFailed: assert.fail };

    assert.equal(
      await runReleaseGate({ github, context: { ...context, sha: releaseSha }, core, attempts: 1 }),
      true,
    );
  });

  test("release gate falls back to exact-SHA checks when provenance lookup errors", async () => {
    const { github } = fixture();
    github.rest.pulls.list = async () => {
      throw new Error("temporary API failure");
    };
    const messages = [];
    const core = {
      info: (message) => messages.push(message),
      warning: (message) => messages.push(message),
      setFailed: assert.fail,
    };

    assert.equal(
      await runReleaseGate({
        github,
        context: { ...context, sha: releaseSha },
        core,
        attempts: 1,
      }),
      true,
    );
    assert.match(messages.join("\n"), /falling back to exact-SHA check runs/);
  });

  test("latest exact-SHA check runs retain success, pending, and failure fallback states", () => {
    const successRuns = REQUIRED_CHECKS.map((name, index) => ({
      name,
      status: "completed",
      conclusion: "success",
      completed_at: `2026-07-16T12:0${index}:00Z`,
    }));
    assert.deepEqual(classifyLatestChecks(successRuns), {
      state: "success",
      pending: [],
      failing: [],
    });

    const pendingRuns = [
      ...successRuns,
      {
        name: "coverage",
        status: "in_progress",
        conclusion: null,
        started_at: "2026-07-16T13:00:00Z",
      },
    ];
    assert.equal(classifyLatestChecks(pendingRuns).state, "pending");

    const failedRuns = successRuns.map((run) => ({ ...run }));
    failedRuns[0].conclusion = "failure";
    failedRuns[0].html_url = "https://example.test/failure";
    assert.deepEqual(classifyLatestChecks(failedRuns), {
      state: "failure",
      pending: [],
      failing: ["test: failure (https://example.test/failure)"],
    });
  });
}

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
