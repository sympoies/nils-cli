#!/usr/bin/env node
// Installed-artifact fixture: Node builtins only; private roots and exact child cleanup.
import assert from 'node:assert/strict';
import { createHash, randomUUID } from 'node:crypto';
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, chmodSync, rmSync, existsSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { spawn, spawnSync } from 'node:child_process';
import { createServer } from 'node:http';
import { setTimeout as delay } from 'node:timers/promises';
import { writeHeartbeat } from './remote-mailbox-heartbeat.mjs';
import { assertHeartbeatWriterBoundary } from './remote-mailbox-heartbeat.test.mjs';

const index = process.argv.indexOf('--agent-session-bin');
assert(index >= 0 && process.argv[index + 1], 'requires --agent-session-bin exact binary');
const binary = process.argv[index + 1];
const root = mkdtempSync(join(tmpdir(), 'nils-remote-mailbox-'));
const hash = value => createHash('sha256').update(value).digest('hex');
const now = () => Math.floor(Date.now()/1000);
const privateWrite = (path, value) => { writeFileSync(path, typeof value === 'string' ? value : JSON.stringify(value), {mode:0o600}); chmodSync(path,0o600); };
const hosts = ['alpha','beta','gamma'].map(machine => ({ machine, root:join(root,machine), session:`${machine}-agent`, incarnation:randomUUID(), capability:randomUUID()+randomUUID(), operator:randomUUID()+randomUUID(), relay:randomUUID()+randomUUID(), ingress:randomUUID()+randomUUID(), child:null, url:null }));
const children = new Set();
const checks = [];
let denyRelay = null, hold = false, dropOnce = false, blockRelay = false, relayBlocked = false, edge;
function seed(host) {
  mkdirSync(join(host.root,'sessions',host.session,'coordination'), {recursive:true,mode:0o700});
  mkdirSync(join(host.root,'coordination'),{recursive:true,mode:0o700});
  const sessionRoot=join(host.root,'sessions',host.session);
  privateWrite(join(sessionRoot,'session.json'),{schema_version:'agent-session.session.v1',id:host.session,agent:'codex',mode:'interactive',title:'remote fixture',title_revision:0,cwd:root,tmux_session:`fixture-${host.machine}`,prompt_file:null,log_file:null,created_at:'2030-01-01T00:00:00Z',updated_at:'2030-01-01T00:00:00Z',coordination_mode:'advisory',runtime:{kind:'tmux',tmux_session:`fixture-${host.machine}`,generation:1,started_at:'2030-01-01T00:00:00Z',launch_id:host.incarnation}});
  host.capabilityFile=join(sessionRoot,'coordination',`capability-${hash(host.incarnation)}`);
  privateWrite(host.capabilityFile,host.capability);
  writeHeartbeat(join(sessionRoot,'coordination','heartbeat'),`${host.incarnation}:${now()}\n`);
  privateWrite(join(host.root,'coordination','registry.json'),{schema_version:'agent-session.coordination-registry.v2',fingerprint_epoch:1,fingerprint_key:randomUUID()+randomUUID(),brokers:{[host.session]:{session_id:host.session,incarnation:host.incarnation,coordination_mode:'advisory',capability_digest:hash(host.capability),generation:1,state:'ready',heartbeat_at:'2030-01-01T00:00:00Z',heartbeat_epoch:now()}},claims:[],operations:[],messages:[],receipts:{},notifications:{}});
}
const address = h => ({machine:h.machine,session_id:h.session,session_incarnation:h.incarnation});
async function waitFor(fn, label, milliseconds=20000) {
  const deadline=Date.now()+milliseconds;
  while(Date.now()<deadline){const value=await fn();if(value)return value;await delay(100);}
  throw new Error(`timed out: ${label}`);
}
async function start(host,federation=true) {
  const env={...process.env};
  for(const key of ['AGENT_SESSION_TOKEN','AGENT_SESSION_RELAY_URL','AGENT_SESSION_RELAY_TOKEN','AGENT_SESSION_RELAY_INGRESS_TOKEN','AGENT_CONSOLE_COORDINATION_RELAYS'])delete env[key];
  if(federation)Object.assign(env,{AGENT_SESSION_RELAY_URL:edge.url,AGENT_SESSION_RELAY_TOKEN:host.relay,AGENT_SESSION_RELAY_INGRESS_TOKEN:host.ingress});
  const endpoint=join(host.root,'coordination','daemon-endpoint.json');
  rmSync(endpoint,{force:true});
  host.child=spawn(binary,['--state-dir',host.root,'serve','--bind','127.0.0.1:0','--machine',host.machine,'--token-stdin','--tmux-bin','/bin/false'],{env,stdio:['pipe','ignore','pipe']});
  children.add(host.child);host.child.stdin.end(host.operator);
  let stderr='';host.child.stderr.on('data',data=>{if(stderr.length<4096)stderr+=data.toString();});
  host.url=await waitFor(()=>{if(host.child.exitCode!==null)throw new Error(`daemon failed: ${stderr}`);if(existsSync(endpoint))return JSON.parse(readFileSync(endpoint,'utf8')).url;},'daemon endpoint');
  await waitFor(async()=>{try{return(await fetch(host.url+'/healthz')).ok;}catch{return false;}},'daemon health');
}
async function stop(host){if(!host.child)return;const child=host.child;child.kill('SIGTERM');await Promise.race([new Promise(resolve=>child.once('exit',resolve)),delay(3000)]);if(child.exitCode===null)child.kill('SIGKILL');children.delete(child);host.child=null;}
async function cli(host,args,expect=0){const output=await new Promise((resolve,reject)=>{const child=spawn(binary,['--state-dir',host.root,'message',...args,'--capability-file',host.capabilityFile,'--format','json'],{stdio:['ignore','pipe','pipe']});let stdout='',stderr='';child.stdout.on('data',data=>stdout+=data);child.stderr.on('data',data=>stderr+=data);child.on('error',reject);child.on('exit',status=>resolve({status,stdout,stderr}));});assert.equal(output.status,expect,`CLI ${args[0]} failed: ${output.stderr} ${output.stdout}`);const parsed=JSON.parse(output.stdout);return parsed.data??parsed;}
function bodyFile(label,text){const path=join(root,label+'.txt');privateWrite(path,text);return path;}
async function status(host,id){return cli(host,['delivery','--session',host.session,'--message',id]);}
async function delivered(host,id){return waitFor(async()=>{const value=await status(host,id);return value.state==='delivered'?value:false;},'persisted delivery');}
async function send(source,target,label){return cli(source,['send','--from',source.session,'--to-machine',target.machine,'--to',target.session,'--body-file',bodyFile(label,label),'--idempotency-key',label]);}
async function inspect(target,id,text){const value=await cli(target,['show','--session',target.session,'--message',id]);assert.equal(value.body.text,text);assert.equal(value.body.classification,'untrusted_peer_data');return value;}
try {
  assertHeartbeatWriterBoundary();
  hosts.forEach(seed);
  for(const collision of ['relay','ingress']){
    const h=hosts[0];const env={...process.env,AGENT_SESSION_RELAY_URL:'https://relay.example',AGENT_SESSION_RELAY_TOKEN:h.relay,AGENT_SESSION_RELAY_INGRESS_TOKEN:h.ingress};
    const result=spawnSync(binary,['--state-dir',h.root,'serve','--bind','127.0.0.1:0','--machine',h.machine,'--token-stdin','--tmux-bin','/bin/false'],{env,input:h[collision],encoding:'utf8',timeout:10000});
    assert.equal(result.status,64,'operator and federation service credentials must be distinct');
  }
  checks.push('operator-service-credential-collision-rejected');
  const server=createServer(async(req,res)=>{
    const source=hosts.find(h=>req.headers.authorization===`Bearer ${h.relay}`);
    if(!source){res.writeHead(401);res.end(JSON.stringify({error:{code:'unauthorized'}}));return;}
    const url=new URL(req.url,'http://localhost');
    if(url.pathname==='/api/coordination/peers/v1'){
      if(url.searchParams.get('source_session_id')!==source.session||url.searchParams.get('source_incarnation')!==source.incarnation){res.writeHead(403);res.end(JSON.stringify({error:{code:'origin-forbidden'}}));return;}
      res.setHeader('Content-Type','application/json');res.end(JSON.stringify({schema_version:'agent-session.remote-peers.v1',peers:hosts.map(h=>({...address(h),messaging_supported:true}))}));return;
    }
    if(url.pathname!=='/api/coordination/relay/v1'){res.writeHead(404);res.end();return;}
    const chunks=[];for await(const chunk of req)chunks.push(chunk);
    const message=JSON.parse(Buffer.concat(chunks));
    assert.deepEqual(message.from,address(source));
    if(message.body==='timeout-first'){await delay(16000);res.destroy();return;}
    if(denyRelay){res.writeHead(denyRelay==='rate-limited'?429:403);res.end(JSON.stringify({error:{code:denyRelay}}));return;}
    if(hold){res.writeHead(503);res.end(JSON.stringify({error:{code:'remote-messaging-unavailable'}}));return;}
    if(blockRelay){relayBlocked=true;await delay(2500);relayBlocked=false;}
    if(message.from.machine==='alpha'&&message.body==='category forwarding body')await delay(2200);
    const target=hosts.find(h=>h.machine===message.to.machine);
    if(!target||JSON.stringify(message.to)!==JSON.stringify(address(target))){res.writeHead(409);res.end(JSON.stringify({error:{code:'session-incarnation-conflict'}}));return;}
    const response=await fetch(target.url+'/coordination/messages/receive/v1',{method:'POST',headers:{Authorization:`Bearer ${target.operator}`,'X-Agent-Session-Relay-Token':target.ingress,'Content-Type':'application/json'},body:JSON.stringify(message)});
    const bytes=await response.text();
    if(dropOnce&&response.ok){dropOnce=false;res.destroy();return;}
    res.writeHead(response.status,{'Content-Type':'application/json'});res.end(bytes);
  });
  await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));edge={server,url:`http://127.0.0.1:${server.address().port}`};
  await Promise.all(hosts.map(h=>start(h)));
  const heartbeat=setInterval(()=>hosts.forEach(h=>writeHeartbeat(join(h.root,'sessions',h.session,'coordination','heartbeat'),`${h.incarnation}:${now()}\n`)),1000);heartbeat.unref();
  const [alpha,beta,gamma]=hosts;
  assert.equal(JSON.parse(readFileSync(join(alpha.root,'coordination','registry.json'),'utf8')).schema_version,'agent-session.coordination-registry.v2');
  assert.equal((await cli(alpha,['peers','--session',alpha.session])).peers.length,3);checks.push('v2-read-before-federation');
  const wrongRecipient=await cli(alpha,['send','--from',alpha.session,'--to-machine',beta.machine,'--to',beta.session+'-wrong','--body-file',bodyFile('wrong-recipient','no-op'),'--idempotency-key','wrong-recipient-key'],65);
  assert.equal(wrongRecipient.error.code,'remote-messaging-unavailable');
  assert.match(wrongRecipient.error.message,/exact --to session ID and --to-machine/);
  assert.match(wrongRecipient.error.message,/message peers/);
  assert(!existsSync(join(alpha.root,'coordination','federation-journal.json')),'refused discovery does not persist an envelope');
  checks.push('absent-recipient-guidance-through-cli');
  const timed=await send(alpha,beta,'timeout-first');
  const following=await send(alpha,beta,'timeout-following');
  await waitFor(async()=>{const v=await status(alpha,following.message_id);return v.state==='delivered';},'later queued entry bypasses repeated timeout',35000);
  const timedState=await status(alpha,timed.message_id);assert(timedState.attempts>0);
  const timedJournal=JSON.parse(readFileSync(join(alpha.root,'coordination','federation-journal.json'),'utf8')).remote_outbox.find(i=>i.envelope.message_id===timed.message_id);
  assert(timedJournal.next_attempt_epoch>now(),'retry deadline begins after request completion');
  await stop(alpha);
  const journalPath=join(alpha.root,'coordination','federation-journal.json');const journal=JSON.parse(readFileSync(journalPath,'utf8'));journal.remote_outbox.find(i=>i.envelope.message_id===timed.message_id).state='delivery-unknown';privateWrite(journalPath,journal);
  const betaRegistryPath=join(beta.root,'coordination','registry.json');const betaRegistry=JSON.parse(readFileSync(betaRegistryPath,'utf8'));betaRegistry.messages=betaRegistry.messages.filter(m=>m.message_id!==following.message_id);privateWrite(betaRegistryPath,betaRegistry);
  await start(alpha);checks.push('fair-two-entry-timeout-post-request-deadline');
  dropOnce=true;
  const initial=await send(alpha,beta,'response-loss');assert.equal(initial.state,'queued');
  await waitFor(()=>JSON.parse(readFileSync(join(beta.root,'coordination','registry.json'),'utf8')).messages.length===1,'destination persisted before lost response');
  await stop(alpha);await start(alpha);await delivered(alpha,initial.message_id);
  const replay=await send(alpha,beta,'response-loss');assert.equal(replay.message_id,initial.message_id);
  const registry=JSON.parse(readFileSync(join(beta.root,'coordination','registry.json'),'utf8'));
  assert.equal(registry.messages.length,1);assert.equal(registry.schema_version,'agent-session.coordination-registry.v2','federation must preserve old registry compatibility');
  checks.push('response-loss-source-restart-dedup');
const previousIndex=process.argv.indexOf('--previous-agent-session-bin');
if(previousIndex>=0){
  const oldBinary=process.argv[previousIndex+1];
  const runOld=args=>spawnSync(oldBinary,['--state-dir',beta.root,...args,'--capability-file',beta.capabilityFile,'--format','json'],{encoding:'utf8',timeout:10000});
  const path=join(beta.root,'coordination','registry.json');
  const before=JSON.parse(readFileSync(path,'utf8'));const sender=before.messages[0].sender_session_id;
  assert(sender.startsWith('remote:'),'authoritative opaque sender is incompatible with local ID grammar');
  const shown=runOld(['message','show','--session',beta.session,'--message',initial.message_id]);
  assert.equal(shown.status,0,'old facade can show and persist remote inbox');
  const after=JSON.parse(readFileSync(path,'utf8'));
  assert.equal(after.messages[0].sender_session_id,sender,'oldwriter preserves authoritative sender');
  assert.equal(after.messages[0].sender_incarnation,alpha.incarnation);
  assert.deepEqual(after.receipts,before.receipts,'oldwriter preserves remote dedup receipt and expiry');
  const broker=runOld(['broker','status','--session',beta.session,'--authenticated']);
  assert.equal(broker.status,0,'old broker operations remain available');
  const guard=runOld(['work-context','check','--session',beta.session,'--allow-incomplete']);
  assert.equal(guard.status,65,'empty fixture has no active claim');
  assert.equal(JSON.parse(guard.stdout).error?.code,'claim-not-active','old hook guard reaches normal semantic result');
  const replied=runOld(['message','reply','--session',beta.session,'--message',initial.message_id,'--if-revision',String(after.messages[0].revision),'--body-file',bodyFile('old-reply','old reply'),'--idempotency-key','old-reply-key']);
  assert.notEqual(replied.status,0,'old facade cannot launder remote origin into local sender');
  assert.equal(JSON.parse(readFileSync(path,'utf8')).messages.length,1);
  const journalPath=join(alpha.root,'coordination','federation-journal.json');const journalBefore=readFileSync(journalPath);
  const oldLocal=spawnSync(oldBinary,['--state-dir',alpha.root,'message','send','--from',alpha.session,'--to',alpha.session,'--body-file',bodyFile('old-local','old local'),'--idempotency-key','old-local-key','--capability-file',alpha.capabilityFile,'--format','json'],{encoding:'utf8',timeout:10000});
  assert.equal(oldLocal.status,0,'old local writer works alongside pending journal');
  assert.deepEqual(readFileSync(journalPath),journalBefore,'old local writer never rewrites source federation journal');
  checks.push('oldwriter-roundtrip-origin-receipt-broker-compatible');
}
await stop(beta);await start(beta);assert.equal(JSON.parse(readFileSync(join(beta.root,'coordination','registry.json'),'utf8')).messages.length,1);checks.push('destination-restart-retains-inbox');
const spoofed=await fetch(alpha.url+`/sessions/${alpha.session}/messages/remote/v1`,{method:'POST',headers:{Authorization:`Bearer ${beta.capability}`,'Content-Type':'application/json'},body:JSON.stringify({to_machine:beta.machine,to_session:beta.session,body:'spoof fixture',idempotency_key:'spoof-key-0001',reply_to:null,expires_in:null,reply_revision:null})});assert.equal(spoofed.status,401);checks.push('local-sender-capability-required');
blockRelay=true;const inFlight=await send(alpha,beta,'network-lock');await waitFor(()=>relayBlocked,'relay held in HTTP fixture');
const started=Date.now();await cli(alpha,['inbox','--session',alpha.session]);assert(Date.now()-started<2000,'network I/O must not hold registry lock');blockRelay=false;await delivered(alpha,inFlight.message_id);checks.push('network-await-holds-no-registry-lock');
const original=await inspect
(beta,initial.message_id,'response-loss');assert.equal(original.sender.machine,'alpha');assert.equal(original.sender.session_incarnation,alpha.incarnation);
  const replied=await cli(beta,['reply','--session',beta.session,'--message',initial.message_id,'--if-revision',String(original.revision),'--body-file',bodyFile('reply','reply'),'--idempotency-key','reply-0001']);
  await delivered(beta,replied.message_id);const shownReply=await inspect(alpha,replied.message_id,'reply');
  assert.equal(shownReply.reply_to,initial.message_id);assert.equal(JSON.parse(readFileSync(join(alpha.root,'coordination','registry.json'),'utf8')).messages.find(m=>m.message_id===replied.message_id).reply_depth,1);
  const replyReplay=await cli(beta,['reply','--session',beta.session,'--message',initial.message_id,'--if-revision',String(original.revision),'--body-file',bodyFile('reply','reply'),'--idempotency-key','reply-0001']);assert.equal(replyReplay.message_id,replied.message_id);
checks.push('bidirectional-reply-show-status');
  // Source -> coordinator -> third recipient: no category loss or source consumption.
  const categoryBody=bodyFile('category-progress','category forwarding body');
  const categorySend=['send','--from',alpha.session,'--to-machine',beta.machine,'--to',beta.session,'--body-file',categoryBody,'--category','progress','--idempotency-key','category-progress-0001'];
  const categorySource=await cli(alpha,categorySend);
  const categorySourceEpoch=JSON.parse(readFileSync(join(alpha.root,'coordination','federation-journal.json'),'utf8')).remote_outbox.find(i=>i.envelope.message_id===categorySource.message_id).envelope.created_at_epoch;
  await delivered(alpha,categorySource.message_id);
  const categoryIngress=JSON.parse(readFileSync(join(beta.root,'coordination','registry.json'),'utf8')).messages.find(m=>m.message_id===categorySource.message_id);
  assert(categoryIngress.created_at_epoch>=categorySourceEpoch+2,'fixture distinguishes source creation from local inbox ingress');
  const filtered=await cli(beta,['inbox','--session',beta.session,'--state','unread','--category','progress']);
  assert.equal(filtered.messages.length,1);assert.equal(filtered.messages[0].message_id,categorySource.message_id);
  assert.equal(filtered.messages[0].revision,1);assert(!JSON.stringify(filtered).includes('category forwarding body'));
  await stop(beta);await start(beta); // Source creation survives destination restart.
  const forwardArgs=['forward','--session',beta.session,'--message',categorySource.message_id,'--if-revision','1','--to-machine',gamma.machine,'--to',gamma.session,'--category','progress','--idempotency-key','category-forward-0001'];
  dropOnce=true;
  const categoryForward=await cli(beta,forwardArgs);assert.equal(categoryForward.state,'queued');
  assert.equal(categoryForward.category,'progress');assert.equal(categoryForward.forwarding.original_sender.machine,'alpha');
  const beforeAck=await cli(beta,['inbox','--session',beta.session,'--category','progress']);
  assert.equal(beforeAck.messages[0].state,'unread');assert.equal(beforeAck.messages[0].revision,1);
  await delivered(beta,categoryForward.message_id);
  const forwarded=await cli(gamma,['show','--session',gamma.session,'--message',categoryForward.message_id]);
  assert.equal(forwarded.body.text,'category forwarding body');assert.equal(forwarded.category,'progress');
  assert.equal(forwarded.sender.machine,'beta');assert.equal(forwarded.forwarding.attestation,'forwarder');
  assert.equal(forwarded.forwarding.original_message_id,categorySource.message_id);
  assert.equal(forwarded.forwarding.original_created_at_epoch,categorySourceEpoch,'forwarding preserves authenticated source creation, not delayed inbox ingress');
  assert.equal(forwarded.forwarding.original_sender.session_id,alpha.session);
  assert.deepEqual(forwarded.forwarding.hops[0].forwarder,address(beta));
  assert.deepEqual(forwarded.forwarding.hops[0].recipient,address(gamma));
  const loop=await cli(gamma,['forward','--session',gamma.session,'--message',categoryForward.message_id,'--if-revision',String(forwarded.revision),'--to-machine',alpha.machine,'--to',alpha.session,'--idempotency-key','category-loop-0001'],65);
  assert.equal(loop.error.code,'message-forward-loop');
  const categoryReply=await cli(gamma,['reply','--session',gamma.session,'--message',categoryForward.message_id,'--if-revision',String(forwarded.revision),'--body-file',bodyFile('category-reply','forward reply'),'--category','report','--idempotency-key','category-reply-0001']);
  await delivered(gamma,categoryReply.message_id);
  const replyCategory=await cli(beta,['show','--session',beta.session,'--message',categoryReply.message_id]);
  assert.equal(replyCategory.category,'report');assert.equal(replyCategory.sender.machine,'gamma');
  await cli(gamma,['ack','--session',gamma.session,'--message',categoryForward.message_id,'--if-revision',String(forwarded.revision),'--idempotency-key','category-copy-ack']);
  await cli(beta,['ack','--session',beta.session,'--message',categorySource.message_id,'--if-revision','1','--idempotency-key','category-source-ack']);
  // Replay and source-side audit survive compaction, source acknowledgement and restart.
  await stop(beta);await start(beta);
  const categoryReplay=await cli(beta,forwardArgs);assert.equal(categoryReplay.message_id,categoryForward.message_id);
  assert.deepEqual(categoryReplay.forwarding,categoryForward.forwarding);
  assert.equal(JSON.parse(readFileSync(join(gamma.root,'coordination','registry.json'),'utf8')).messages.filter(m=>m.message_id===categoryForward.message_id).length,1);
  checks.push('categories-authenticated-forward-independent-ack-loop-retry-audit');

  const alphaPath=join(alpha.root,'sessions',alpha.session,'session.json');const alphaRecord=JSON.parse(readFileSync(alphaPath,'utf8'));const originalAlphaIncarnation=alpha.incarnation;
  alpha.incarnation=randomUUID();alphaRecord.runtime.launch_id=alpha.incarnation;privateWrite(alphaPath,alphaRecord);
  const fencedReply=await cli(beta,['reply','--session',beta.session,'--message',initial.message_id,'--if-revision',String(original.revision),'--body-file',bodyFile('fenced-reply','fenced reply'),'--idempotency-key','fenced-reply-0001']);
  assert.equal(fencedReply.recipient.session_incarnation,originalAlphaIncarnation);
  const fencedReplyResult=await waitFor(async()=>{const v=await status(beta,fencedReply.message_id);return v.state==='rejected'?v:false;},'reply exact old source incarnation refused');assert.equal(fencedReplyResult.reason,'session-incarnation-conflict');
  assert(!JSON.parse(readFileSync(join(alpha.root,'coordination','registry.json'),'utf8')).messages.some(m=>m.message_id===fencedReply.message_id));
  alpha.incarnation=originalAlphaIncarnation;alphaRecord.runtime.launch_id=alpha.incarnation;privateWrite(alphaPath,alphaRecord);writeHeartbeat(join(alpha.root,'sessions',alpha.session,'coordination','heartbeat'),alpha.incarnation+':'+now()+'\n');checks.push('reply-origin-replacement-no-retarget');
  const bad=await fetch(beta.url+'/coordination/messages/receive/v1',{method:'POST',headers:{Authorization:`Bearer ${beta.operator}`,'Content-Type':'application/json'},body:'{}'});assert.equal(bad.status,401);checks.push('dedicated-ingress-required');
  for(const reason of ['origin-forbidden','coordination-unauthorized']){
    denyRelay=reason;const refused=await send(alpha,beta,'reject-'+reason);
    const result=await waitFor(async()=>{const value=await status(alpha,refused.message_id);return value.state==='rejected'?value:false;},'terminal source rejection');assert.equal(result.reason,reason);
  }
  denyRelay=null;checks.push('authorization-rejection-terminal-status');
  denyRelay='rate-limited';const limited=await send(alpha,beta,'rate-limit-retry');
  const limitedPending=await waitFor(async()=>{const v=await status(alpha,limited.message_id);return v.attempts>0?v:false;},'rate-limit remains queued');assert.equal(limitedPending.state,'queued');assert.equal(limitedPending.reason,'rate-limited');
  denyRelay=null;await delivered(alpha,limited.message_id);assert.equal(JSON.parse(readFileSync(join(beta.root,'coordination','registry.json'),'utf8')).messages.filter(m=>m.message_id===limited.message_id).length,1);checks.push('rate-limit-retry-once-delivery');
  dropOnce=true;const ambiguous=await send(alpha,beta,'lost-before-replacement');
  await waitFor(()=>JSON.parse(readFileSync(join(beta.root,'coordination','registry.json'),'utf8')).messages.some(m=>m.message_id===ambiguous.message_id),'commit before response loss');
  await waitFor(async()=>(await status(alpha,ambiguous.message_id)).attempts>0,'unconfirmed attempt recorded');
  const betaPath=join(beta.root,'sessions',beta.session,'session.json');const betaRecord=JSON.parse(readFileSync(betaPath,'utf8'));const originalBetaIncarnation=beta.incarnation;
  beta.incarnation=randomUUID();betaRecord.runtime.launch_id=beta.incarnation;privateWrite(betaPath,betaRecord);
  const ambiguousResult=await waitFor(async()=>{const v=await status(alpha,ambiguous.message_id);return v.state==='delivery-unknown'?v:false;},'response loss plus replacement remains unknown');assert.equal(ambiguousResult.reason,'session-incarnation-conflict');
  assert.equal(JSON.parse(readFileSync(join(beta.root,'coordination','registry.json'),'utf8')).messages.filter(m=>m.message_id===ambiguous.message_id).length,1);
  beta.incarnation=originalBetaIncarnation;betaRecord.runtime.launch_id=beta.incarnation;privateWrite(betaPath,betaRecord);writeHeartbeat(join(beta.root,'sessions',beta.session,'coordination','heartbeat'),beta.incarnation+':'+now()+'\n');checks.push('lost-response-replacement-preserves-delivery-ambiguity');
  hold=true;const fenced=await send(alpha,beta,'replacement');
  await waitFor(async()=>(await status(alpha,fenced.message_id)).attempts>0,'pending failed attempt');
  const recordPath=join(beta.root,'sessions',beta.session,'session.json');const record=JSON.parse(readFileSync(recordPath,'utf8'));beta.incarnation=randomUUID();record.runtime.launch_id=beta.incarnation;privateWrite(recordPath,record);
  hold=false;
  const rejected=await waitFor(async()=>{const value=await status(alpha,fenced.message_id);return value.state==='delivery-unknown'?value:false;},'replacement cannot prove nondelivery');assert.equal(rejected.reason,'session-incarnation-conflict');
  assert(!JSON.parse(readFileSync(join(beta.root,'coordination','registry.json'),'utf8')).messages.some(m=>m.message_id===fenced.message_id));checks.push('exact-incarnation-no-retarget');
  // Owner metadata audit observes both lifecycle cases without reading bodies.
  beta.incarnation=originalBetaIncarnation;record.runtime.launch_id=beta.incarnation;privateWrite(recordPath,record);
  writeHeartbeat(join(beta.root,'sessions',beta.session,'coordination','heartbeat'),beta.incarnation+':'+now()+'\n');
  const orphan=await send(alpha,beta,'audit-accepted-then-stopped');await delivered(alpha,orphan.message_id);
  const stoppedRegistry=JSON.parse(readFileSync(betaRegistryPath,'utf8'));stoppedRegistry.brokers[beta.session].state='stopped';privateWrite(betaRegistryPath,stoppedRegistry);
  const destinationBefore=readFileSync(betaRegistryPath);
  const destinationAuditResponse=await fetch(beta.url+'/coordination/messages/audit/v1',{headers:{Authorization:`Bearer ${beta.operator}`}});
  assert.equal(destinationAuditResponse.status,200);
  const destinationAudit=(await destinationAuditResponse.json()).data;
  const orphanRow=destinationAudit.records.find(r=>r.message_id===orphan.message_id);
  assert(orphanRow.anomalies.includes('recipient-stopped'));assert.equal(orphanRow.mailbox_state,'unread');
  assert.deepEqual(readFileSync(betaRegistryPath),destinationBefore,'audit does not consume accepted orphan');
  assert(!JSON.stringify(destinationAudit).includes('audit-accepted-then-stopped'),'body never projected');
  const stoppedSubmission=await send(alpha,beta,'audit-stopped-target-submission');
  const pendingStopped=await waitFor(async()=>{const v=await status(alpha,stoppedSubmission.message_id);return v.attempts>0&&v.reason==='remote-messaging-unavailable'?v:false;},'stopped target refusal stays source pending');
  assert.equal(pendingStopped.state,'queued');
  assert(!JSON.parse(readFileSync(betaRegistryPath,'utf8')).messages.some(m=>m.message_id===stoppedSubmission.message_id),'no fabricated destination unread');
  const sourceAuditResponse=await fetch(alpha.url+'/coordination/messages/audit/v1?include_healthy=true',{headers:{Authorization:`Bearer ${alpha.operator}`}});
  const sourceAudit=(await sourceAuditResponse.json()).data;
  const pendingRow=sourceAudit.records.find(r=>r.message_id===stoppedSubmission.message_id);
  assert.equal(pendingRow.delivery_state,'queued');assert.equal(pendingRow.reason_code,'remote-messaging-unavailable');
  assert(pendingRow.sent_at);assert(pendingRow.last_attempt_at);assert(pendingRow.state_changed_at);
  const acceptedRow=sourceAudit.records.find(r=>r.message_id===orphan.message_id);
  assert.equal(acceptedRow.delivery_state,'delivered');assert(acceptedRow.sent_at);assert(acceptedRow.persisted_at);assert(acceptedRow.last_attempt_at);
  checks.push('audit-stopped-submission-and-accepted-orphan');
  await stop(alpha);await start(alpha,false);
  const localPeer={...alpha,session:alpha.session+'-local-peer',incarnation:randomUUID(),capability:randomUUID()+randomUUID()};
  const peerRoot=join(alpha.root,'sessions',localPeer.session);mkdirSync(join(peerRoot,'coordination'),{recursive:true,mode:0o700});
  const peerRecord=JSON.parse(readFileSync(join(alpha.root,'sessions',alpha.session,'session.json'),'utf8'));
  peerRecord.id=localPeer.session;peerRecord.runtime.launch_id=localPeer.incarnation;privateWrite(join(peerRoot,'session.json'),peerRecord);
  localPeer.capabilityFile=join(peerRoot,'coordination','capability-'+hash(localPeer.incarnation));privateWrite(localPeer.capabilityFile,localPeer.capability);
  writeHeartbeat(join(peerRoot,'coordination','heartbeat'),localPeer.incarnation+':'+now()+'\n');
  const localRegistryPath=join(alpha.root,'coordination','registry.json');const localRegistry=JSON.parse(readFileSync(localRegistryPath,'utf8'));
  localRegistry.brokers[localPeer.session]={...localRegistry.brokers[alpha.session],session_id:localPeer.session,incarnation:localPeer.incarnation,capability_digest:hash(localPeer.capability)};privateWrite(localRegistryPath,localRegistry);
  const local=await cli(alpha,['send','--from',alpha.session,'--to',localPeer.session,'--body-file',bodyFile('local','local'),'--idempotency-key','local-0001']);
  const localOriginal=await inspect(localPeer,local.message_id,'local');
  privateWrite(join(alpha.root,'coordination','federation-journal.json'),{schema_version:'agent-session.federation-journal.v99',remote_outbox:[]});
  const localReply=await cli(localPeer,['reply','--session',localPeer.session,'--message',local.message_id,'--if-revision',String(localOriginal.revision),'--body-file',bodyFile('local-reply','local reply'),'--idempotency-key','local-reply-key']);
  await inspect(alpha,localReply.message_id,'local reply');checks.push('local-mailbox-federation-disabled-journal-isolated');
  clearInterval(heartbeat);
  console.log(JSON.stringify({schema_version:'agent-session.remote-acceptance.v1',status:'passed',checks}));
} finally {
  await Promise.all(hosts.map(stop));for(const child of children)child.kill('SIGKILL');if(edge)await new Promise(resolve=>edge.server.close(resolve));rmSync(root,{recursive:true,force:true});
}
