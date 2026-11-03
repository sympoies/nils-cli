// Real separate-root CLI/HTTP deadline transport and reply-obligation acceptance.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';

export async function replyDeadlineAcceptance({alpha,beta,cli,bodyFile,delivered,inspect,checks}) {
  const invalidHttp=await fetch(beta.url+`/sessions/${beta.session}/messages/v1`,{
    method:'POST',headers:{Authorization:`Bearer ${beta.operator}`,'X-Agent-Session-Capability':beta.capability,'Content-Type':'application/json'},
    body:JSON.stringify({body:'invalid HTTP deadline',idempotency_key:'invalid-http-deadline',reply_by:'0'})
  });
  assert.equal(invalidHttp.status,400);
  assert.equal((await invalidHttp.json()).error.code,'mailbox-reply-by-invalid');
  const localResponse=await fetch(beta.url+`/sessions/${beta.session}/messages/v1`,{
    method:'POST',headers:{Authorization:`Bearer ${beta.operator}`,'X-Agent-Session-Capability':beta.capability,'Content-Type':'application/json'},
    body:JSON.stringify({body:'local HTTP deadline',idempotency_key:'local-http-deadline',reply_by:'1h'})
  });
  assert.equal(localResponse.status,200);
  const localEnvelope=await localResponse.json();
  const local=localEnvelope.data.coordination;
  assert.equal(typeof local.reply_by,'string');
  const deadlineBody=bodyFile('reply-deadline','deadline body');
  const args=['send','--from',alpha.session,'--to-machine',beta.machine,'--to',beta.session,
    '--body-file',deadlineBody,'--idempotency-key','reply-deadline','--reply-by','1s'];
  const sent=await cli(alpha,args);
  await delivered(alpha,sent.message_id);
  const shown=await inspect(beta,sent.message_id,'deadline body');
  assert.equal(typeof shown.reply_by,'string');
  const audit=async()=>{
    const response=await fetch(beta.url+'/coordination/messages/audit/v1?include_healthy=true',{
      headers:{Authorization:`Bearer ${beta.operator}`}
    });
    assert.equal(response.status,200);
    const page=await response.json();
    const data=page.data;
    assert.equal(data.schema_version,'agent-session.mail-audit.v1');
    return data.records.find(r=>r.message_id===sent.message_id);
  };
  assert.equal((await audit()).reply_by,shown.reply_by);
  await delay(Math.max(0,Date.parse(shown.reply_by)-Date.now())+1100);
  assert((await audit()).anomalies.includes('overdue-reply'));
  const reply=await cli(beta,['reply','--session',beta.session,'--message',sent.message_id,
    '--if-revision',String(shown.revision),'--body-file',bodyFile('deadline-answer','answered'),
    '--idempotency-key','deadline-answer']);
  await delivered(beta,reply.message_id);
  const row=await audit();
  assert(!row.anomalies.includes('overdue-reply'));
  assert.equal(typeof row.replied_at,'string');
  const replay=await cli(alpha,args);
  assert.equal(replay.message_id,sent.message_id);
  assert.equal(replay.reply_by,shown.reply_by);
  const stored=JSON.parse(readFileSync(join(beta.root,'coordination','registry.json'),'utf8')).messages.find(m=>m.message_id===sent.message_id);
  assert.equal(stored.reply_by_epoch*1000,Date.parse(shown.reply_by));
  const invalid=await cli(alpha,[...args.slice(0,-3),'invalid-deadline','--reply-by','8d'],64);
  assert.equal(invalid.error.code,'mailbox-reply-by-invalid');
  checks.push('reply-deadline-cli-http-transport-overdue-and-cleared');
}
