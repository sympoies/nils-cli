"use strict";

const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const crypto = require("node:crypto");
const { spawnSync } = require("node:child_process");
const PACKAGE = "@anthropic-ai/claude-code";
// The scheduled runner is Ubuntu x64/glibc. This names the external npm layout
// introduced in Claude Code 2.1.316; a changed layout fails closed for review.
const PLATFORM = `${PACKAGE}-linux-x64`;
const REGISTRY = "https://registry.npmjs.org/";
const VERSION = "crates/agent-session/src/send/claude_notice/version.rs";
const NOTICE = "crates/agent-session/tests/guarded_send/claude_notice.rs";
const SPEC = "crates/agent-session/docs/specs/session-notices-v1.md";
const CREATION_UNAVAILABLE = "candidate creation unavailable; maintainer must check workflow PR creation permission and prepare the draft manually or rerun; do not add credentials or change repository settings automatically";
const END = "<!-- claude-release-check:end -->";
const COMMANDS = [
  "npm install --prefix $SANDBOX/install --ignore-scripts --omit=optional --offline --no-audit --no-fund --no-package-lock $SANDBOX/0.tgz $SANDBOX/1.tgz",
  "$SANDBOX/bin/claude --version",
  "$CONTRACT_TEST_BINARY send::claude_notice::module_contract:: --nocapture (call-shape scan and claude plugin validate)",
];
const sha = (text) => crypto.createHash("sha256").update(text).digest("hex");
const read = (root, file) => fs.readFileSync(path.join(root, file), "utf8");
const stable = (text) => /^(0|[1-9]\d{0,8})\.(0|[1-9]\d{0,8})\.(0|[1-9]\d{0,8})$/.test(text || "");
function compare(a, b) {
  if (!stable(a) || !stable(b)) throw new Error("invalid stable version");
  const x = a.split(".").map(Number), y = b.split(".").map(Number);
  for (let i = 0; i < 3; i++) if (x[i] !== y[i]) return x[i] > y[i] ? 1 : -1;
  return 0;
}
function range(root) {
  const text = read(root, VERSION);
  const bound = (name) => {
    const match = text.match(new RegExp(`${name}: Version = Version\\((\\d+), (\\d+), (\\d+)\\);`));
    if (!match) throw new Error("range owner changed; update the preparation helper");
    return match.slice(1).join(".");
  };
  return { min: bound("SUPPORTED_MIN"), max: bound("SUPPORTED_MAX") };
}
function metadata(item, name, exact) {
  if (item?.name !== name || !stable(item.version) || (exact && item.version !== exact))
    throw new Error("invalid official stable package metadata");
  const basename = name.split("/")[1];
  if (item.dist?.tarball !== `${REGISTRY}${name}/-/${basename}-${item.version}.tgz` ||
      !/^sha512-[A-Za-z0-9+/]{86}==$/.test(item.dist?.integrity || ""))
    throw new Error("invalid official registry URL or integrity");
  return item;
}
async function getBytes(url, limit = 400 * 1024 * 1024) {
  const response = await fetch(url, { redirect: "error", signal: AbortSignal.timeout(120000) });
  if (!response.ok) throw new Error("official registry request failed");
  const chunks = []; let size = 0;
  for await (const chunk of response.body) {
    size += chunk.length;
    if (size > limit) { await response.body.cancel().catch(() => {}); throw new Error("registry response exceeds bound"); }
    chunks.push(chunk);
  }
  return Buffer.concat(chunks);
}
const getJSON = async (url) => JSON.parse((await getBytes(url, 2 * 1024 * 1024)).toString("utf8"));
async function discover(fetchJSON = getJSON) {
  // npm's official latest dist-tag, with prerelease/non-SemVer values refused.
  return metadata(await fetchJSON(`${REGISTRY}@anthropic-ai%2Fclaude-code/latest`), PACKAGE);
}
function verifyIntegrity(bytes, integrity) {
  if (!/^sha512-[A-Za-z0-9+/]{86}==$/.test(integrity || "")) throw new Error("invalid registry integrity");
  const expected = Buffer.from(integrity.slice(7), "base64");
  const actual = crypto.createHash("sha512").update(bytes).digest();
  if (expected.length !== actual.length || !crypto.timingSafeEqual(expected, actual))
    throw new Error("registry integrity mismatch");
  return true;
}
function isolatedEnvironment(sandbox) {
  const env = { PATH: `${path.join(sandbox, "bin")}:${path.dirname(process.execPath)}:/usr/bin:/bin`, DISABLE_AUTOUPDATER: "1",
    DISABLE_TELEMETRY: "1", DISABLE_ERROR_REPORTING: "1", CI: "1", LANG: "C.UTF-8" };
  for (const [key, dir] of Object.entries({ HOME: "home", CLAUDE_CONFIG_DIR: "config", XDG_CONFIG_HOME: "xdg-config",
    XDG_CACHE_HOME: "xdg-cache", XDG_DATA_HOME: "xdg-data", XDG_STATE_HOME: "xdg-state", TMPDIR: "tmp",
    npm_config_cache: "npm-cache" })) {
    env[key] = path.join(sandbox, dir); fs.mkdirSync(env[key], { recursive: true, mode: 0o700 });
  }
  fs.mkdirSync(path.join(sandbox, "bin"), { recursive: true, mode: 0o700 });
  return env;
}
async function checkCandidate({ release, contractBinary, root = process.cwd(), fetchJSON = getJSON,
  fetchBytes = getBytes, execute = spawnSync, artifactDir = null }) {
  const sandbox = fs.mkdtempSync(path.join(os.tmpdir(), "claude-release-check-"));
  const result = { passed: false, packages: [], commands: [], failure: "isolated installation or contract failed" };
  try {
    metadata(release, PACKAGE);
    if (release.optionalDependencies?.[PLATFORM] !== release.version) throw new Error("official platform dependency changed");
    const native = metadata(await fetchJSON(`${REGISTRY}${encodeURIComponent(PLATFORM)}/${release.version}`), PLATFORM, release.version);
    // Verify both the release wrapper and the exact executable package before installation.
    for (const [index, item] of [release, native].entries()) {
      const bytes = await fetchBytes(item.dist.tarball);
      const record = { name: item.name, version: item.version, tarball: item.dist.tarball, integrity: item.dist.integrity, verified: false };
      result.packages.push(record);
      verifyIntegrity(bytes, item.dist.integrity);
      record.verified = true;
      fs.writeFileSync(path.join(sandbox, `${index}.tgz`), bytes);
    }
    const env = isolatedEnvironment(sandbox);
    const run = (command, args, label) => {
      const output = execute(command, args, { cwd: sandbox, env, timeout: 240000, maxBuffer: 1024 * 1024, encoding: "utf8" });
      result.commands.push({ command: label, exit: output.status ?? null, signal: output.signal || null });
      // Child diagnostics are artifacts, never copied into public issue/PR prose.
      if (artifactDir) fs.writeFileSync(path.join(artifactDir, `command-${result.commands.length}.log`),
        `${output.stdout || ""}${output.stderr || ""}`.replaceAll(sandbox, "$SANDBOX"));
      if (output.error || output.status !== 0) throw new Error("isolated command failed");
      return `${output.stdout || ""}${output.stderr || ""}`;
    };
    run("npm", ["install", "--prefix", path.join(sandbox, "install"), "--ignore-scripts", "--omit=optional", "--offline",
      "--no-audit", "--no-fund", "--no-package-lock", path.join(sandbox, "0.tgz"), path.join(sandbox, "1.tgz")],
    COMMANDS[0]);
    const binary = path.join(sandbox, "install/node_modules", PLATFORM, "claude");
    if (!fs.statSync(binary).isFile()) throw new Error("official executable layout changed");
    fs.symlinkSync(binary, path.join(sandbox, "bin/claude"));
    const banner = run(binary, ["--version"], COMMANDS[1]).trim();
    if (banner !== `${release.version} (Claude Code)`) throw new Error("candidate banner mismatch");
    if (!contractBinary || !path.isAbsolute(contractBinary)) throw new Error("compiled contract test binary required");
    const output = run(contractBinary, ["send::claude_notice::module_contract::", "--nocapture"],
      COMMANDS[2]);
    // The existing optional installed-CLI test must actually run for this candidate.
    if (!output.includes(`checking the module against Claude Code ${release.version}\n`) || output.includes("skipped:") ||
        !/test result: ok\. [3-9]\d* passed; 0 failed; 0 ignored;/.test(output)) throw new Error("contract skipped or incomplete");
    result.passed = true; result.failure = null;
  } catch {
    // Bound diagnostics and keep provider/registry/child text out of public records.
  } finally { fs.rmSync(sandbox, { recursive: true, force: true }); }
  return result;
}
function replaceBoundLiterals(text, old, oldNext, candidate, next) {
  return text.replace(/"(\d+\.\d+\.\d+)"/g, (literal, value) =>
    value === old ? `"${candidate}"` : value === oldNext ? `"${next}"` : literal);
}
function candidateChanges(root, candidate) {
  const current = range(root);
  if (compare(candidate, current.max) <= 0) throw new Error("candidate is not newer");
  const parts = candidate.split(".").map(Number); parts[2]++;
  const next = parts.join(".");
  if (!stable(next)) throw new Error("candidate next boundary is unsupported");
  const old = current.max, oldNext = old.split(".").map(Number); oldNext[2]++;
  let version = read(root, VERSION);
  version = version.replace(`SUPPORTED_MAX: Version = Version(${old.replaceAll(".", ", ")});`,
    `SUPPORTED_MAX: Version = Version(${candidate.replaceAll(".", ", ")});`);
  version = version.replace(`let (min, max) = (Version(${current.min.replaceAll(".", ", ")}), Version(${old.replaceAll(".", ", ")}));`,
    `let (min, max) = (Version(${current.min.replaceAll(".", ", ")}), Version(${candidate.replaceAll(".", ", ")}));`);
  version = replaceBoundLiterals(version, old, oldNext.join("."), candidate, next)
    .replace(/\("(\d+\.\d+\.\d+)", (?:true|false)\)/g, (_, v) => `("${v}", ${compare(v, current.min) >= 0 && compare(v, candidate) <= 0})`);
  const notice = replaceBoundLiterals(read(root, NOTICE), old, oldNext.join("."), candidate, next);
  const spec = read(root, SPEC);
  const bounds = `\`${current.min}\` through \`${old}\` inclusive`;
  if (!spec.includes(bounds) || !version.includes(`Version(${candidate.replaceAll(".", ", ")})`) ||
      !notice.includes(`"supported_max": "${candidate}"`)) throw new Error("range expectation owners changed; review preparation helper");
  const pending = `Native acceptance: PENDING for candidate \`${candidate}\`. Contract checks do not\n` +
    `prove native delivery. Before this draft range change may merge, attach the\nexact-version native notice acceptance and complete reviewed merge gates.\n\n`;
  return { [VERSION]: version, [NOTICE]: notice,
    [SPEC]: spec.replace(bounds, `\`${current.min}\` through \`${candidate}\` inclusive`)
      .replace(`The current upper bound, \`${old}\`, rests`, `The previously accepted upper bound, \`${old}\`, rests`)
      .replace("### Tested Claude Code range\n\n", `### Tested Claude Code range\n\n${pending}`) };
}
// Runs only in the read-only job. The provider-write job never calls this
// collector or executes an executable from the candidate/evidence artifact.
async function collectReleaseCheck({ sourceSha, root = process.cwd(), discoverRelease = discover,
  checkRelease = checkCandidate, contractBinary = process.env.CLAUDE_CONTRACT_TEST_BINARY,
  artifactDir = "claude-release-check", writeEvidence = (checked, patch) => {
    fs.mkdirSync(artifactDir, { recursive: true });
    fs.writeFileSync(path.join(artifactDir, "checked.json"), JSON.stringify(checked, null, 2));
    fs.writeFileSync(path.join(artifactDir, "candidate.patch.json"), JSON.stringify(patch, null, 2));
  } }) {
  const checked = { schema_version: 1, source_sha: sourceSha, release: null, check: null, error: null };
  let patch = {};
  try {
    const release = metadata(await discoverRelease(), PACKAGE);
    checked.release = { name: release.name, version: release.version, dist: release.dist,
      optionalDependencies: { [PLATFORM]: release.optionalDependencies?.[PLATFORM] } };
    if (compare(release.version, range(root).max) > 0) {
      fs.mkdirSync(artifactDir, { recursive: true });
      checked.check = await checkRelease({ release, contractBinary, artifactDir });
      if (checked.check.passed) patch = candidateChanges(root, release.version);
    }
  } catch { checked.error = "unprivileged release collection failed"; }
  writeEvidence(checked, patch);
  return { checked, patch };
}
async function validateCollected({ checked, patch, root, sourceSha, fetchJSON }) {
  if (checked?.schema_version !== 1 || checked.source_sha !== sourceSha || checked.error || !checked.release)
    throw new Error("source-bound candidate evidence unavailable; inspect read-only check artifacts");
  const release = metadata(checked.release, PACKAGE);
  // Artifact contents are untrusted. Re-read exact registry metadata as data,
  // then regenerate the only allowed source patch from this trusted checkout.
  const official = metadata(await fetchJSON(`${REGISTRY}${encodeURIComponent(PACKAGE)}/${release.version}`), PACKAGE, release.version);
  if (release.dist.tarball !== official.dist.tarball || release.dist.integrity !== official.dist.integrity)
    throw new Error("candidate registry evidence mismatch");
  if (compare(release.version, range(root).max) <= 0) {
    if (Object.keys(patch || {}).length) throw new Error("unexpected in-range patch");
    return release;
  }
  const check = checked.check;
  if (!check || typeof check.passed !== "boolean" || !Array.isArray(check.packages) || check.packages.length > 2 ||
      !Array.isArray(check.commands) || check.commands.length > 3) throw new Error("invalid candidate check evidence");
  for (const [index, item] of check.packages.entries()) {
    const name = [PACKAGE, PLATFORM][index];
    const found = index === 0 ? official : metadata(await fetchJSON(`${REGISTRY}${encodeURIComponent(name)}/${release.version}`), name, release.version);
    if (item.name !== name || item.version !== release.version || item.tarball !== found.dist.tarball ||
        item.integrity !== found.dist.integrity || typeof item.verified !== "boolean") throw new Error("candidate package evidence mismatch");
  }
  for (const [index, command] of check.commands.entries()) {
    if (command.command !== COMMANDS[index] || !(command.exit === null || Number.isInteger(command.exit) && command.exit >= 0 && command.exit <= 255) ||
        ![null, undefined, "SIGTERM", "SIGKILL"].includes(command.signal)) throw new Error("invalid candidate command evidence");
  }
  if (check.passed && (check.packages.length !== 2 || check.commands.length !== 3 ||
      check.packages.some(item => !item.verified) || check.commands.some(item => item.exit !== 0 || item.signal)))
    throw new Error("incomplete passing candidate evidence");
  const expected = check.passed ? candidateChanges(root, release.version) : {};
  if (!patch || JSON.stringify(Object.keys(patch).sort()) !== JSON.stringify(Object.keys(expected).sort()) ||
      Object.entries(expected).some(([file, content]) => patch[file] !== content)) throw new Error("candidate patch differs from trusted range-only patch");
  return release;
}
function evidence(result) {
  return ["## Claude Code release check", "", `- Candidate: ${result.newest}`,
    `- Source revision: ${result.source_sha}`, `- Reviewed range: ${result.minimum} through ${result.maximum}`,
    `- Run and artifacts (claude-release-check): ${result.run_url}`,
    "- Upstream: https://www.npmjs.com/package/@anthropic-ai/claude-code",
    ...((result.check?.packages || []).map(p => `- ${p.name}@${p.version}: ${p.tarball}; ${p.integrity}; verified=${p.verified === true}`)),
    ...((result.check?.commands || []).map(c => `- Command: \`${c.command}\`; exit ${c.exit}; signal ${c.signal || "none"}`)),
    ...(result.check && !result.check.passed ? ["Module contract or isolated installation failed; inspect command artifacts."] : []),
    "", "Native acceptance: PENDING. Contract success never claims native success.",
    "Require exact-version native notice acceptance and reviewed merge gates before any range-extension merge.",
    "No automatic merge, release, deploy or runtime range widening. The schedule cannot guarantee beating every external auto-update.",
    "", `Status: ${result.status}. ${result.action || ""}`, "", "Refs #3138"].join("\n");
}
function ownedBody(marker, content) { return `${marker}\n<!-- sha256:${sha(content)} -->\n${content}\n${END}`; }
function intactBody(body, marker, exact = false) {
  const start = body?.indexOf(marker), end = body?.indexOf(END, start);
  if (start < 0 || end < 0) return false;
  const segment = body.slice(start + marker.length + 1, end);
  const match = /^<!-- sha256:([a-f0-9]{64}) -->\n([\s\S]*)\n$/.exec(segment);
  return !!match && sha(match[2]) === match[1] && (!exact || (start === 0 && end + END.length === body.length));
}
async function failureIssue({ github, context, result }) {
  const marker = `<!-- claude-release-failure:${result.newest} -->`;
  const issues = await github.paginate(github.rest.issues.listForRepo, { ...context.repo, state: "all", per_page: 100 });
  const matches = issues.filter(x => !x.pull_request && x.body?.includes(marker));
  if (matches.length > 1) throw new Error("duplicate failure issues; maintainer reconciliation required");
  const section = ownedBody(marker, evidence(result));
  if (!matches.length) return (await github.rest.issues.create({ ...context.repo,
    title: `agent-session: Claude Code ${result.newest} range preparation failed`, body: section })).data.html_url;
  const item = matches[0];
  if (item.user?.login !== "github-actions[bot]" || !intactBody(item.body, marker))
    throw new Error("human-owned failure evidence; maintainer reconciliation required");
  // Keep first failure evidence immutable. Later runs link this same record and
  // retain fresh evidence in their own artifacts, avoiding a read/write race
  // that could replace a maintainer's concurrent correction or closed decision.
  return item.html_url;
}
async function prepareDraft({ github, context, root, result }) {
  const branch = `ci/claude-range-${result.newest}`;
  const marker = `<!-- claude-release-candidate:${result.newest} -->`;
  const title = `ci(agent-session): prepare Claude Code ${result.newest} range extension`;
  const changes = candidateChanges(root, result.newest);
  const message = `ci(agent-session): prepare Claude Code ${result.newest}\n\nclaude-release-check:${sha(JSON.stringify(changes))}`;
  const pulls = await github.paginate(github.rest.pulls.list, { ...context.repo, state: "all", head: `${context.repo.owner}:${branch}`, per_page: 100 });
  if (pulls.length > 1) throw new Error("duplicate candidate PRs; maintainer reconciliation required");
  let ref = null;
  try { ref = (await github.rest.git.getRef({ ...context.repo, ref: `heads/${branch}` })).data.object.sha; }
  catch (error) { if (error.status !== 404) throw error; }
  if (pulls.length) {
    const pr = pulls[0];
    // Never update existing refs or PRs. Preserve every human/review decision.
    const reviews = await github.paginate(github.rest.pulls.listReviews, { ...context.repo, pull_number: pr.number, per_page: 100 });
    const comments = await github.paginate(github.rest.issues.listComments, { ...context.repo, issue_number: pr.number, per_page: 100 });
    const inline = await github.paginate(github.rest.pulls.listComments, { ...context.repo, pull_number: pr.number, per_page: 100 });
    const commit = ref ? (await github.rest.git.getCommit({ ...context.repo, commit_sha: ref })).data : null;
    if (pr.state !== "open" || !pr.draft || pr.user?.login !== "github-actions[bot]" || pr.title !== title ||
        pr.base?.ref !== "main" || pr.head?.repo?.full_name !== `${context.repo.owner}/${context.repo.repo}` ||
        pr.head.sha !== ref || !intactBody(pr.body, marker, true) || commit?.message !== message || reviews.length || comments.length || inline.length)
      return { status: "preserved", url: pr.html_url };
    return { status: "draft", url: pr.html_url };
  }
  const defaultHead = async () => (await github.rest.git.getRef({ ...context.repo, ref: "heads/main" })).data.object.sha;
  if (await defaultHead() !== context.sha) throw new Error("source revision moved; rerun on default branch");
  if (ref) {
    // Recover only our exact immutable orphan commit after failed PR creation.
    const commit = (await github.rest.git.getCommit({ ...context.repo, commit_sha: ref })).data;
    if (commit.message !== message || commit.parents?.length !== 1 || commit.parents[0].sha !== context.sha)
      throw new Error("candidate branch occupied; maintainer reconciliation required");
  } else {
    const base = (await github.rest.git.getCommit({ ...context.repo, commit_sha: context.sha })).data;
    const tree = (await github.rest.git.createTree({ ...context.repo, base_tree: base.tree.sha,
      tree: Object.entries(changes).map(([file, content]) => ({ path: file, mode: "100644", type: "blob", content })) })).data.sha;
    ref = (await github.rest.git.createCommit({ ...context.repo, message, tree, parents: [context.sha] })).data.sha;
    if (await defaultHead() !== context.sha) throw new Error("source revision moved; rerun on default branch");
    await github.rest.git.createRef({ ...context.repo, ref: `refs/heads/${branch}`, sha: ref });
  }
  const body = ownedBody(marker, `## Summary\n\nPrepare this range-only candidate for native acceptance and review.\n\n${evidence(result)}\n\n## Test changes\n\nOnly directly necessary maximum/next-refusal expectations change; the minimum and refusal behavior remain.\n\n## Test plan\n\nContract commands and exits are recorded above. Native acceptance: PENDING.\nA maintainer must attach exact-version native acceptance and review this range-only patch before merge.`);
  try {
    return { status: "draft", url: (await github.rest.pulls.create({ ...context.repo, head: branch, base: "main", title, body, draft: true })).data.html_url };
  } catch { throw new Error(CREATION_UNAVAILABLE); }
}
async function runReleaseCheck({ github, context, core, root = process.cwd(), checked, patch, fetchJSON = getJSON,
  artifactDir = "claude-release-check", writeResult = (result) => {
    fs.mkdirSync(artifactDir, { recursive: true }); fs.writeFileSync(path.join(artifactDir, "result.json"), `${JSON.stringify(result, null, 2)}\n`);
  } }) {
  const current = range(root);
  const result = { schema_version: 1, source_sha: context.sha, minimum: current.min, maximum: current.max,
    newest: null, status: "error", native_acceptance: "PENDING", check: null,
    run_url: `https://github.com/${context.repo.owner}/${context.repo.repo}/actions/runs/${context.runId}` };
  try {
    const repo = (await github.rest.repos.get(context.repo)).data;
    if (repo.default_branch !== "main" || context.ref !== "refs/heads/main") throw new Error("default-branch-only execution required");
    const release = await validateCollected({ checked, patch, root, sourceSha: context.sha, fetchJSON });
    result.newest = release.version;
    if (compare(release.version, current.max) <= 0) { result.status = "current"; result.action = "In-range or older; no candidate installation or provider writes."; }
    else {
      result.check = checked.check;
      if (!result.check.passed) {
        result.status = "failed"; result.action = "Isolated installation or module contract failed; inspect command artifacts and keep the current range.";
        result.issue_url = await failureIssue({ github, context, result });
        core.setFailed(result.action);
      } else {
        result.status = "draft";
        const candidate = await prepareDraft({ github, context, root, result }).catch(error => {
          if (error.status) throw new Error(CREATION_UNAVAILABLE);
          throw error;
        });
        result.status = candidate.status; result.candidate_url = candidate.url;
        result.action = candidate.status === "preserved" ? "Existing human/review state preserved; maintainer must continue that candidate." : "Draft prepared; native acceptance and reviewed merge remain required.";
      }
    }
  } catch (error) {
    result.status = "error";
    const safe = ["default-branch-only execution required", "source revision moved; rerun on default branch",
      "candidate branch occupied; maintainer reconciliation required", "duplicate candidate PRs; maintainer reconciliation required",
      "duplicate failure issues; maintainer reconciliation required", "human-owned failure evidence; maintainer reconciliation required"];
    result.action = safe.includes(error.message) || error.message.startsWith("candidate creation unavailable;")
      ? error.message : "Release preparation failed; maintainer must inspect the retained artifacts and retry without changing the current range.";
    if (result.newest && compare(result.newest, current.max) > 0 && context.ref === "refs/heads/main") {
      try { result.issue_url = await failureIssue({ github, context, result }); }
      catch { result.action += " Failure issue unavailable or human-owned; maintainer must reconcile it manually."; }
    }
    core.setFailed(result.action);
  } finally {
    writeResult(result); await core.summary.addRaw(evidence(result)).write();
  }
  return result;
}
module.exports = { collectReleaseCheck, range, compare, discover, verifyIntegrity, isolatedEnvironment, candidateChanges, checkCandidate, runReleaseCheck };
