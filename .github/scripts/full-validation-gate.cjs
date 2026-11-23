'use strict';

const REQUIRED = Object.freeze(['test', 'test_macos', 'coverage', 'test_containment', 'cargo_deny', 'macos_check']);

function requireFullValidation({ expected, actual, needs, isolationGuard = false, macosPartitionsOnly = false }) {
  if (!/^[0-9a-f]{40}$/.test(expected || '') || actual !== expected) {
    throw new Error('Full validation commit does not match the requested commit');
  }
  const required = macosPartitionsOnly ? ['macos_check'] : [...REQUIRED];
  if (isolationGuard) required.push('isolation_guard');
  const failed = required.filter(name => needs?.[name]?.result !== 'success');
  if (failed.length) {
    throw new Error(`Full validation did not succeed: ${failed.join(', ')}`);
  }
  return true;
}

module.exports = { requireFullValidation };
