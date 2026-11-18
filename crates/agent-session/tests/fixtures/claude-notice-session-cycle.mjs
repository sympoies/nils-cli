// Model a Claude session.end / session.start pair in the same terminal runtime.
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { setImmediate } from 'node:timers/promises';

const source = await readFile(process.argv[2], 'utf8');
const { register } = await import('data:text/javascript;base64,' + Buffer.from(source).toString('base64'));
const hooks = new Map();
register((event, callback) => hooks.set(event, callback));

const schema = name => 'agent-session.claude-notice-' + name + '.v1';
const boot = {
  schema_version: schema('runtime'),
  launch: { session_id: 'managed-session', incarnation: 'fixture-launch', generation: 1, profile: null },
  module_sha256: 'a'.repeat(64),
  provider_version: '2.1.296',
  socket_path: '/fixture/notice.sock',
  nonce: 'b'.repeat(64),
};
const origin = { kind: 'plugin', name: 'agent-session-claude-notice', asUser: true };
const submits = [];
const reports = [];
const authorizations = [];
const running = new Set();
let offer = {};
let readiness;
let draft = '';
let clears = 0;
let holdAuthorization = false;
let releaseAuthorization;
const api = {
  plugin: { root: '/fixture' },
  fs: {
    async read(path) { return JSON.stringify(path.endsWith('/bootstrap.json') ? boot : offer); },
    async write(_path, text) { readiness = JSON.parse(text); },
  },
  env: {
    async get(name) {
      return { AGENT_SESSION_ID: boot.launch.session_id, AGENT_SESSION_RUNTIME_ID: boot.launch.incarnation }[name];
    },
  },
  session: {
    async id() { return 'fixture-conversation'; },
    async version() { return { version: boot.provider_version }; },
    async surfaces() { return ['terminal']; },
  },
  prompt: {
    async read() { return { text: draft, cursor: draft.length }; },
    async submit(input) { submits.push(input); return { ...input, origin }; },
    async clear() { clears++; draft = ''; },
  },
  clock: {
    every(_ms, tick) {
      running.add(tick);
      return { cancel() { running.delete(tick); } };
    },
  },
  http: {
    async fetch(url, request) {
      const value = JSON.parse(request.body);
      assert.equal(request.socketPath, boot.socket_path);
      assert.equal(value.nonce, boot.nonce);
      assert.equal(value.op, offer.op);
      assert.deepEqual(value.binding, readiness.binding);
      if (new URL(url).pathname === '/result') {
        reports.push(value);
        return { ok: true, status: 200 };
      }
      authorizations.push(value.op);
      if (!value.box_empty || value.cursor !== 0) return { ok: false, status: 409 };
      if (holdAuthorization) await new Promise(resolve => { releaseAuthorization = resolve; });
      return { ok: true, status: 200, text: JSON.stringify({
        schema_version: schema('authorized'),
        op: offer.op,
        binding: offer.binding,
        text: 'notice',
        envelope_bytes: 6,
        envelope_sha256: 'c'.repeat(64),
      }) };
    },
  },
};
const event = { isInteractive: true, surface: 'terminal' };
const start = () => hooks.get('session.start')(api, event, async value => value);
const end = () => hooks.get('session.end')(api, {}, async value => value);
async function poll() {
  assert.equal(running.size, 1);
  for (const tick of running) tick();
  await setImmediate();
}
function prepare(op) {
  offer = { schema_version: schema('offer'), op, binding: readiness.binding,
    socket_path: boot.socket_path, nonce: boot.nonce };
}

await start();
prepare('11111111-1111-4111-8111-111111111111');
await poll();
assert.equal(reports[0]?.outcome, 'accepted');
await end();
assert.equal(readiness.state, 'unavailable');
assert.equal(running.size, 0);
await start();
assert.equal(readiness.timer_started, true, 'the next interactive start must restore native notice readiness');
await poll(); // The last offer can still be visible after the restart.
assert.equal(submits.length, 1);
assert.equal(authorizations.length, 1);
prepare('22222222-2222-4222-8222-222222222222');
await poll();
assert.equal(reports[1]?.outcome, 'accepted');
assert.deepEqual(submits, Array(2).fill({ text: 'notice', asUser: true }));

draft = 'operator draft';
prepare('33333333-3333-4333-8333-333333333333');
await poll();
assert.equal(submits.length, 2);
assert.equal(draft, 'operator draft');
assert.equal(clears, 0);

draft = '';
holdAuthorization = true;
prepare('44444444-4444-4444-8444-444444444444');
await poll();
assert.equal(typeof releaseAuthorization, 'function');
await end();
await start();
releaseAuthorization();
await setImmediate();
await poll(); // The replacement must not reauthorize the old operation.
assert.equal(submits.length, 2);
assert.equal(reports[2]?.outcome, 'unknown');
assert.equal(reports.length, 3);
assert.equal(authorizations.length, 4);
