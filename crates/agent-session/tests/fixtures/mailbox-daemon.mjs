import assert from 'node:assert/strict';
import { lstatSync, mkdirSync, realpathSync } from 'node:fs';
import { basename, dirname, isAbsolute, join, sep } from 'node:path';

function assertInside(root, name, path) {
  assert(isAbsolute(path) && !path.split(sep).includes('..'), `invalid fixture path for ${name}`);
  // Serve creates some directories lazily. Resolve the nearest existing
  // ancestor, including symlinks, before allowing any writes.
  let parent = path;
  while (!lstatSync(parent, { throwIfNoEntry: false })) parent = dirname(parent);
  const resolved = realpathSync(parent);
  assert(resolved === root || resolved.startsWith(root + sep), `${name} escaped the mailbox fixture`);
}

export function mailboxDaemon(fixture, host, { relayUrl, servicesFile } = {}) {
  assert.equal(dirname(host.root), fixture, 'daemon state must be a direct child of the fixture root');
  const fixtureRoot = realpathSync(fixture);
  const state = join(fixtureRoot, basename(host.root));
  assert.equal(realpathSync(host.root), state, 'daemon state must resolve to its exact private root');

  const home = join(state, 'home');
  const config = join(state, 'config');
  const data = join(state, 'data');
  const agentHome = join(state, 'agent-home');
  const tmuxSocket = join(state, 'tmux.sock');
  const paths = {
    HOME: home,
    AGENT_HOME: agentHome,
    TMPDIR: join(state, 'tmp'),
    XDG_CONFIG_HOME: config,
    XDG_CONFIG_DIRS: config,
    XDG_DATA_HOME: data,
    XDG_DATA_DIRS: data,
    XDG_CACHE_HOME: join(state, 'cache'),
    XDG_STATE_HOME: join(state, 'xdg-state'),
    XDG_RUNTIME_DIR: state,
    AGENT_SESSION_STATE_DIR: state,
    CODEX_HOME: join(home, '.codex'),
    CLAUDE_CONFIG_DIR: join(home, '.claude'),
    AGENT_SESSION_CODEX_BIN: join(state, 'unused-provider'),
    AGENT_SESSION_CLAUDE_BIN: join(state, 'unused-provider'),
    TMUX_TMPDIR: state,
  };
  for (const [name, path] of Object.entries(paths)) assertInside(state, name, path);
  assertInside(state, 'session temp root', join(agentHome, 'tmp'));
  assertInside(state, 'tmux socket', tmuxSocket);
  mkdirSync(paths.TMPDIR, { recursive: true, mode: 0o700 });

  // The absolute inert executable is shared by both fixtures.
  const tmux = '/bin/false';
  const env = { ...paths, PATH: '/usr/bin:/bin', AGENT_SESSION_TMUX_BIN: tmux, TMUX: `${tmuxSocket},0,0` };
  if (relayUrl) {
    env.AGENT_SESSION_RELAY_URL = relayUrl;
    env.AGENT_SESSION_RELAY_TOKEN = host.relay;
    env.AGENT_SESSION_RELAY_INGRESS_TOKEN = host.ingress;
  }
  if (servicesFile) {
    assertInside(fixtureRoot, 'mailbox service admission', servicesFile);
    env.AGENT_SESSION_MAILBOX_SERVICES_FILE = servicesFile;
  }
  const args = ['--state-dir', state, 'serve', '--bind', '127.0.0.1:0', '--machine', host.machine, '--token-stdin', '--tmux-bin', tmux];
  return { args, env };
}
