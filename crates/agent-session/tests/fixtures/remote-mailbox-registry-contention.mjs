// Real separate-root CLI/HTTP acceptance for a queued remote send whose follow-up calls contend
// on the source registry lock. The lock is held by a synthetic flock holder; nothing else is shared.
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';

const HOLDER = 'import fcntl,sys\nf=open(sys.argv[1],"a")\nfcntl.flock(f,fcntl.LOCK_EX)\nprint("locked",flush=True)\nsys.stdin.read()\n';

async function holdRegistryLock(host) {
  const child = spawn('python3', ['-c', HOLDER, join(host.root, 'coordination', 'registry.lock')], { stdio: ['pipe', 'pipe', 'inherit'] });
  await new Promise((resolve, reject) => {
    child.stdout.once('data', resolve);
    child.once('error', reject);
    child.once('exit', () => reject(new Error('lock holder exited early')));
  });
  return () => { child.stdin.end(); return new Promise(resolve => child.once('exit', resolve)); };
}

// Wall-clock a CLI call; `cli` asserts the exit code and returns the parsed envelope.
async function timed(cli, host, args, expect) {
  const started = Date.now();
  const output = await cli(host, args, expect);
  return { output, elapsed: Date.now() - started };
}

export async function registryContentionAcceptance({ alpha, beta, cli, bodyFile, delivered, relay, waitFor, checks }) {
  const copies = id => JSON.parse(readFileSync(join(beta.root, 'coordination', 'registry.json'), 'utf8')).messages.filter(m => m.message_id === id).length;
  const sendArgs = (label, key = label) => ['send', '--from', alpha.session, '--to-machine', beta.machine, '--to', beta.session,
    '--body-file', bodyFile(label, label), '--idempotency-key', key];

  relay.block();
  const queued = await cli(alpha, sendArgs('contention-queued'));
  assert.equal(queued.state, 'queued');
  await waitFor(() => relay.blocked(), 'relay holds the queued message');
  const release = await holdRegistryLock(alpha);
  try {
    // A queued remote message is answered from the federation journal, not the registry.
    const lookup = await timed(cli, alpha, ['delivery', '--session', alpha.session, '--message', queued.message_id], 0);
    assert.equal(lookup.output.message_id, queued.message_id);
    assert.equal(lookup.output.state, 'queued');
    assert(lookup.elapsed < 1500, `delivery lookup waited ${lookup.elapsed}ms on the registry lock`);
    // The same idempotency key replays the stored outcome without the registry.
    const replay = await timed(cli, alpha, sendArgs('contention-queued'), 0);
    assert.equal(replay.output.message_id, queued.message_id);
    assert(replay.elapsed < 1500, `same-key send waited ${replay.elapsed}ms on the registry lock`);
    // A different body under that key is still refused without the registry.
    const conflict = await cli(alpha, sendArgs('contention-different-body', 'contention-queued'), 65);
    assert.equal(conflict.error.code, 'idempotency-key-conflict');
    // A first submit still needs the registry; its bounded timeout must say how to recover.
    const fresh = await cli(alpha, sendArgs('contention-fresh'), 65);
    assert.equal(fresh.error.code, 'coordination-lock-timeout');
    assert.equal(fresh.error.details.retryable, true);
    assert.equal(fresh.error.details.next_action, 'retry-same-idempotency-key');
    // An unknown id can only be a same-machine id, which lives in the registry.
    const unknown = await cli(alpha, ['delivery', '--session', alpha.session, '--message', 'contention-unknown-id'], 65);
    assert.equal(unknown.error.code, 'coordination-lock-timeout');
    assert.equal(unknown.error.details.retryable, true);
    assert.equal(unknown.error.details.next_action, 'retry-delivery-check');
    relay.release();
    await waitFor(() => copies(queued.message_id) === 1, 'destination persisted while the source registry was locked');
  } finally {
    await release();
    relay.release();
    // Let the released relay request finish so a failed assertion is reported instead of a reset socket.
    await waitFor(() => copies(queued.message_id) === 1, 'released relay request drains').catch(() => {});
  }
  await delivered(alpha, queued.message_id);
  const again = await cli(alpha, sendArgs('contention-queued'));
  assert.equal(again.message_id, queued.message_id);
  await delay(300);
  assert.equal(copies(queued.message_id), 1);
  checks.push('queued-remote-send-recovers-under-registry-contention');
}
