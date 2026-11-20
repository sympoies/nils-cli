// Local service reply acceptance. The parent fixture owns private roots and daemons.
import assert from 'node:assert/strict';
import { randomUUID } from 'node:crypto';
import { chmodSync, existsSync, readFileSync } from 'node:fs';
import { join } from 'node:path';

export function serviceReplyCases({
  root, source, service, origin, serviceEntry, admission, privateWrite, registry, runCli, payload, submit,
}) {
  const replyBody = 'synthetic reply';
  async function postReply(host, parent, revision, key) {
    const result = await fetch(host.url + `/sessions/${host.session}/messages/${parent}/reply/v1`, {
      method: 'POST',
      headers: {
        Authorization: `Bearer ${host.operator}`,
        'X-Agent-Session-Capability': host.capability,
        'Content-Type': 'application/json',
      },
      body: JSON.stringify({ body: replyBody, if_revision: revision, idempotency_key: key }),
    });
    return result.json();
  }
  async function poll(principal, path = '', { query = {}, ack, token = principal.token, headers = {} } = {}) {
    const selector = new URLSearchParams({ service_id: principal.id, service_generation: principal.generation, ...query });
    const result = await fetch(source.url + '/coordination/services/messages' + path + '/v1?' + selector, {
      method: ack ? 'POST' : 'GET',
      headers: { Authorization: `Bearer ${token}`, 'Content-Type': 'application/json', ...headers },
      body: ack ? JSON.stringify(ack) : undefined,
    });
    const value = await result.json();
    assert(!JSON.stringify(value).includes(principal.token), 'service token leaked into the response');
    return value;
  }
  function replyCli(host, command, parent, key, flags = [], expected = 0) {
    const bodyFile = join(root, 'session-reply-body');
    privateWrite(bodyFile, replyBody);
    return runCli(['--state-dir', host.root, 'message', command,
      ...(command === 'reply' ? ['--session', host.session, '--message', parent] :
        ['--from', host.session, '--reply-to', parent]),
      '--capability-file', host.capabilityFile, '--body-file', bodyFile,
      '--idempotency-key', key, '--format', 'json', ...flags], expected);
  }
  function readServiceCli(principal, operation, flags = []) {
    return runCli(['--state-dir', source.root, 'message', 'service-' + operation,
      '--service', principal.id, '--service-generation', principal.generation,
      '--credential-file', principal.credential, '--format', 'json', ...flags]);
  }
  function parentMarker(host, parent) {
    return registry(host).messages.find(message => message.message_id === parent).reply_sent_at_epoch;
  }
  async function assertReplyRefused(host, parent, revision, key, code) {
    const before = registry(host);
    const result = await postReply(host, parent, revision, key);
    assert.equal(result.error?.code, code, 'reply refusal');
    const after = registry(host);
    assert.deepEqual(after.messages, before.messages, 'refusal must preserve the parent and mailbox');
    assert.deepEqual(after.receipts, before.receipts, 'refusal must not create a receipt');
    const path = join(host.root, 'coordination', 'federation-journal.json');
    if (existsSync(path)) {
      const journal = JSON.parse(readFileSync(path, 'utf8'));
      assert(![...journal.remote_outbox, ...journal.retained].some(item => item.idempotency_key === key));
    }
  }
  async function exerciseLocalPolling(parent, revision, httpReply) {
    const second = {
      id: 'second-reporter', generation: 'generation-1',
      token: randomUUID() + randomUUID(), credential: join(root, 'second-service-credential'),
    };
    privateWrite(second.credential, second.token);
    admission([serviceEntry(service), serviceEntry(second)]);
    const recipient = { kind: 'service', ...origin() };
    assert.deepEqual(httpReply.recipient, recipient);
    assert.equal(httpReply.reply_to, parent);
    assert(parentMarker(source, parent), 'parent reply marker');
    assert.equal((await postReply(source, parent, revision, 'local-service-reply')).data.coordination.message_id, httpReply.message_id);

    const sent = await replyCli(source, 'send', parent, 'send-service-reply');
    assert.deepEqual(sent.recipient, recipient);
    assert.equal(sent.reply_to, parent);
    const replied = await replyCli(source, 'reply', parent, 'cli-service-reply', ['--if-revision', String(revision)]);
    assert.deepEqual(replied.recipient, recipient);
    assert.equal(replied.reply_to, parent);
    assert.equal((await replyCli(source, 'send', parent, 'send-service-reply')).message_id, sent.message_id);
    assert(!httpReply.notification);
    assert(Object.values(registry(source).notifications).every(value => value.target_session_id === source.session));

    const page = (await poll(service, '', { query: { state: 'unread', limit: 1 } })).data.coordination;
    assert.equal(page.messages.length, 1);
    assert.deepEqual(page.messages[0].recipient, recipient);
    assert(!('body' in page.messages[0]));
    assert.equal(typeof page.next_cursor, 'string');
    const next = (await poll(service, '', { query: { state: 'unread', limit: 1, cursor: page.next_cursor } })).data.coordination;
    assert.notEqual(next.messages[0].message_id, page.messages[0].message_id);
    assert.equal((await poll(second)).data.coordination.messages.length, 0);
    assert.equal((await poll(second, '', { query: { state: 'unread', limit: 1, cursor: page.next_cursor } })).error.code, 'cursor-invalid');
    assert.equal((await poll(second, '/' + sent.message_id)).error.code, 'message-not-found');
    assert.equal((await poll(second, '/' + sent.message_id + '/ack', { ack: { if_revision: 1, idempotency_key: 'foreign-ack' } })).error.code, 'message-not-found');
    assert.equal((await poll(service, '', { token: source.operator })).error.code, 'mailbox-service-unauthorized');
    assert.equal((await poll(service, '', { token: source.capability })).error.code, 'mailbox-service-unauthorized');
    assert.equal((await poll(service, '', { headers: { 'X-Forwarded-For': '127.0.0.1' } })).error.code, 'mailbox-service-forbidden');
    await assertReplyRefused(source, parent, revision - 1, 'stale-service-reply', 'message-revision-conflict');

    const shown = (await poll(service, '/' + sent.message_id)).data.coordination;
    assert.deepEqual(shown.body, { classification: 'peer_message', text: replyBody });
    assert.equal(shown.state, 'read');
    assert.equal(shown.revision, sent.revision + 1);
    const ack = { if_revision: shown.revision, idempotency_key: 'service-ack-fixture' };
    const ackPath = '/' + shown.message_id + '/ack';
    assert.equal((await poll(service, ackPath, { ack: { ...ack, if_revision: sent.revision } })).error.code, 'message-revision-conflict');
    const acknowledged = (await poll(service, ackPath, { ack })).data.coordination;
    assert.equal(acknowledged.state, 'acknowledged');
    assert(!('body' in acknowledged));
    assert.deepEqual((await poll(service, ackPath, { ack })).data.coordination, acknowledged);
    assert.equal((await poll(service, ackPath, { ack: { ...ack, if_revision: acknowledged.revision } })).error.code, 'idempotency-key-reused');
    const cliInbox = await readServiceCli(service, 'inbox', ['--state', 'unread']);
    assert.deepEqual(cliInbox.messages.map(row => row.message_id).sort(), [httpReply.message_id, replied.message_id].sort());
    const cliShown = await readServiceCli(service, 'show', ['--message', replied.message_id]);
    assert.deepEqual(cliShown.body, { classification: 'peer_message', text: replyBody });
    const cliAck = await readServiceCli(service, 'ack', ['--message', replied.message_id, '--if-revision', String(cliShown.revision), '--idempotency-key', 'cli-service-ack']);
    assert.equal(cliAck.state, 'acknowledged');
    privateWrite(second.credential, service.token);
    assert.equal((await poll(second, '', { token: service.token })).error.code, 'mailbox-service-unauthorized');
    assert.equal((await poll(service)).error.code, 'mailbox-service-unauthorized');
    admission();
  }
  async function exerciseAdmissionChanges(parent, revision, previousReply) {
    const replacement = {
      ...service, generation: 'generation-2', token: randomUUID() + randomUUID(),
      credential: join(root, 'replacement-service-credential'),
    };
    privateWrite(replacement.credential, replacement.token);
    const unanswered = await submit(payload('lifecycle-refusal-parent'));
    assert.equal(parentMarker(source, unanswered.message_id), undefined);
    admission([]);
    await assertReplyRefused(source, unanswered.message_id, unanswered.revision, 'unadmitted-reply', 'mailbox-service-recipient-unadmitted');
    admission([serviceEntry(replacement)]);
    await assertReplyRefused(source, unanswered.message_id, unanswered.revision, 'conflicting-reply', 'mailbox-service-recipient-generation-conflict');
    assert.equal((await poll(service)).error.code, 'mailbox-service-unauthorized');
    assert.equal((await poll(replacement)).data.coordination.messages.length, 0);
    assert.equal((await postReply(source, parent, revision, 'local-service-reply')).data.coordination.message_id, previousReply.message_id);
    assert.equal((await poll(replacement)).data.coordination.messages.length, 0);

    const retired = [{ service_id: service.id, service_generation: service.generation }];
    admission([], retired);
    await assertReplyRefused(source, unanswered.message_id, unanswered.revision, 'retired-reply', 'mailbox-service-recipient-retired');
    admission([serviceEntry(replacement)], retired);
    await assertReplyRefused(source, unanswered.message_id, unanswered.revision, 'retained-marker-reply', 'mailbox-service-recipient-retired');

    chmodSync(service.admission, 0o644);
    await assertReplyRefused(source, unanswered.message_id, unanswered.revision, 'unreadable-admission-reply', 'mailbox-service-unavailable');
    assert.equal((await poll(replacement)).error.code, 'mailbox-service-unavailable');
    admission([serviceEntry(service)], retired);
    await assertReplyRefused(source, unanswered.message_id, unanswered.revision, 'invalid-admission-reply', 'mailbox-service-unavailable');
    admission([serviceEntry(replacement)], retired);
    const freshParent = await submit(payload('replacement-generation-parent', { service_generation: replacement.generation }), 200, replacement.token);
    const freshReply = (await postReply(source, freshParent.message_id, freshParent.revision, 'replacement-generation-reply')).data.coordination;
    assert.equal(freshReply.recipient.service_generation, replacement.generation);
    assert.equal((await poll(replacement)).data.coordination.messages[0].message_id, freshReply.message_id);
    assert.equal((await poll(service)).error.code, 'mailbox-service-unauthorized');
  }

  return { postReply, assertReplyRefused, exerciseLocalPolling, exerciseAdmissionChanges };
}
