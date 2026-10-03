"use strict";

const fs = require("node:fs");
const OFFICIAL = { owner: "openclaw", repo: "Peekaboo" };
const REPOSITORY = "https://github.com/openclaw/Peekaboo";
const SECTION_END = "<!-- peekaboo-release-check:end -->";

function version(tag) {
  const match = /^v(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/.exec(tag || "");
  return match ? match.slice(1).map(BigInt) : null;
}

function compare(left, right) {
  const a = version(left);
  const b = version(right);
  for (let index = 0; index < a.length; index += 1) {
    if (a[index] !== b[index]) return a[index] > b[index] ? 1 : -1;
  }
  return 0;
}

function report(result) {
  return [
    "## Peekaboo release check",
    "",
    `- Last checked: ${result.checked_at}`,
    `- Current accepted lock: ${result.current}`,
    `- Newest stable official release: ${result.newest || "unavailable"}`,
    `- Candidate: ${result.candidate || "none"} (not selected)`,
    `- Status: ${result.status}`,
    `- Check run: ${result.run_url}`,
    ...(result.release_url ? [`- Official release: ${result.release_url}`] : []),
    ...(result.candidate_url ? [`- Candidate record: ${result.candidate_url}`] : []),
    "",
    result.action,
  ].join("\n");
}

async function recordCandidate({ github, context, result }) {
  const marker = `<!-- peekaboo-release-check:${result.candidate} -->`;
  // All states matter: a rejected/closed candidate must not be recreated daily.
  const issues = await github.paginate(github.rest.issues.listForRepo, {
    ...context.repo, state: "all", per_page: 100,
  });
  const matches = issues.filter((issue) => !issue.pull_request &&
    (issue.body || "").includes(marker));
  if (matches.length > 1) throw new Error("duplicate candidate records; maintainer must reconcile them");
  if (matches.length === 1 && matches[0].user?.login !== "github-actions[bot]") {
    throw new Error("foreign-owned candidate marker; maintainer must reconcile ownership before retrying");
  }
  const section = `${marker}\n${report(result)}\n${SECTION_END}`;
  if (matches.length === 0) {
    const response = await github.rest.issues.create({
      ...context.repo, title: `macos-agent: review Peekaboo ${result.candidate}`, body: section,
    });
    return response.data.html_url;
  }
  const issue = matches[0];
  const start = issue.body.indexOf(marker);
  const end = issue.body.indexOf(SECTION_END, start);
  if (end < 0) throw new Error("candidate record section is incomplete; maintainer must repair its markers");
  // Replace only the machine-owned section; preserve decisions, notes and state.
  await github.rest.issues.update({ ...context.repo, issue_number: issue.number,
    body: issue.body.slice(0, start) + section + issue.body.slice(end + SECTION_END.length) });
  return issue.html_url;
}

async function runReleaseCheck({ github, context, core,
  lock = JSON.parse(fs.readFileSync("crates/macos-agent/peekaboo-lock.json", "utf8")),
  now = () => new Date(),
  writeResult = (result) => fs.writeFileSync("peekaboo-release-check.json", `${JSON.stringify(result, null, 2)}\n`),
}) {
  const result = { schema_version: 1, checked_at: now().toISOString(), current: lock.tag,
    newest: null, candidate: null, status: "error",
    run_url: `https://github.com/${context.repo.owner}/${context.repo.repo}/actions/runs/${context.runId}`,
    action: "Release discovery failed; keep the accepted backend and inspect the check run before retrying." };
  try {
    if (lock.repository !== REPOSITORY || !version(lock.tag)) throw new Error("invalid official backend lock");
    const releases = await github.paginate(github.rest.repos.listReleases, { ...OFFICIAL, per_page: 100 });
    const stable = releases.filter((release) => release.draft === false && release.prerelease === false && version(release.tag_name))
      .sort((a, b) => compare(b.tag_name, a.tag_name));
    if (!stable.length) throw new Error("no stable official release found");
    const newest = stable[0];
    result.newest = newest.tag_name;
    result.release_url = `${REPOSITORY}/releases/tag/${result.newest}`;
    result.status = "current";
    result.action = "No newer stable release found; keep the accepted backend.";
    if (compare(result.newest, lock.tag) > 0) {
      result.candidate = result.newest;
      const names = new Set((newest.assets || []).map((asset) => asset.name));
      const missing = ["peekaboo-macos-universal.tar.gz", `Peekaboo-${result.candidate.slice(1)}.app.zip`]
        .filter((name) => !names.has(name));
      result.status = missing.length ? "unsupported" : "candidate";
      result.action = missing.length
        ? `Missing official install assets: ${missing.join(", ")}. Report upstream or review adapter support; keep the accepted backend.`
        : "Prepare a separate exact-lock PR with official archive/executable digests, commit, signing and notarization metadata. " +
          "Run the existing compatibility/security gates and resident macOS interaction acceptance before selection. " +
          "Failed candidates must retain an actionable report and keep the accepted backend.";
      result.candidate_url = await recordCandidate({ github, context, result });
    }
  } catch (error) {
    result.status = "error";
    // Do not forward API diagnostics or upstream prose into public reports.
    const reason = ["duplicate candidate records; maintainer must reconcile them",
      "foreign-owned candidate marker; maintainer must reconcile ownership before retrying",
      "candidate record section is incomplete; maintainer must repair its markers", "invalid official backend lock",
      "no stable official release found"].includes(error.message) ? error.message : "GitHub API request failed";
    result.action = `Release check failed: ${reason}; keep the accepted backend. Inspect the check run and retry after resolving the failure.`;
    core.setFailed(result.action);
  } finally {
    writeResult(result);
    await core.summary.addRaw(report(result)).write();
  }
  return result;
}

module.exports = { runReleaseCheck };
