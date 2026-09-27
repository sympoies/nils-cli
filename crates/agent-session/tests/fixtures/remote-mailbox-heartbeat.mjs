import fs from 'node:fs';
import { randomUUID } from 'node:crypto';
import { dirname, join } from 'node:path';

export function writeHeartbeat(path, value) {
  const temporary = join(dirname(path), `.heartbeat-${randomUUID()}.tmp`);
  const fd = fs.openSync(temporary, 'wx', 0o600);
  try {
    fs.writeFileSync(fd, value);
    fs.closeSync(fd);
    fs.renameSync(temporary, path);
  } catch (error) {
    try { fs.closeSync(fd); } catch { /* already closed before rename */ }
    throw error;
  } finally { fs.rmSync(temporary, {force:true}); }
}
