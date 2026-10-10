#!/usr/bin/env node
// Disposable private roots, fixture identities only; no provider/model invocation.
import assert from 'node:assert/strict';
import { createHash, randomUUID } from 'node:crypto';
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, chmodSync, rmSync, existsSync, symlinkSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { spawn } from 'node:child_process';
import { createServer } from 'node:http';
import { setTimeout as delay } from 'node:timers/promises';
import { writeHeartbeat } from './remote-mailbox-heartbeat.mjs';

const binary = process.argv[process.argv.indexOf('--agent-session-bin') + 1];
assert(binary, 'requires exact binary');
const root = mkdtempSync(join(tmpdir(), 'nils-service-mailbox-'));
const hash = value => createHash('sha256').update(value).digest('hex');
const now = () => Math.floor(Date.now() / 1000);
const privateWrite = (path, value) => { writeFileSync(path, typeof value === 'string' ? value : JSON.stringify(value), { mode: 0o600 }); chmodSync(path, 0o600); };
const machineArg = flag => process.argv.includes(flag) ? process.argv[process.argv.indexOf(flag) + 1] : null;
const hosts = ['alpha', 'beta'].map((label, index) => ({ machine: machineArg(index === 0 ? '--source-machine' : '--target-machine') ?? label, root: join(root, label), session: `${label}-recipient`, incarnation: randomUUID(), capability: randomUUID() + randomUUID(), operator: randomUUID() + randomUUID(), relay: randomUUID() + randomUUID(), ingress: randomUUID() + randomUUID() }));
const [source, target] = hosts;
const service = { id: 'reporter', generation: 'generation-1', token: randomUUID() + randomUUID(), credential: join(root, 'service-credential'), admission: join(root, 'service-admission.json') };
const address = host => ({ machine: host.machine, session_id: host.session, session_incarnation: host.incarnation });
const origin = () => ({ machine: source.machine, service_id: service.id, service_generation: service.generation });
const checks = [];
let edge, heartbeat, hold = false, dropOnce = false, discoveryDenied = false, oldRelay = false;
function admission(entries = [{ service_id: service.id, service_generation: service.generation, credential_file: service.credential }]) {
  privateWrite(service.admission, { schema_version: 'agent-session.mailbox-services.v1', services: entries });
}
function seed(host) {
  const sessionRoot = join(host.root, 'sessions', host.session);
  mkdirSync(join(sessionRoot, 'coordination'), { recursive: true, mode: 0o700 });
  mkdirSync(join(host.root, 'coordination'), { recursive: true, mode: 0o700 });
  privateWrite(join(sessionRoot, 'session.json'), { schema_version: 'agent-session.session.v1', id: host.session, agent: 'codex', mode: 'interactive', title: 'mailbox fixture', title_revision: 0, cwd: root, tmux_session: 'fixture', prompt_file: null, log_file: null, created_at: '2030-01-01T00:00:00Z', updated_at: '2030-01-01T00:00:00Z', coordination_mode: 'advisory', runtime: { kind: 'tmux', tmux_session: 'fixture', generation: 1, started_at: '2030-01-01T00:00:00Z', launch_id: host.incarnation } });
  host.capabilityFile = join(sessionRoot, 'coordination', `capability-${hash(host.incarnation)}`);
  privateWrite(host.capabilityFile, host.capability);
  writeHeartbeat(join(sessionRoot, 'coordination', 'heartbeat'), `${host.incarnation}:${now()}\n`);
  privateWrite(join(host.root, 'coordination', 'registry.json'), { schema_version: 'agent-session.coordination-registry.v2', fingerprint_epoch: 1, fingerprint_key: randomUUID() + randomUUID(), brokers: { [host.session]: { session_id: host.session, incarnation: host.incarnation, coordination_mode: 'advisory', capability_digest: hash(host.capability), generation: 1, state: 'ready', heartbeat_at: '2030-01-01T00:00:00Z', heartbeat_epoch: now() } }, claims: [], operations: [], messages: [], receipts: {}, notifications: {} });
}
async function waitFor(fn, label, ms = 25000) {
  const until = Date.now() + ms;
  while (Date.now() < until) { const value = await fn(); if (value) return value; await delay(100); }
  throw new Error(`timed out: ${label}`);
}
async function start(host) {
  const env = { ...process.env };
  for (const key of Object.keys(env)) if (key.startsWith('AGENT_SESSION_') || key.startsWith('AGENT_CONSOLE_')) delete env[key];
  Object.assign(env, { AGENT_SESSION_MAILBOX_SERVICES_FILE: host === source ? service.admission : '', AGENT_SESSION_RELAY_URL: edge.url, AGENT_SESSION_RELAY_TOKEN: host.relay, AGENT_SESSION_RELAY_INGRESS_TOKEN: host.ingress });
  const endpoint = join(host.root, 'coordination', 'daemon-endpoint.json');
  rmSync(endpoint, { force: true });
  host.child = spawn(binary, ['--state-dir', host.root, 'serve', '--bind', '127.0.0.1:0', '--machine', host.machine, '--token-stdin', '--tmux-bin', '/bin/false'], { env, stdio: ['pipe', 'ignore', 'pipe'] });
  host.child.stdin.end(host.operator);
  host.stderr = ''; host.child.stderr.on('data', data => { if (host.stderr.length < 16384) host.stderr += data.toString(); });
  host.url = await waitFor(() => { if ((host.child.exitCode !== null || host.child.signalCode !== null)) throw new Error('fixture daemon stopped: ' + host.stderr); if (existsSync(endpoint)) return JSON.parse(readFileSync(endpoint, 'utf8')).url; }, 'endpoint');
  await waitFor(async () => { try { return (await fetch(host.url + '/healthz')).ok; } catch { return false; } }, 'health');
}
async function stop(host) {
  if (!host.child) return;
  const child = host.child; if ((child.exitCode !== null || child.signalCode !== null)) { host.child = null; return; } child.kill('SIGTERM');
  await Promise.race([new Promise(resolve => child.once('exit', resolve)), delay(3000)]);
  if (child.exitCode === null && child.signalCode === null) { child.kill('SIGKILL'); await new Promise(resolve => child.once('exit', resolve)); }
  host.child = null;
}
const payload = (key, extra = {}) => ({ service_id: service.id, service_generation: service.generation, to_session: source.session, to_machine: null, body: 'PRIVATE-SERVICE-BODY-CANARY', idempotency_key: key, expires_in: null, expected_recipient_incarnation: null, ...extra });
async function submit(body, expected = 200, token = service.token, headers = {}) {
  const response = await fetch(source.url + '/coordination/services/messages/v1', { method: 'POST', headers: { Authorization: `Bearer ${token}`, 'Content-Type': 'application/json', ...headers }, body: JSON.stringify(body) });
  assert.equal(response.status, expected, 'service route admission status');
  const value = await response.json();
  if (expected !== 200 && body.body) assert(!JSON.stringify(value).includes(body.body), 'diagnostics must exclude body');
  assert(!JSON.stringify(value).includes(service.token), 'diagnostics must exclude credential');
  return value.data?.coordination ?? value.data ?? value;
}
const registry = host => JSON.parse(readFileSync(join(host.root, 'coordination', 'registry.json'), 'utf8'));
const journal = () => JSON.parse(readFileSync(join(source.root, 'coordination', 'federation-journal.json'), 'utf8'));
const outboxCount = () => existsSync(join(source.root, 'coordination', 'federation-journal.json')) ? journal().remote_outbox.length : 0;
async function serviceCli(key, extra = [], expected = 0, recipient = source.session) {
  const bodyFile = join(root, 'cli-body'); privateWrite(bodyFile, 'PRIVATE-SERVICE-BODY-CANARY');
  const env = { ...process.env }; for (const key of Object.keys(env)) if (key.startsWith('AGENT_SESSION_')) delete env[key];
  const args = ['--state-dir', source.root, 'message', 'service-send', '--service', service.id, '--service-generation', service.generation, '--credential-file', service.credential, '--to', recipient, '--body-file', bodyFile, '--idempotency-key', key, '--format', 'json', ...extra];
  const result = await new Promise((resolve, reject) => { const child = spawn(binary, args, { env, stdio: ['ignore', 'pipe', 'pipe'] }); let stdout = '', stderr = ''; child.stdout.on('data', data => stdout += data); child.stderr.on('data', data => stderr += data); child.on('error', reject); child.on('exit', code => resolve({ code, stdout, stderr })); });
  assert.equal(result.code, expected, 'unattended CLI exit');
  assert(!result.stdout.includes('PRIVATE-SERVICE-BODY-CANARY')); assert(!result.stderr.includes(service.token));
  const value = JSON.parse(result.stdout); return value.data ?? value;
}
try {
  hosts.forEach(seed); privateWrite(service.credential, service.token); admission();
  const server = createServer(async (req, res) => {
    const host = hosts.find(h => req.headers.authorization === `Bearer ${h.relay}`);
    if (!host) { res.writeHead(401); res.end(JSON.stringify({ error: { code: 'unauthorized' } })); return; }
    const url = new URL(req.url, 'http://localhost');
    if (url.pathname === '/api/coordination/peers/v1') {
      if (discoveryDenied) { res.writeHead(503); res.end(JSON.stringify({ error: { code: 'remote-messaging-unavailable' } })); return; }
      if (url.searchParams.has('source_service_id')) { assert.equal(url.searchParams.get('source_service_id'), service.id); assert.equal(url.searchParams.get('source_service_generation'), service.generation); } else { assert.equal(url.searchParams.get('source_session_id'), source.session); assert.equal(url.searchParams.get('source_incarnation'), source.incarnation); }
      res.end(JSON.stringify({ schema_version: 'agent-session.remote-peers.v1', peers: hosts.map(h => ({ ...address(h), messaging_supported: true })) })); return;
    }
    assert.equal(url.pathname, '/api/coordination/relay/v1');
    const chunks = []; for await (const chunk of req) chunks.push(chunk);
    const envelope = JSON.parse(Buffer.concat(chunks));
    const sessionForward = JSON.stringify(envelope.from) === JSON.stringify(address(source)) && envelope.forwarding;
    if (oldRelay || (!sessionForward && JSON.stringify(envelope.from) !== JSON.stringify(origin()))) { res.writeHead(403); res.end(JSON.stringify({ error: { code: 'origin-forbidden' } })); return; }
    assert.equal(envelope.schema_version, sessionForward ? 'agent-session.remote-message.v2' : (envelope.category ? 'agent-session.remote-service-message.v2' : 'agent-session.remote-service-message.v1')); assert.deepEqual(envelope.from, sessionForward ? address(source) : origin());
    if (hold) { res.writeHead(503); res.end(JSON.stringify({ error: { code: 'remote-messaging-unavailable' } })); return; }
    const response = await fetch(target.url + '/coordination/messages/receive/v1', { method: 'POST', headers: { Authorization: `Bearer ${target.operator}`, 'X-Agent-Session-Relay-Token': target.ingress, 'Content-Type': 'application/json' }, body: JSON.stringify(envelope) });
    const text = await response.text();
    if (dropOnce && response.ok) { dropOnce = false; res.destroy(); return; }
    res.writeHead(response.status, { 'Content-Type': 'application/json' }); res.end(text);
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve)); edge = { server, url: `http://127.0.0.1:${server.address().port}` };
  await Promise.all(hosts.map(start));
  heartbeat = setInterval(() => hosts.forEach(host => writeHeartbeat(join(host.root, 'sessions', host.session, 'coordination', 'heartbeat'), `${host.incarnation}:${now()}\n`)), 1000); heartbeat.unref();
  await submit(payload('unknown', { service_id: 'unknown' }), 401);
  await submit(payload('wrong-token'), 401, 'wrong-token');
  await submit(payload('operator-token'), 401, source.operator);
  await submit(payload('session-token'), 401, source.capability);
  await submit(payload('stale-generation', { service_generation: 'old' }), 401);
  await submit(payload('proxy'), 403, service.token, { 'X-Forwarded-For': '127.0.0.1' });
  await submit(payload('hybrid-origin', { source_session_id: source.session }), 400);
  assert.equal(registry(source).messages.length, 0); checks.push('unadmitted-unknown-stale-generation-and-proxy-denied');
  const local = await submit(payload('local-send'));
  const row = registry(source).messages.find(m => m.message_id === local.message_id); assert(row); assert.equal(row.body, 'PRIVATE-SERVICE-BODY-CANARY');
  assert.equal(local.sender.kind, 'service'); assert.equal(local.sender.service_id, service.id); assert(!('session_id' in local.sender));
  assert.equal(local.sender.machine, source.machine);
  const inboxResponse = await fetch(source.url + `/sessions/${source.session}/messages/v1`, { headers: { Authorization: `Bearer ${source.operator}`, 'X-Agent-Session-Capability': source.capability } });
  const inbox = await inboxResponse.json(); assert(!JSON.stringify(inbox).includes('PRIVATE-SERVICE-BODY-CANARY'));
  const showResponse = await fetch(source.url + `/sessions/${source.session}/messages/${local.message_id}/v1`, { headers: { Authorization: `Bearer ${source.operator}`, 'X-Agent-Session-Capability': source.capability } });
  const show = await showResponse.json(); assert.equal(show.data.coordination.body.classification, 'untrusted_service_data'); assert.equal(show.data.coordination.body.text, row.body);
  assert.equal(show.data.coordination.sender.machine, source.machine);
  const replyResponse = await fetch(source.url + `/sessions/${source.session}/messages/${local.message_id}/reply/v1`, { method: 'POST', headers: { Authorization: `Bearer ${source.operator}`, 'X-Agent-Session-Capability': source.capability, 'Content-Type': 'application/json' }, body: JSON.stringify({ body: 'reply', idempotency_key: 'service-reply-unsupported', if_revision: show.data.coordination.revision }) });
  const reply = await replyResponse.json(); assert.equal(reply.error.code, 'mailbox-service-reply-unsupported');
  assert.equal(local.sender.service_generation, service.generation); assert(!JSON.stringify(local).includes(row.body)); checks.push('explicit-local-service-origin');
  await submit(payload('abbreviated-recipient', { to_session: 'alpha' }), 404);
  assert.equal((await submit(payload('local-send'))).message_id, local.message_id);
  await submit(payload('local-send', { body: 'changed-body' }), 409); checks.push('dedup-and-key-conflict');
  await submit(payload('local-send', { to_machine: target.machine, to_session: target.session }), 409);
  await submit(payload('empty-body', { body: '' }), 422);
  await submit(payload('oversize', { body: 'x'.repeat(16385) }), 422);
  for (const expires_in of ['0s', '8d']) await submit(payload('expiry-' + expires_in, { expires_in }), 400);
  await submit(payload('control-body', { body: 'private\u0000body' }), 422); await submit(payload('maximum-bounds', { body: 'x'.repeat(16384), expires_in: '7d' })); checks.push('body-and-expiry-bounds');
  await serviceCli('unattended-cli'); admission([]); await serviceCli('cli-denied', [], 65); admission(); checks.push('cli-without-session-environment');
  admission([]); await submit(payload('revoked'), 401); await submit(payload('local-send'), 401); admission();
  chmodSync(service.credential, 0o644); await submit(payload('unsafe-credential'), 401); chmodSync(service.credential, 0o600);
  const saved = service.credential; service.credential = join(root, 'symlink-credential'); symlinkSync(saved, service.credential); admission(); await submit(payload('symlink'), 401); service.credential = saved; admission();
  chmodSync(service.admission, 0o644); await submit(payload('unsafe-admission'), 401); chmodSync(service.admission, 0o600); checks.push('revocation-and-unsafe-files');
  const sessionPath = join(source.root, 'sessions', source.session, 'session.json'); const original = JSON.parse(readFileSync(sessionPath, 'utf8')); const replacement = structuredClone(original); replacement.runtime.launch_id = randomUUID(); privateWrite(sessionPath, replacement);
  assert.equal((await submit(payload('local-send'))).message_id, local.message_id);
  await submit(payload('stale-local', { expected_recipient_incarnation: source.incarnation }), 409);
  await submit(payload('unready-local'), 503);
  assert.equal(registry(source).messages.filter(m => m.message_id === local.message_id).length, 1); privateWrite(sessionPath, original); checks.push('local-stale-recipient-and-stable-replay');
  const outboxBeforeRefusals = outboxCount();
  for (const [key, selectors] of [
    ['absent-remote-session', { to_machine: target.machine, to_session: 'wrong-recipient' }],
    ['absent-remote-machine', { to_machine: 'wrong-host', to_session: target.session }],
  ]) {
    const rejected = await submit(payload(key, selectors), 422);
    assert.equal(rejected.error.code, 'remote-recipient-not-discovered');
    assert.equal(rejected.error.details.retryable, false);
    assert.equal(rejected.error.details.next_action, 'inspect_submission_contract');
    assert.equal(rejected.error.details.recovery.kind, 'operator');
    assert.match(rejected.error.message, /exact --to session ID and --to-machine/);
    assert.match(rejected.error.message, /message peers/);
  }
  const cliAbsent = await serviceCli('cli-absent-remote', ['--to-machine', target.machine], 65, 'wrong-recipient');
  assert.equal(cliAbsent.error.code, 'remote-recipient-not-discovered');
  assert.equal(cliAbsent.error.details.retryable, false);
  assert.match(cliAbsent.error.message, /message peers/);
  assert.equal(outboxCount(), outboxBeforeRefusals);
  checks.push('absent-remote-address-nonretryable-through-service-http-and-cli');
  discoveryDenied = true;
  const unavailable = await submit(payload('discovery-failure', { to_machine: target.machine, to_session: target.session }), 503);
  assert.equal(unavailable.error.code, 'remote-messaging-unavailable');
  assert.equal(unavailable.error.details.retryable, true);
  assert.equal(unavailable.error.details.next_action, 'retry_same_submission');
  discoveryDenied = false;
  oldRelay = true;
  const incompatibleBody = payload('old-relay', { to_machine: target.machine, to_session: target.session });
  const incompatible = await submit(incompatibleBody);
  await waitFor(async () => (await submit(incompatibleBody)).state === 'rejected', 'old relay rejects retained origin');
  assert(journal().remote_outbox.some(i => i.envelope.message_id === incompatible.message_id && i.state === 'rejected' && i.envelope.body === incompatibleBody.body)); oldRelay = false; checks.push('old-relay-rejection-retains-receipt');
  hold = true;
  const remoteBody = payload('remote-send', { to_machine: target.machine, to_session: target.session });
  const remote = await submit(remoteBody); assert.equal(remote.state, 'queued');
  await waitFor(() => journal().remote_outbox.some(i => i.envelope.message_id === remote.message_id && i.attempts > 0), 'transport failure retained');
  assert.equal((await submit(remoteBody)).message_id, remote.message_id); assert.equal(journal().remote_outbox.filter(i => i.envelope.message_id === remote.message_id).length, 1);
  const conflict = await submit({ ...remoteBody, body: 'changed-remote-body' }, 409);
  assert.equal(conflict.error.code, 'idempotency-key-conflict');
  assert.equal(journal().remote_outbox.filter(i => i.envelope.message_id === remote.message_id).length, 1);
  assert.equal(journal().remote_outbox.find(i => i.envelope.message_id === remote.message_id).envelope.body, remoteBody.body);
  checks.push('remote-changed-retry-conflict-retains-original');
  const queuedEnvelope = journal().remote_outbox.find(i => i.envelope.message_id === remote.message_id).envelope;
  assert.equal(queuedEnvelope.from.machine, source.machine); assert.equal(queuedEnvelope.to.machine, target.machine);
  for (const field of ['machine', 'service_generation']) {
    const tampered = structuredClone(queuedEnvelope); tampered.from[field] = 'tampered';
    const denied = await fetch(edge.url + '/api/coordination/relay/v1', { method: 'POST', headers: { Authorization: `Bearer ${source.relay}`, 'Content-Type': 'application/json' }, body: JSON.stringify(tampered) }); assert.equal(denied.status, 403);
  }
  for (const headers of [{}, { Authorization: `Bearer ${target.operator}`, 'X-Agent-Session-Relay-Token': 'wrong-ingress' }]) {
    const denied = await fetch(target.url + '/coordination/messages/receive/v1', { method: 'POST', headers: { ...headers, 'Content-Type': 'application/json' }, body: JSON.stringify(queuedEnvelope) }); assert.equal(denied.status, 401);
  }
  const hybrid = structuredClone(queuedEnvelope); hybrid.from.session_id = 'fabricated';
  const hybridDenied = await fetch(target.url + '/coordination/messages/receive/v1', { method: 'POST', headers: { Authorization: `Bearer ${target.operator}`, 'X-Agent-Session-Relay-Token': target.ingress, 'Content-Type': 'application/json' }, body: JSON.stringify(hybrid) }); assert.equal(hybridDenied.status, 400);
  const mismatchedSchema = structuredClone(queuedEnvelope); mismatchedSchema.schema_version = 'agent-session.remote-message.v1';
  const mismatchDenied = await fetch(target.url + '/coordination/messages/receive/v1', { method: 'POST', headers: { Authorization: `Bearer ${target.operator}`, 'X-Agent-Session-Relay-Token': target.ingress, 'Content-Type': 'application/json' }, body: JSON.stringify(mismatchedSchema) }); assert.equal(mismatchDenied.status, 422);
  checks.push('source-tuple-tamper-and-unauthorized-ingress-denied');
  hold = false; dropOnce = true;
  await waitFor(() => registry(target).messages.some(m => m.message_id === remote.message_id), 'destination persisted');
  await waitFor(async () => (await submit(remoteBody)).state === 'delivered', 'lost response retry');
  assert.equal(registry(target).messages.filter(m => m.message_id === remote.message_id).length, 1); checks.push('remote-forward-transport-failure-and-response-loss-dedup');
  const remoteShowResponse = await fetch(target.url + `/sessions/${target.session}/messages/${remote.message_id}/v1`, { headers: { Authorization: `Bearer ${target.operator}`, 'X-Agent-Session-Capability': target.capability } });
  assert.equal(remoteShowResponse.status, 200);
  const remoteShow = await remoteShowResponse.json(); assert.equal(remoteShow.data.coordination.sender.machine, source.machine);
  checks.push('existing-machine-identity-provenance-preserved');
  const taggedLocal = await serviceCli('tagged-local', ['--category', 'progress']);
  assert.equal(taggedLocal.category, 'progress');assert.equal(taggedLocal.sender.kind, 'service');
  const taggedRemoteBody = payload('tagged-remote', { to_machine: target.machine, to_session: target.session, category: 'handoff' });
  const taggedRemote = await submit(taggedRemoteBody);
  await waitFor(async () => (await submit(taggedRemoteBody)).state === 'delivered', 'tagged service remote delivery');
  assert.equal(registry(target).messages.find(m => m.message_id === taggedRemote.message_id).category, 'handoff');
  const serviceForward = await fetch(source.url + `/sessions/${source.session}/messages/remote/v1`, { method: 'POST', headers: { Authorization: `Bearer ${source.capability}`, 'Content-Type': 'application/json' }, body: JSON.stringify({
    to_machine: target.machine, to_session: target.session, body: '', idempotency_key: 'forward-service', reply_to: null, expires_in: null, reply_revision: null,
    forward: { message: taggedLocal.message_id, if_revision: 1, categories: ['progress'] },
  }) });
  assert.equal(serviceForward.status, 200);
  const serviceCopy = await serviceForward.json();const serviceCopyId = serviceCopy.message_id;
  await waitFor(() => registry(target).messages.some(m => m.message_id === serviceCopyId), 'recipient-forwarded service delivery');
  const serviceShownResponse = await fetch(target.url + `/sessions/${target.session}/messages/${serviceCopyId}/v1`, { headers: { Authorization: `Bearer ${target.operator}`, 'X-Agent-Session-Capability': target.capability } });
  assert.equal(serviceShownResponse.status, 200);
  const serviceShown = (await serviceShownResponse.json()).data.coordination;
  assert.equal(serviceShown.body.classification, 'untrusted_service_data');
  assert.equal(serviceShown.forwarding.original_sender.service_id, service.id);
  assert.equal(serviceShown.sender.session_id, source.session);assert.equal(serviceShown.category, 'progress');
  checks.push('service-categories-forward-provenance-preserve-authority');
  hold = true; const movedBody = payload('moved-remote', { to_machine: target.machine, to_session: target.session }); const moved = await submit(movedBody);
  const targetPath = join(target.root, 'sessions', target.session, 'session.json'); const changed = JSON.parse(readFileSync(targetPath, 'utf8')); changed.runtime.launch_id = randomUUID(); privateWrite(targetPath, changed);
  hold = false; const fenced = await waitFor(async () => { const value = await submit(movedBody); return ['rejected', 'delivery-unknown'].includes(value.state) ? value : false; }, 'moved recipient fenced');
  assert.equal(fenced.reason, 'session-incarnation-conflict');
  assert(journal().remote_outbox.some(i => i.envelope.message_id === moved.message_id && i.envelope.body === movedBody.body)); assert(!registry(target).messages.some(m => m.message_id === moved.message_id));
  assert.equal((await submit(movedBody)).recipient.session_incarnation, target.incarnation); checks.push('remote-no-retarget-or-queue-loss');
  for (const host of hosts) { assert(!host.stderr.includes(service.token)); assert(!host.stderr.includes('PRIVATE-SERVICE-BODY-CANARY')); }
  console.log(JSON.stringify({ schema_version: 'agent-session.service-mailbox-acceptance.v1', status: 'passed', checks }));
} finally {
  clearInterval(heartbeat); await Promise.all(hosts.map(stop)); if (edge) { edge.server.closeAllConnections(); await new Promise(resolve => edge.server.close(resolve)); } rmSync(root, { recursive: true, force: true });
}
