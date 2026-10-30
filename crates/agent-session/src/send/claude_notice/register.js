// External native contract: Claude Code 2.1.296. No terminal input mutations.
export function register(on) {
  let started = false;
  let inFlight = false;
  let lastOp;
  let timer;
  const schema = (name) => `agent-session.claude-notice-${name}.v1`;
  const keys = (value, names) => value && typeof value === 'object'
    && !Array.isArray(value) && Object.keys(value).sort().join(',') === [...names].sort().join(',');
  const launchEqual = (a, b) => keys(a, ['session_id', 'incarnation', 'generation', 'profile'])
    && keys(b, ['session_id', 'incarnation', 'generation', 'profile']) && a.session_id === b.session_id
    && a.incarnation === b.incarnation && a.generation === b.generation && a.profile === b.profile;
  const bindingEqual = (a, b) => keys(a, ['launch', 'provider', 'provider_session_id', 'provider_version', 'module_sha256'])
    && launchEqual(a.launch, b.launch) && a.provider === b.provider
    && a.provider_session_id === b.provider_session_id && a.provider_version === b.provider_version
    && a.module_sha256 === b.module_sha256;
  const utf8Length = (text) => {
    let bytes = 0;
    for (const character of text) {
      const cp = character.codePointAt(0);
      if (cp >= 0xd800 && cp <= 0xdfff) return -1;
      bytes += cp < 0x80 ? 1 : cp < 0x800 ? 2 : cp < 0x10000 ? 3 : 4;
    }
    return bytes;
  };
  on('session.start', async ($, e, next) => {
    const result = await next(e);
    if (started || !e.isInteractive || e.surface !== 'terminal') return result;
    started = true;
    const root = $.plugin.root;
    const unavailable = () => $.fs.write(`${root}/ready.json`, JSON.stringify({
      schema_version: schema('ready'), state: 'unavailable',
    }));
    try {
      const boot = JSON.parse(await $.fs.read(`${root}/bootstrap.json`));
      if (!keys(boot, ['schema_version', 'launch', 'module_sha256', 'socket_path', 'nonce'])
          || boot.schema_version !== schema('runtime')
          || !keys(boot.launch, ['session_id', 'incarnation', 'generation', 'profile'])
          || !/^[a-f0-9]{64}$/.test(boot.module_sha256) || !/^[a-f0-9]{64}$/.test(boot.nonce)
          || typeof boot.socket_path !== 'string' || !boot.socket_path.startsWith('/')) throw new Error();
      const observe = async () => ({
        session: await $.env.get('AGENT_SESSION_ID'),
        incarnation: await $.env.get('AGENT_SESSION_RUNTIME_ID'),
        provider: await $.session.id(), version: (await $.session.version()).version,
        terminal: (await $.session.surfaces()).includes('terminal'),
      });
      const first = await observe();
      const current = (value) => value.session === boot.launch.session_id
        && value.incarnation === boot.launch.incarnation && value.provider === first.provider
        && value.version === '2.1.296' && value.terminal;
      if (!current(first) || !first.provider) throw new Error();
      const binding = { launch: boot.launch, provider: 'claude', provider_session_id: first.provider,
        provider_version: '2.1.296', module_sha256: boot.module_sha256 };
      const empty = async () => {
        const draft = await $.prompt.read();
        return draft.text === '' && draft.cursor === 0;
      };
      const run = async () => {
        const offer = JSON.parse(await $.fs.read(`${root}/offer.json`));
        if (!keys(offer, ['schema_version', 'op', 'binding', 'socket_path', 'nonce'])
            || offer.schema_version !== schema('offer') || offer.op === lastOp
            || !/^[a-f0-9-]{36}$/.test(offer.op) || !bindingEqual(offer.binding, binding)
            || offer.socket_path !== boot.socket_path || offer.nonce !== boot.nonce) return;
        lastOp = offer.op; // No retry of a lingering offer, even after unknown I/O.
        if (!current(await observe()) || !(await empty())) return;
        const authorization = await $.http.fetch('http://notice.invalid/authorize', {
          method: 'POST', socketPath: boot.socket_path,
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ schema_version: schema('authorize'), op: offer.op, binding,
            nonce: boot.nonce, terminal: true, box_empty: true, cursor: 0 }),
        });
        if (!authorization.ok || authorization.status !== 200) return;
        const authorized = JSON.parse(authorization.text);
        if (!keys(authorized, ['schema_version', 'op', 'binding', 'envelope_bytes', 'envelope_sha256', 'text'])
            || authorized.schema_version !== schema('authorized') || authorized.op !== offer.op
            || !bindingEqual(authorized.binding, binding) || typeof authorized.text !== 'string'
            || authorized.text.includes('\r') || utf8Length(authorized.text) !== authorized.envelope_bytes
            || authorized.envelope_bytes > 16384 || !/^[a-f0-9]{64}$/.test(authorized.envelope_sha256)) return;
        const report = { schema_version: schema('result'), op: offer.op, binding,
          nonce: boot.nonce, outcome: 'unknown', after_empty: false };
        try {
          if (current(await observe()) && await empty()) {
            const returned = await $.prompt.submit({ text: authorized.text, asUser: true });
            report.after_empty = await empty();
            if (returned.drop !== undefined) report.outcome = 'dropped';
            else if (returned.text === authorized.text && report.after_empty && current(await observe())
                && returned.origin?.kind === 'plugin' && returned.origin.name === 'agent-session-claude-notice'
                && returned.origin.asUser === true) {
              report.outcome = 'accepted';
              report.returned_text = returned.text;
              report.origin = { kind: returned.origin.kind, name: returned.origin.name,
                as_user: returned.origin.asUser };
            }
          }
        } catch { /* A consumed authorization is never retried or cleared. */ }
        await $.http.fetch('http://notice.invalid/result', {
          method: 'POST', socketPath: boot.socket_path,
          headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(report),
        });
      };
      timer = $.clock.every(250, () => {
        if (inFlight) return;
        inFlight = true;
        void run().catch(() => {}).finally(() => { inFlight = false; });
      });
      await $.fs.write(`${root}/ready.json`, JSON.stringify({ schema_version: schema('ready'), binding,
        nonce: boot.nonce, terminal: true, timer_started: true }));
    } catch {
      if (timer) timer.cancel();
      await unavailable();
    }
    return result;
  });
  on('session.end', async ($, e, next) => {
    if (timer) timer.cancel();
    if (started) await $.fs.write(`${$.plugin.root}/ready.json`, JSON.stringify({
      schema_version: schema('ready'), state: 'unavailable',
    }));
    return next(e);
  });
}
