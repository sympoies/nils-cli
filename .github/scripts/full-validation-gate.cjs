'use strict';

const REQUIRED = Object.freeze(['test', 'test_macos', 'coverage', 'test_containment', 'cargo_deny']);
const { capacityReason } = require('./macos-runner-capacity.cjs');

function requireFullValidation({ expected, actual, needs, isolationGuard = false, macosJobs = [] }) {
  if (!/^[0-9a-f]{40}$/.test(expected || '') || actual !== expected) {
    throw new Error('Full validation commit does not match the requested commit');
  }
  const required = isolationGuard ? [...REQUIRED, 'isolation_guard'] : REQUIRED;
  const failed = required.filter(name => needs?.[name]?.result !== 'success');
  if (failed.length) {
    const capacity = capacityReason(macosJobs);
    throw new Error(`Full validation did not succeed: ${failed.join(', ')}${capacity ? `; ${capacity}` : ''}`);
  }
  return true;
}

module.exports = { requireFullValidation };
