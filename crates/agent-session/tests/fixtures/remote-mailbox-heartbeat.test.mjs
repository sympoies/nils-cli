import assert from 'node:assert/strict';
import fs from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { writeHeartbeat } from './remote-mailbox-heartbeat.mjs';

export function assertHeartbeatWriterBoundary() {
  const root = fs.mkdtempSync(join(tmpdir(), 'nils-heartbeat-boundary-'));
  const path = join(root, 'heartbeat');
  const previous = 'original-incarnation:100\n';
  const next = 'replacement-incarnation:101\n';
  const originalWrite = fs.writeFileSync;
  const originalRename = fs.renameSync;
  let observedWrite = false, observedCommit = false;
  try {
    originalWrite(path, previous, {mode:0o600});
    // Deterministic reader barrier after opening the writer, before its bytes.
    fs.writeFileSync = (target, value, options) => {
      const ownsFd = typeof target !== 'number';
      const fd = ownsFd ? fs.openSync(target, options?.flag ?? 'w', options?.mode) : target;
      try {
        observedWrite = true;
        assert.equal(fs.readFileSync(path, 'utf8'), previous, 'live heartbeat was truncated before publication');
        originalWrite(fd, value, options);
      } finally { if (ownsFd) fs.closeSync(fd); }
    };
    fs.renameSync = (temporary, target) => {
      observedCommit = true;
      assert.equal(fs.readFileSync(path, 'utf8'), previous);
      assert.equal(fs.readFileSync(temporary, 'utf8'), next);
      assert.equal(fs.statSync(temporary).mode & 0o777, 0o600);
      assert.equal(join(root, temporary.split('/').at(-1)), temporary, 'temporary must share destination directory');
      originalRename(temporary, target);
    };
    writeHeartbeat(path, next);
    assert(observedWrite);
    assert(observedCommit, 'heartbeat must commit through rename');
    assert.equal(fs.readFileSync(path, 'utf8'), next);
    assert.equal(fs.statSync(path).mode & 0o777, 0o600);
    assert.deepEqual(fs.readdirSync(root), ['heartbeat']);
  } finally {
    fs.writeFileSync = originalWrite;
    fs.renameSync = originalRename;
    fs.rmSync(root, {recursive:true, force:true});
  }
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  assertHeartbeatWriterBoundary();
  console.log('passed: atomic heartbeat reader boundary and mode 0600');
}
