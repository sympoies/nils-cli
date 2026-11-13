// Execute the mounted module against synthetic native APIs and the real notice bridge.
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import http from 'node:http';
import { join } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';

const [root, module, mode] = process.argv.slice(2);
const boot = JSON.parse(await fs.readFile(join(module, 'bootstrap.json'), 'utf8'));
const source = await fs.readFile(join(module, 'hooks/register.js'), 'utf8');
const { register } = await import('data:text/javascript;base64,' + Buffer.from(source).toString('base64'));
const hooks = new Map();
register((event, callback) => hooks.set(event, callback));
const trace = { reads: [], authorizations: 0, submits: [], clears: 0, reports: [] };
let tick;
let offered = false;
let done = false;
const prompt = {
  async read() {
    const text = await fs.readFile(join(root, 'composer'), 'utf8');
    const cursor = mode === 'cursor-before' ? 1 : text.length;
    trace.reads.push({ text, cursor });
    return { text, cursor };
  },
  async submit(value) { trace.submits.push(value); throw new Error('unexpected submit'); },
  async clear() { trace.clears++; throw new Error('unexpected clear'); },
};
const api = {
  plugin: { root: module },
  fs: {
    async read(path) {
      const text = await fs.readFile(path, 'utf8');
      if (path.endsWith('/offer.json') && JSON.parse(text).op) offered = true;
      return text;
    },
    write: (path, text) => fs.writeFile(path, text),
  },
  env: { get: async name => ({ AGENT_SESSION_ID: boot.launch.session_id,
    AGENT_SESSION_RUNTIME_ID: boot.launch.incarnation })[name] },
  session: {
    id: async () => 'provider-one',
    version: async () => ({ version: boot.provider_version }),
    surfaces: async () => ['terminal'],
  },
  prompt,
  clock: { every(_ms, callback) { tick = callback; return { cancel() {} }; } },
  http: {
    async fetch(url, options) {
      const path = new URL(url).pathname;
      if (path === '/authorize') trace.authorizations++;
      else trace.reports.push(JSON.parse(options.body));
      const response = await new Promise((resolve, reject) => {
        const request = http.request({ socketPath: options.socketPath, path,
          method: options.method, headers: options.headers }, res => {
          let text = '';
          res.setEncoding('utf8');
          res.on('data', chunk => { text += chunk; });
          res.on('end', () => {
            if (path === '/authorize' && res.statusCode !== 200) done = true;
            resolve({ ok: res.statusCode === 200, status: res.statusCode, text });
          });
        });
        request.setTimeout(5000, () => request.destroy(new Error('bridge timeout')));
        request.on('error', error => {
          if (path === '/authorize') done = true;
          reject(error);
        });
        request.end(options.body);
      });
      if (path === '/authorize' && response.ok && mode === 'draft-after') {
        await fs.writeFile(join(root, 'composer'), 'new human draft');
      }
      if (path === '/result') done = true;
      return response;
    },
  },
};
if (mode === 'draft-before') await fs.writeFile(join(root, 'composer'), 'native human draft');
await hooks.get('session.start')(api, { isInteractive: true, surface: 'terminal' }, async e => e);
assert.equal(typeof tick, 'function');
const deadline = Date.now() + 10000;
while (!done && Date.now() < deadline) {
  tick();
  await delay(2);
  if (offered && trace.reads.length && trace.authorizations === 0) done = true;
}
assert.ok(done, 'module did not finish the synthetic offer');
await delay(10);
await hooks.get('session.end')(api, {}, async e => e);
console.log(JSON.stringify(trace));
